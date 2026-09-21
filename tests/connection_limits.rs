//! Socket-level tests for the accept loop.
//!
//! `serve` lives in the binary, not the library, so these spawn the real
//! binary and drive it over a TCP socket. Nothing else in the suite opens a
//! socket — every other test calls `handle()` in process.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// Long enough to cover the server's 10s read timeout plus scheduling slack,
/// short enough that a genuine hang still fails the test rather than the suite.
const HANG_BUDGET: Duration = Duration::from_secs(25);

/// Kills the server when the test ends, including on panic.
struct ServerGuard(Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Ask the OS for a free port, then release it for the server to claim.
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind probe");
    listener.local_addr().expect("probe addr").port()
}

fn start_server(db: &str) -> (ServerGuard, u16) {
    start_server_with(db, &[("AGENT_KEY", "test-agent-key")])
}

fn start_server_with(db: &str, env: &[(&str, &str)]) -> (ServerGuard, u16) {
    let port = free_port();
    let child = Command::new(env!("CARGO_BIN_EXE_almanac"))
        .env("PORT", port.to_string())
        .env("HOST", "127.0.0.1")
        .env("DB_PATH", db)
        .envs(env.iter().copied())
        .current_dir(std::env::temp_dir())
        .spawn()
        .expect("spawn almanac");

    let guard = ServerGuard(child);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return (guard, port);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("server did not start on port {port}");
}

#[test]
fn drops_a_connection_that_never_finishes_its_headers() {
    let (_server, port) = start_server("almanac-slowloris-test.db");

    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    // A request line with no terminating blank line: the header loop in
    // read_request would otherwise block here forever.
    stream.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n").expect("write partial");
    stream.flush().expect("flush");

    // Read past the server's own deadline. Without the timeout this blocks
    // until the test harness gives up.
    stream
        .set_read_timeout(Some(HANG_BUDGET))
        .expect("set client timeout");

    let started = Instant::now();
    let mut buf = Vec::new();
    let result = stream.read_to_end(&mut buf);
    let elapsed = started.elapsed();

    assert!(
        result.is_ok(),
        "server should close the stalled connection, not leave it hanging: {result:?}"
    );
    assert!(
        elapsed < HANG_BUDGET,
        "connection was still open after {elapsed:?}"
    );
}

#[test]
fn still_serves_a_well_formed_request() {
    let (_server, port) = start_server("almanac-normal-test.db");

    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set client timeout");
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: x\r\nUser-Agent: curl/8\r\n\r\n")
        .expect("write request");
    stream.flush().expect("flush");

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).expect("read response");
    let text = String::from_utf8_lossy(&buf);

    assert!(
        text.starts_with("HTTP/1.1 200"),
        "timeouts must not break a normal request, got: {}",
        text.lines().next().unwrap_or("<empty>")
    );
}

/// Send `raw`, read until the server closes, return everything it said.
fn exchange(port: u16, raw: &[u8]) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set client timeout");
    stream.write_all(raw).expect("write request");
    stream.flush().expect("flush");
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

#[test]
fn drops_a_client_that_drips_bytes_forever() {
    let (_server, port) = start_server("almanac-drip-test.db");

    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(HANG_BUDGET))
        .expect("set client timeout");
    let mut writer = stream.try_clone().expect("clone");
    // One byte every 4s: always inside the server's 10s per-read timeout, so
    // only the total request deadline can end this.
    // Detached on purpose: against a server that never hangs up this thread
    // would otherwise keep the test alive long after its assertions fail.
    std::thread::spawn(move || {
        let head = b"GET / HTTP/1.1\r\nHost: x\r\nX-Slow: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        for b in head {
            if writer.write_all(&[*b]).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_secs(4));
        }
    });

    let started = Instant::now();
    let mut buf = Vec::new();
    let result = stream.read_to_end(&mut buf);
    let elapsed = started.elapsed();
    drop(stream);

    assert!(result.is_ok(), "server left the connection hanging: {result:?}");
    assert!(
        elapsed < Duration::from_secs(20),
        "a drip-feeding client held its connection for {elapsed:?}"
    );
    assert!(
        String::from_utf8_lossy(&buf).starts_with("HTTP/1.1 408"),
        "expected a 408, got {:?}",
        String::from_utf8_lossy(&buf)
    );
}

#[test]
fn answers_oversized_and_unsupported_requests_instead_of_dropping() {
    let (_server, port) = start_server("almanac-reject-test.db");

    let big = exchange(
        port,
        b"POST /calendars HTTP/1.1\r\nHost: x\r\nContent-Length: 70000\r\n\r\n",
    );
    assert!(big.starts_with("HTTP/1.1 413"), "{big:?}");

    let chunked = exchange(
        port,
        b"POST /calendars HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n",
    );
    assert!(chunked.starts_with("HTTP/1.1 411"), "{chunked:?}");

    let dup = exchange(
        port,
        b"POST /calendars HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}",
    );
    assert!(dup.starts_with("HTTP/1.1 400"), "{dup:?}");

    let mut huge = b"GET / HTTP/1.1\r\nHost: x\r\nX-Big: ".to_vec();
    huge.resize(huge.len() + 70_000, b'a');
    let too_big = exchange(port, &huge);
    assert!(too_big.starts_with("HTTP/1.1 431"), "{too_big:?}");
}

#[test]
fn honors_expect_100_continue() {
    let (_server, port) = start_server("almanac-continue-test.db");

    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set client timeout");
    stream
        .write_all(
            b"POST /calendars HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\n\
              Accept: application/json\r\nExpect: 100-continue\r\nContent-Length: 2\r\n\r\n",
        )
        .expect("write head");
    let mut interim = [0u8; 25];
    stream.read_exact(&mut interim).expect("read 100 Continue");
    assert_eq!(&interim, b"HTTP/1.1 100 Continue\r\n\r\n");

    stream.write_all(b"{}").expect("write body");
    let mut rest = Vec::new();
    stream.read_to_end(&mut rest).expect("read response");
    let text = String::from_utf8_lossy(&rest);
    assert!(text.starts_with("HTTP/1.1 201"), "{text:?}");
}

#[test]
fn a_rotated_agent_key_applies_on_the_next_start() {
    let db = std::env::temp_dir().join(format!("almanac-rotate-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&db);
    let db = db.to_str().unwrap().to_string();
    let get = |port: u16, key: &str| {
        exchange(
            port,
            format!("GET /v1/events HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {key}\r\n\r\n")
                .as_bytes(),
        )
    };

    let (first, port) = start_server_with(&db, &[("FEED_TOKEN", "feed-1"), ("AGENT_KEY", "key-1")]);
    assert!(get(port, "key-1").starts_with("HTTP/1.1 200"));
    drop(first);

    let (_second, port) = start_server_with(&db, &[("FEED_TOKEN", "feed-1"), ("AGENT_KEY", "key-2")]);
    assert!(get(port, "key-1").starts_with("HTTP/1.1 401"), "old key still works");
    assert!(get(port, "key-2").starts_with("HTTP/1.1 200"), "new key rejected");
    let _ = std::fs::remove_file(&db);
}
