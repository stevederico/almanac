use std::io::{BufReader, BufWriter};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use almanac::db::Db;
use almanac::env::{load_env_file, read_config_from_os};
use almanac::http::{read_request, write_response};
use almanac::json::{stringify, Value};
use almanac::server::{handle, AppState};
use almanac::time::now_iso;

fn main() {
    let env_path = Path::new(".env");
    if let Err(e) = load_env_file(env_path) {
        eprintln!("{e}");
        std::process::exit(1);
    }
    // yagni: rustls for TLS_KEY/TLS_CERT; add if loopback HTTPS comes back
    let cfg = read_config_from_os();
    let db = Db::open(&cfg.db_path).unwrap_or_else(|e| {
        eprintln!("db: {e}");
        std::process::exit(1);
    });
    if !cfg.feed_token.is_empty() && !cfg.agent_key.is_empty() {
        if let Err(e) = db.ensure_home_calendar(&cfg.feed_token, &cfg.agent_key, &cfg.cal_name) {
            eprintln!("home calendar: {e}");
            std::process::exit(1);
        }
    }
    let public_base = std::env::var("PUBLIC_BASE").unwrap_or_default();
    let state = AppState {
        db: Arc::new(Mutex::new(db)),
        public_base,
    };
    let binds = listen_hosts(&cfg.host);
    let mut listeners = Vec::new();
    for host in &binds {
        let addr = parse_bind(host, cfg.port).unwrap_or_else(|| {
            eprintln!("bad bind {host}:{}", cfg.port);
            std::process::exit(1);
        });
        let listener = TcpListener::bind(addr).unwrap_or_else(|e| {
            eprintln!("listen {addr}: {e}");
            std::process::exit(1);
        });
        listeners.push(listener);
    }
    println!(
        "{}",
        stringify(&Value::object(&[
            ("t", Value::String(now_iso())),
            ("msg", Value::String("listen".into())),
            ("host", Value::String(cfg.host.clone())),
            (
                "binds",
                Value::Array(binds.iter().cloned().map(Value::String).collect()),
            ),
            ("port", Value::Number(i64::from(cfg.port))),
            ("scheme", Value::String("http".into())),
        ]))
    );
    let mut joins = Vec::new();
    for listener in listeners {
        let state = state.clone();
        joins.push(thread::spawn(move || accept_loop(listener, state)));
    }
    for join in joins {
        let _ = join.join();
    }
}

/// Give up on a client that has not finished its request headers.
///
/// Every response sends `connection: close` and one request is served per
/// connection, so no legitimate client holds a socket open waiting. Without a
/// deadline a client sending one byte a minute never trips the byte caps in
/// `read_request` and pins its thread forever.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Give up on a client that will not accept its response.
///
/// Generous compared to the read side: an ICS feed can be large and the client
/// on a slow mobile link.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// Refuse new connections past this many in flight.
///
/// One thread per connection, so this bounds thread and fd use under a flood.
/// Roomy for a personal calendar sitting behind a proxy.
const MAX_CONNECTIONS: usize = 512;

/// Decrements the live-connection count when a worker finishes.
///
/// A guard rather than a bare decrement so the slot is released on panic too --
/// `read_request` and `handle` both run inside the worker.
struct ConnectionGuard(Arc<AtomicUsize>);

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn accept_loop(listener: TcpListener, state: AppState) {
    let live = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };

        // Reserve the slot before spawning, so a burst cannot overshoot.
        let taken = live.fetch_add(1, Ordering::SeqCst) + 1;
        if taken > MAX_CONNECTIONS {
            live.fetch_sub(1, Ordering::SeqCst);
            eprintln!("almanac: refusing connection, {MAX_CONNECTIONS} already in flight");
            drop(stream);
            continue;
        }

        let guard = ConnectionGuard(live.clone());
        let state = state.clone();
        thread::spawn(move || {
            let _guard = guard;
            serve(stream, &state);
        });
    }
}

fn serve(stream: TcpStream, state: &AppState) {
    if let Err(e) = stream.set_read_timeout(Some(READ_TIMEOUT)) {
        eprintln!("almanac: set_read_timeout failed: {e}");
        return;
    }
    if let Err(e) = stream.set_write_timeout(Some(WRITE_TIMEOUT)) {
        eprintln!("almanac: set_write_timeout failed: {e}");
        return;
    }

    // Was `.expect("clone")`, which panicked the worker on fd exhaustion --
    // exactly the state a connection flood reaches.
    let Ok(read_half) = stream.try_clone() else {
        eprintln!("almanac: could not clone stream, dropping connection");
        return;
    };

    let mut reader = BufReader::new(read_half);
    let req = match read_request(&mut reader) {
        Ok(r) => r,
        Err(e) => {
            // Nothing logged this before: a stalled connection produced no
            // output at all, so a flood looked like a silent, idle server.
            eprintln!("almanac: dropping connection: {e}");
            return;
        }
    };
    let res = handle(state, &req);
    let mut writer = BufWriter::new(stream);
    let _ = write_response(&mut writer, &res);
}

/// Loopback hostnames must listen on v4 and v6. `localhost` prefers ::1.
fn listen_hosts(host: &str) -> Vec<String> {
    if host == "localhost" || host == "127.0.0.1" || host == "::1" {
        vec!["127.0.0.1".into(), "::1".into()]
    } else {
        vec![host.to_string()]
    }
}

fn parse_bind(host: &str, port: u16) -> Option<SocketAddr> {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}").parse().ok()
    } else {
        format!("{host}:{port}").parse().ok()
    }
}
