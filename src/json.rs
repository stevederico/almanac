use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(i64),
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

impl Value {
    pub fn object(pairs: &[(&str, Value)]) -> Self {
        Value::Object(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), v.clone()))
                .collect(),
        )
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Value::Object(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_object()?.get(key)
    }
}

pub fn stringify(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value);
    out
}

fn write_value(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => write_string(out, s),
        Value::Array(items) => {
            out.push('[');
            for (i, v) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, v);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (k, v)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(out, k);
                out.push(':');
                write_value(out, v);
            }
            out.push('}');
        }
    }
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Deepest object/array nesting `parse` accepts. Recursive descent would
/// otherwise overflow the stack on `[[[[...`, and a stack overflow aborts the
/// whole process rather than one worker thread.
const MAX_DEPTH: usize = 32;

pub fn parse(input: &str) -> Result<Value, String> {
    let mut p = Parser {
        src: input,
        bytes: input.as_bytes(),
        i: 0,
        depth: 0,
    };
    p.skip_ws();
    let v = p.value()?;
    p.skip_ws();
    if p.i != p.bytes.len() {
        return Err("trailing json".into());
    }
    Ok(v)
}

struct Parser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    i: usize,
    depth: usize,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while self.i < self.bytes.len() && self.bytes[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.i).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.i += 1;
        Some(b)
    }

    fn value(&mut self) -> Result<Value, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.nested(Self::object),
            Some(b'[') => self.nested(Self::array),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b't') => self.ident(b"true", Value::Bool(true)),
            Some(b'f') => self.ident(b"false", Value::Bool(false)),
            Some(b'n') => self.ident(b"null", Value::Null),
            Some(b'-') | Some(b'0'..=b'9') => self.number(),
            _ => Err("invalid json".into()),
        }
    }

    fn nested(&mut self, f: fn(&mut Self) -> Result<Value, String>) -> Result<Value, String> {
        if self.depth >= MAX_DEPTH {
            return Err("json nested too deep".into());
        }
        self.depth += 1;
        let v = f(self);
        self.depth -= 1;
        v
    }

    fn ident(&mut self, expected: &[u8], v: Value) -> Result<Value, String> {
        if self.bytes.get(self.i..self.i + expected.len()) != Some(expected) {
            return Err("invalid json".into());
        }
        self.i += expected.len();
        Ok(v)
    }

    fn object(&mut self) -> Result<Value, String> {
        self.bump();
        let mut map = BTreeMap::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.bump();
            return Ok(Value::Object(map));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err("object key must be a string".into());
            }
            let key = self.string()?;
            self.skip_ws();
            if self.bump() != Some(b':') {
                return Err("expected :".into());
            }
            let val = self.value()?;
            map.insert(key, val);
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b'}') => break,
                _ => return Err("expected }".into()),
            }
        }
        Ok(Value::Object(map))
    }

    fn array(&mut self) -> Result<Value, String> {
        self.bump();
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.bump();
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b']') => break,
                _ => return Err("expected ]".into()),
            }
        }
        Ok(Value::Array(items))
    }

    fn string(&mut self) -> Result<String, String> {
        if self.bump() != Some(b'"') {
            return Err("expected string".into());
        }
        let mut out = String::new();
        loop {
            // Copy the run up to the next delimiter in one slice. Every
            // delimiter is ASCII, so both ends sit on char boundaries.
            let start = self.i;
            while self.i < self.bytes.len() {
                let b = self.bytes[self.i];
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.i += 1;
            }
            out.push_str(&self.src[start..self.i]);
            match self.bump() {
                None => return Err("unterminated string".into()),
                Some(b'"') => return Ok(out),
                Some(b'\\') => match self.bump() {
                    Some(b'"') => out.push('"'),
                    Some(b'\\') => out.push('\\'),
                    Some(b'/') => out.push('/'),
                    Some(b'n') => out.push('\n'),
                    Some(b'r') => out.push('\r'),
                    Some(b't') => out.push('\t'),
                    Some(b'u') => {
                        let hex = self.bytes.get(self.i..self.i + 4).ok_or("bad \\u")?;
                        let n = u32::from_str_radix(
                            std::str::from_utf8(hex).map_err(|_| "bad \\u")?,
                            16,
                        )
                        .map_err(|_| "bad \\u")?;
                        self.i += 4;
                        out.push(char::from_u32(n).ok_or("bad \\u")?);
                    }
                    _ => return Err("bad escape".into()),
                },
                Some(_) => return Err("raw control in string".into()),
            }
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.bump();
        }
        if self.peek() == Some(b'0') {
            self.bump();
        } else if matches!(self.peek(), Some(b'1'..=b'9')) {
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.bump();
            }
        } else {
            return Err("invalid number".into());
        }
        if self.peek() == Some(b'.') || self.peek() == Some(b'e') || self.peek() == Some(b'E') {
            return Err("floats not supported".into());
        }
        let s = std::str::from_utf8(&self.bytes[start..self.i]).map_err(|_| "invalid number")?;
        let n: i64 = s.parse().map_err(|_| "invalid number")?;
        Ok(Value::Number(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_object() {
        let v = parse(r#"{"summary":"Dinner","allDay":false,"n":1}"#).unwrap();
        assert_eq!(v.get("summary").and_then(Value::as_str), Some("Dinner"));
        assert_eq!(v.get("allDay").and_then(Value::as_bool), Some(false));
        let back = parse(&stringify(&v)).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn caps_nesting_depth() {
        let ok = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
        assert!(parse(&ok).is_ok());
        let deep = format!("{}{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
        assert!(parse(&deep).is_err());
        assert!(parse(&"[".repeat(200_000)).is_err());
        assert!(parse(&"{\"a\":".repeat(200_000)).is_err());
    }

    #[test]
    fn parses_long_unicode_strings_fast() {
        let body = format!("{{\"name\":\"{}\"}}", "é".repeat(500_000));
        let t = std::time::Instant::now();
        let v = parse(&body).unwrap();
        assert!(t.elapsed().as_millis() < 500);
        assert_eq!(v.get("name").and_then(Value::as_str).unwrap().chars().count(), 500_000);
    }

    #[test]
    fn keeps_escapes_and_mixed_text() {
        let v = parse(r#"{"a":"x\ny\u00e9 é \"q\" \\"}"#).unwrap();
        assert_eq!(v.get("a").and_then(Value::as_str), Some("x\ny\u{e9} é \"q\" \\"));
        assert!(parse("\"a\u{1}b\"").is_err());
        assert!(parse("\"unterminated").is_err());
    }
}
