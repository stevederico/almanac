use std::io::{BufReader, BufWriter};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

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

fn accept_loop(listener: TcpListener, state: AppState) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let state = state.clone();
        thread::spawn(move || serve(stream, &state));
    }
}

fn serve(stream: TcpStream, state: &AppState) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let req = match read_request(&mut reader) {
        Ok(r) => r,
        Err(_) => return,
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
