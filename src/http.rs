use std::collections::HashMap;
use std::io::{Read, Write};

const MAX_HEADERS: usize = 64 * 1024;
const MAX_BODY: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn new(method: &str, path: &str) -> Self {
        Self {
            method: method.to_string(),
            path: path.to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        }
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

pub fn read_request(stream: &mut impl Read) -> Result<Request, String> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    let header_end;
    loop {
        let n = stream.read(&mut tmp).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("empty request".into());
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = find_double_crlf(&buf) {
            header_end = pos;
            break;
        }
        if buf.len() > MAX_HEADERS {
            return Err("headers too large".into());
        }
    }
    let (mut req, already) = parse_head(&buf, header_end)?;
    let want = content_length(&req);
    if want > MAX_BODY {
        return Err("body too large".into());
    }
    req.body = already;
    while req.body.len() < want {
        let n = stream.read(&mut tmp).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        req.body.extend_from_slice(&tmp[..n]);
        if req.body.len() > MAX_BODY {
            return Err("body too large".into());
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

fn parse_head(buf: &[u8], header_end: usize) -> Result<(Request, Vec<u8>), String> {
    let head = std::str::from_utf8(&buf[..header_end]).map_err(|_| "headers not utf-8")?;
    let mut lines = head.split("\r\n");
    let start = lines.next().ok_or("empty request")?;
    let mut parts = start.splitn(3, ' ');
    let method = parts.next().ok_or("bad request line")?.to_string();
    let path = parts.next().ok_or("bad request line")?.to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
    }
    let already = buf[header_end..].to_vec();
    Ok((
        Request {
            method,
            path,
            headers,
            body: Vec::new(),
        },
        already,
    ))
}

fn content_length(req: &Request) -> usize {
    req.header("content-length").parse().unwrap_or(0)
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    }
}
