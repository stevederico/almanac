use std::collections::HashMap;
use std::io::{self, Read, Write};

const MAX_HEADERS: usize = 64 * 1024;
/// Every field is capped at 4000 chars, so a legitimate body is far smaller.
const MAX_BODY: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    /// Socket peer address. Empty when not from a socket (tests).
    pub peer: String,
}

impl Request {
    pub fn new(method: &str, path: &str) -> Self {
        Self {
            method: method.to_string(),
            path: path.to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
            peer: String::new(),
        }
    }

    pub fn with_peer(mut self, peer: &str) -> Self {
        self.peer = peer.to_string();
        self
    }

    pub fn with_header(mut self, key: &str, value: &str) -> Self {
        self.headers
            .insert(key.to_ascii_lowercase(), value.to_string());
        self
    }

    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    pub fn header(&self, key: &str) -> &str {
        self.headers
            .get(&key.to_ascii_lowercase())
            .map(String::as_str)
            .unwrap_or("")
    }
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn header(mut self, key: &str, value: &str) -> Self {
        self.headers.push((key.to_string(), value.to_string()));
        self
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    pub fn header_value(&self, key: &str) -> Option<&str> {
        let key = key.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&key))
            .map(|(_, v)| v.as_str())
    }
}

pub fn json_response(status: u16, body: &str) -> Response {
    Response::new(status)
        .header("content-type", "application/json; charset=utf-8")
        .body(body.as_bytes().to_vec())
}

pub fn text_response(status: u16, ctype: &str, body: impl Into<Vec<u8>>) -> Response {
    Response::new(status)
        .header("content-type", ctype)
        .body(body)
}

pub fn html_response(status: u16, body: String) -> Response {
    text_response(status, "text/html; charset=utf-8", body)
}

/// Why a request could not be read.
#[derive(Debug, PartialEq, Eq)]
pub enum ReadError {
    /// The peer is gone or sent nothing usable. There is no one to answer.
    Closed(String),
    /// Answer with this status and message, then close.
    Reject(u16, &'static str),
}

fn io_error(e: io::Error) -> ReadError {
    match e.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {
            ReadError::Reject(408, "request timeout")
        }
        _ => ReadError::Closed(e.to_string()),
    }
}

pub fn read_request(stream: &mut impl Read) -> Result<Request, ReadError> {
    read_request_with(stream, &mut || {})
}

/// Read one request. `on_continue` runs when the client sent
/// `Expect: 100-continue` and is waiting for a go-ahead before its body.
pub fn read_request_with(
    stream: &mut impl Read,
    on_continue: &mut dyn FnMut(),
) -> Result<Request, ReadError> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    let header_end;
    loop {
        let n = stream.read(&mut tmp).map_err(io_error)?;
        if n == 0 {
            return Err(ReadError::Closed(if buf.is_empty() {
                "empty request".into()
            } else {
                "closed mid-headers".into()
            }));
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = find_double_crlf(&buf) {
            header_end = pos;
            break;
        }
        if buf.len() > MAX_HEADERS {
            return Err(ReadError::Reject(431, "headers too large"));
        }
    }
    if header_end > MAX_HEADERS {
        return Err(ReadError::Reject(431, "headers too large"));
    }
    let (mut req, already) = parse_head(&buf, header_end)?;
    if req.headers.contains_key("transfer-encoding") {
        return Err(ReadError::Reject(
            411,
            "chunked bodies are not supported, send content-length",
        ));
    }
    let want = content_length(&req)?;
    if want > MAX_BODY {
        return Err(ReadError::Reject(413, "body too large"));
    }
    req.body = already;
    if req.body.len() < want && req.header("expect").eq_ignore_ascii_case("100-continue") {
        on_continue();
    }
    while req.body.len() < want {
        let n = stream.read(&mut tmp).map_err(io_error)?;
        if n == 0 {
            return Err(ReadError::Closed("closed mid-body".into()));
        }
        req.body.extend_from_slice(&tmp[..n]);
        if req.body.len() > MAX_BODY {
            return Err(ReadError::Reject(413, "body too large"));
        }
    }
    req.body.truncate(want);
    Ok(req)
}

pub fn write_response(stream: &mut impl Write, res: &Response) -> Result<(), String> {
    let reason = reason(res.status);
    let mut head = format!("HTTP/1.1 {} {reason}\r\n", res.status);
    let mut has_len = false;
    for (k, v) in &res.headers {
        if k.eq_ignore_ascii_case("content-length") {
            has_len = true;
        }
        head.push_str(k);
        head.push_str(": ");
        head.push_str(v);
        head.push_str("\r\n");
    }
    if !has_len {
        head.push_str(&format!("content-length: {}\r\n", res.body.len()));
    }
    head.push_str("connection: close\r\n\r\n");
    stream
        .write_all(head.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.write_all(&res.body).map_err(|e| e.to_string())?;
    Ok(())
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

fn parse_head(buf: &[u8], header_end: usize) -> Result<(Request, Vec<u8>), ReadError> {
    let bad = bad_request();
    let head = std::str::from_utf8(&buf[..header_end]).map_err(|_| bad_request())?;
    let mut lines = head.split("\r\n");
    let start = lines.next().ok_or_else(bad_request)?;
    let mut parts = start.splitn(3, ' ');
    let method = parts.next().filter(|m| !m.is_empty()).ok_or_else(bad_request)?;
    let path = parts.next().filter(|p| !p.is_empty()).ok_or_else(bad_request)?;
    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let key = k.trim().to_ascii_lowercase();
        // Two lengths is how a request gets read differently by two parsers.
        if key == "content-length" && headers.contains_key(&key) {
            return Err(bad);
        }
        headers.insert(key, v.trim().to_string());
    }
    let already = buf[header_end..].to_vec();
    Ok((
        Request {
            method: method.to_string(),
            path: path.to_string(),
            headers,
            body: Vec::new(),
            peer: String::new(),
        },
        already,
    ))
}

fn bad_request() -> ReadError {
    ReadError::Reject(400, "bad request")
}

fn content_length(req: &Request) -> Result<usize, ReadError> {
    let raw = req.header("content-length");
    if raw.is_empty() {
        return Ok(0);
    }
    if !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ReadError::Reject(400, "bad content-length"));
    }
    // Digits that overflow usize are a body too large, not a bad header.
    Ok(raw.parse().unwrap_or(usize::MAX))
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        408 => "Request Timeout",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "OK",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(raw: &[u8]) -> Result<Request, ReadError> {
        read_request(&mut &raw[..])
    }

    #[test]
    fn reads_a_request_with_a_body() {
        let req = read(b"POST /x HTTP/1.1\r\nContent-Length: 2\r\n\r\nhi").unwrap();
        assert_eq!((req.method.as_str(), req.path.as_str()), ("POST", "/x"));
        assert_eq!(req.body, b"hi");
    }

    #[test]
    fn rejects_an_oversized_body_by_its_header() {
        let raw = format!("POST /x HTTP/1.1\r\nContent-Length: {}\r\n\r\n", MAX_BODY + 1);
        assert_eq!(read(raw.as_bytes()).unwrap_err(), ReadError::Reject(413, "body too large"));
        let huge = b"POST /x HTTP/1.1\r\nContent-Length: 99999999999999999999999\r\n\r\n";
        assert_eq!(read(huge).unwrap_err(), ReadError::Reject(413, "body too large"));
    }

    #[test]
    fn rejects_oversized_headers() {
        let mut raw = b"GET / HTTP/1.1\r\nX: ".to_vec();
        raw.resize(raw.len() + MAX_HEADERS + 10, b'a');
        assert_eq!(read(&raw).unwrap_err(), ReadError::Reject(431, "headers too large"));
    }

    #[test]
    fn rejects_chunked_and_bad_lengths() {
        let chunked = b"POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n";
        assert!(matches!(read(chunked), Err(ReadError::Reject(411, _))));
        let dup = b"POST /x HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\nab";
        assert!(matches!(read(dup), Err(ReadError::Reject(400, _))));
        for bad in ["-1", "+1", "1x", "0x10"] {
            let raw = format!("POST /x HTTP/1.1\r\nContent-Length: {bad}\r\n\r\n");
            assert!(
                matches!(read(raw.as_bytes()), Err(ReadError::Reject(400, _))),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_short_body_is_closed_not_served_truncated() {
        let raw = b"POST /x HTTP/1.1\r\nContent-Length: 10\r\n\r\nhi";
        assert!(matches!(read(raw), Err(ReadError::Closed(_))));
    }

    #[test]
    fn a_timed_out_read_asks_for_408() {
        struct Stalled;
        impl Read for Stalled {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::ErrorKind::WouldBlock.into())
            }
        }
        assert_eq!(
            read_request(&mut Stalled).unwrap_err(),
            ReadError::Reject(408, "request timeout")
        );
    }

    #[test]
    fn expect_continue_fires_only_when_the_body_is_missing() {
        let mut fired = 0;
        let head = b"POST /x HTTP/1.1\r\nExpect: 100-continue\r\nContent-Length: 2\r\n\r\n";
        let mut raw = head.to_vec();
        let mut cb = || fired += 1;
        let _ = read_request_with(&mut &raw[..], &mut cb);
        raw.extend_from_slice(b"hi");
        let req = read_request_with(&mut &raw[..], &mut cb).unwrap();
        assert_eq!(req.body, b"hi");
        assert_eq!(fired, 1);
    }
}
