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
    let port = free_port();
    let child = Command::new(env!("CARGO_BIN_EXE_almanac"))
        .env("PORT", port.to_string())
        .env("HOST", "127.0.0.1")
        .env("DB_PATH", db)
        .env("AGENT_KEY", "test-agent-key")
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
