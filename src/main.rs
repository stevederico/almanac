use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use almanac::db::Db;
use almanac::env::{load_env_file, read_config_from_os};
use almanac::server::{app, AppState};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
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
    let router = app(state);
    let binds = listen_hosts(&cfg.host);
    let mut set = tokio::task::JoinSet::new();
    for host in &binds {
        let addr = parse_bind(host, cfg.port).unwrap_or_else(|| {
            eprintln!("bad bind {host}:{}", cfg.port);
            std::process::exit(1);
        });
        let listener = TcpListener::bind(addr).await.unwrap_or_else(|e| {
            eprintln!("listen {addr}: {e}");
            std::process::exit(1);
        });
        let router = router.clone();
        set.spawn(async move { axum::serve(listener, router).await });
    }
    println!(
        "{}",
        serde_json::json!({
            "t": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "msg": "listen",
            "host": cfg.host,
            "binds": binds,
            "port": cfg.port,
            "scheme": "http",
        })
    );
    if let Some(Ok(Err(e))) = set.join_next().await {
        eprintln!("server: {e}");
        std::process::exit(1);
    }
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
