//! Item seals. AES-256-GCM via libcrypto. The server stores the wire string
//! and never sees the write key.
//!
//! `alm1.` + base64url(nonce || ciphertext || tag). The key is SHA-256 of the
//! write key and a fixed domain. The associated data is calendar id, kind, and
//! uid, so a seal cannot be moved to another row.

use std::ffi::c_int;
use std::ptr;

use crate::json::Value;
use crate::sha256::sha256;
use crate::util::random_bytes;

const DOMAIN: &[u8] = b"almanac-seal-v1";
const PREFIX: &str = "alm1.";
const NONCE: usize = 12;
const TAG: usize = 16;
/// Fits in the 64KB request cap once wrapped as `{"seal":"..."}`.
pub const MAX_WIRE: usize = 60_000;

const EVP_CTRL_GCM_SET_IVLEN: c_int = 0x9;
const EVP_CTRL_GCM_GET_TAG: c_int = 0x10;
const EVP_CTRL_GCM_SET_TAG: c_int = 0x11;

#[link(name = "crypto")]
extern "C" {
    fn EVP_aes_256_gcm() -> *const u8;
    fn EVP_CIPHER_CTX_new() -> *mut u8;
    fn EVP_CIPHER_CTX_free(ctx: *mut u8);
    fn EVP_EncryptInit_ex(
        ctx: *mut u8,
        cipher: *const u8,
        engine: *mut u8,
        key: *const u8,
        iv: *const u8,
    ) -> c_int;
    fn EVP_EncryptUpdate(
        ctx: *mut u8,
        out: *mut u8,
        outl: *mut c_int,
        input: *const u8,
        inl: c_int,
    ) -> c_int;
    fn EVP_EncryptFinal_ex(ctx: *mut u8, out: *mut u8, outl: *mut c_int) -> c_int;
    fn EVP_DecryptInit_ex(
        ctx: *mut u8,
        cipher: *const u8,
        engine: *mut u8,
        key: *const u8,
        iv: *const u8,
    ) -> c_int;
    fn EVP_DecryptUpdate(
        ctx: *mut u8,
        out: *mut u8,
        outl: *mut c_int,
        input: *const u8,
        inl: c_int,
    ) -> c_int;
    fn EVP_DecryptFinal_ex(ctx: *mut u8, out: *mut u8, outl: *mut c_int) -> c_int;
    fn EVP_CIPHER_CTX_ctrl(ctx: *mut u8, typ: c_int, arg: c_int, ptr: *mut u8) -> c_int;
}

struct Ctx(*mut u8);

impl Drop for Ctx {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { EVP_CIPHER_CTX_free(self.0) }
        }
    }
}

fn derive(write_key: &str) -> [u8; 32] {
    let mut msg = Vec::with_capacity(write_key.len() + DOMAIN.len());
    msg.extend_from_slice(write_key.as_bytes());
    msg.extend_from_slice(DOMAIN);
    sha256(&msg)
}

fn aad(calendar_id: &str, kind: &str, uid: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(calendar_id.len() + kind.len() + uid.len() + 2);
    out.extend_from_slice(calendar_id.as_bytes());
    out.push(0);
    out.extend_from_slice(kind.as_bytes());
    out.push(0);
    out.extend_from_slice(uid.as_bytes());
    out
}

pub fn kind_ok(kind: &str) -> bool {
    matches!(kind, "event" | "todo" | "note")
}

pub fn seal(
    write_key: &str,
    calendar_id: &str,
    kind: &str,
    uid: &str,
    plaintext: &[u8],
) -> Result<String, String> {
    if !kind_ok(kind) {
        return Err("kind must be event, todo, or note".into());
    }
    if uid.is_empty() {
        return Err("uid is required".into());
    }
    if plaintext.len() > 48 * 1024 {
        return Err("item is too long".into());
    }
    let key = derive(write_key);
    let nonce = random_bytes(NONCE);
    let extra = aad(calendar_id, kind, uid);
    let ctx = Ctx(unsafe { EVP_CIPHER_CTX_new() });
    if ctx.0.is_null() {
        return Err("seal failed".into());
    }
    let ok = unsafe {
        EVP_EncryptInit_ex(ctx.0, EVP_aes_256_gcm(), ptr::null_mut(), ptr::null(), ptr::null()) == 1
            && EVP_CIPHER_CTX_ctrl(
                ctx.0,
                EVP_CTRL_GCM_SET_IVLEN,
                NONCE as c_int,
                ptr::null_mut(),
            ) == 1
            && EVP_EncryptInit_ex(ctx.0, ptr::null(), ptr::null_mut(), key.as_ptr(), nonce.as_ptr())
                == 1
    };
    if !ok {
        return Err("seal failed".into());
    }
    let mut ignored = 0;
    let aad_ok = unsafe {
        EVP_EncryptUpdate(
            ctx.0,
            ptr::null_mut(),
            &mut ignored,
            extra.as_ptr(),
            extra.len() as c_int,
        ) == 1
    };
    if !aad_ok {
        return Err("seal failed".into());
    }
    let mut out = vec![0u8; plaintext.len() + 16];
    let mut out_len = 0;
    let updated = unsafe {
        EVP_EncryptUpdate(
            ctx.0,
            out.as_mut_ptr(),
            &mut out_len,
            plaintext.as_ptr(),
            plaintext.len() as c_int,
        ) == 1
    };
    if !updated {
        return Err("seal failed".into());
    }
    let mut final_len = 0;
    let finished = unsafe {
        EVP_EncryptFinal_ex(ctx.0, out.as_mut_ptr().add(out_len as usize), &mut final_len) == 1
    };
    if !finished {
        return Err("seal failed".into());
    }
    let n = out_len as usize + final_len as usize;
    let mut tag = [0u8; TAG];
    let tagged = unsafe {
        EVP_CIPHER_CTX_ctrl(
            ctx.0,
            EVP_CTRL_GCM_GET_TAG,
            TAG as c_int,
            tag.as_mut_ptr(),
        ) == 1
    };
    if !tagged {
        return Err("seal failed".into());
    }
    let mut packed = Vec::with_capacity(NONCE + n + TAG);
    packed.extend_from_slice(&nonce);
    packed.extend_from_slice(&out[..n]);
    packed.extend_from_slice(&tag);
    let wire = format!("{PREFIX}{}", base64url(&packed));
    if wire.len() > MAX_WIRE {
        return Err("seal is too long".into());
    }
    Ok(wire)
}

pub fn open(
    write_key: &str,
    calendar_id: &str,
    kind: &str,
    uid: &str,
    wire: &str,
) -> Result<Vec<u8>, String> {
    if !kind_ok(kind) {
        return Err("kind must be event, todo, or note".into());
    }
    let packed = decode_wire(wire)?;
    if packed.len() < NONCE + TAG {
        return Err("seal is invalid".into());
    }
    let (nonce, rest) = packed.split_at(NONCE);
    let (ciphertext, tag) = rest.split_at(rest.len() - TAG);
    let key = derive(write_key);
    let extra = aad(calendar_id, kind, uid);
    let ctx = Ctx(unsafe { EVP_CIPHER_CTX_new() });
    if ctx.0.is_null() {
        return Err("seal was rejected".into());
    }
    let ok = unsafe {
        EVP_DecryptInit_ex(ctx.0, EVP_aes_256_gcm(), ptr::null_mut(), ptr::null(), ptr::null()) == 1
            && EVP_CIPHER_CTX_ctrl(
                ctx.0,
                EVP_CTRL_GCM_SET_IVLEN,
                NONCE as c_int,
                ptr::null_mut(),
            ) == 1
            && EVP_DecryptInit_ex(ctx.0, ptr::null(), ptr::null_mut(), key.as_ptr(), nonce.as_ptr())
                == 1
    };
    if !ok {
        return Err("seal was rejected".into());
    }
    let mut ignored = 0;
    let aad_ok = unsafe {
        EVP_DecryptUpdate(
            ctx.0,
            ptr::null_mut(),
            &mut ignored,
            extra.as_ptr(),
            extra.len() as c_int,
        ) == 1
    };
    if !aad_ok {
        return Err("seal was rejected".into());
    }
    let mut out = vec![0u8; ciphertext.len() + 16];
    let mut out_len = 0;
    let updated = unsafe {
        EVP_DecryptUpdate(
            ctx.0,
            out.as_mut_ptr(),
            &mut out_len,
            ciphertext.as_ptr(),
            ciphertext.len() as c_int,
        ) == 1
    };
    if !updated {
        return Err("seal was rejected".into());
    }
    let tag_ok = unsafe {
        EVP_CIPHER_CTX_ctrl(
            ctx.0,
            EVP_CTRL_GCM_SET_TAG,
            TAG as c_int,
            tag.as_ptr() as *mut u8,
        ) == 1
    };
    if !tag_ok {
        return Err("seal was rejected".into());
    }
    let mut final_len = 0;
    let finished = unsafe {
        EVP_DecryptFinal_ex(ctx.0, out.as_mut_ptr().add(out_len as usize), &mut final_len) == 1
    };
    if !finished {
        return Err("seal was rejected".into());
    }
    out.truncate(out_len as usize + final_len as usize);
    Ok(out)
}

pub fn validate_wire(wire: &str) -> Result<(), String> {
    decode_wire(wire).map(|_| ())
}

fn decode_wire(wire: &str) -> Result<Vec<u8>, String> {
    let rest = wire.strip_prefix(PREFIX).ok_or("seal is invalid")?;
    if rest.is_empty() || rest.len() > MAX_WIRE || wire.len() > MAX_WIRE {
        return Err("seal is invalid".into());
    }
    if !rest.bytes().all(|b| {
        b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'='
    }) {
        return Err("seal is invalid".into());
    }
    let packed = base64url_decode(rest).map_err(|_| "seal is invalid")?;
    if packed.len() < NONCE + TAG {
        return Err("seal is invalid".into());
    }
    Ok(packed)
}

/// `Some(seal)` when the body is a seal write. `None` when it is a field write.
/// A sealed calendar rejects a field write. A seal body may also carry `uid`.
pub fn classify(body: &Value, feed_mode: &str) -> Result<Option<String>, String> {
    let obj = body.as_object().ok_or("body must be an object")?;
    if obj.contains_key("seal") {
        if obj.keys().any(|k| k != "seal" && k != "uid") {
            return Err("seal replaces the item".into());
        }
        let wire = obj
            .get("seal")
            .and_then(Value::as_str)
            .ok_or("seal must be a string")?;
        validate_wire(wire)?;
        return Ok(Some(wire.to_string()));
    }
    if feed_mode == "seal" {
        return Err("calendar is sealed".into());
    }
    Ok(None)
}

pub fn feed_mode(value: &str) -> Result<&'static str, String> {
    match value {
        "seal" => Ok("seal"),
        "plain" => Ok("plain"),
        _ => Err("feed must be seal or plain".into()),
    }
}

fn base64url(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = u32::from_be_bytes([0, bytes[i], bytes[i + 1], bytes[i + 2]]);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push(T[(n & 63) as usize] as char);
        i += 3;
    }
    if i < bytes.len() {
        let b0 = bytes[i];
        let b1 = if i + 1 < bytes.len() { bytes[i + 1] } else { 0 };
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        if i + 1 < bytes.len() {
            out.push(T[((n >> 6) & 63) as usize] as char);
        }
    }
    out
}

fn base64url_decode(text: &str) -> Result<Vec<u8>, ()> {
    fn val(b: u8) -> Result<u8, ()> {
        Ok(match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return Err(()),
        })
    }
    let text = text.trim_end_matches('=');
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut i = 0;
    while i + 4 <= bytes.len() {
        let n = (u32::from(val(bytes[i])?) << 18)
            | (u32::from(val(bytes[i + 1])?) << 12)
            | (u32::from(val(bytes[i + 2])?) << 6)
            | u32::from(val(bytes[i + 3])?);
        out.push((n >> 16) as u8);
        out.push((n >> 8) as u8);
        out.push(n as u8);
        i += 4;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        return Err(());
    }
    if rem >= 2 {
        let n = (u32::from(val(bytes[i])?) << 18) | (u32::from(val(bytes[i + 1])?) << 12);
        out.push((n >> 16) as u8);
        if rem == 3 {
            let n = n | (u32::from(val(bytes[i + 2])?) << 6);
            out.push((n >> 8) as u8);
        }
    }
    Ok(out)
}

/// `almanac seal|open`. The key is read from the credentials file and never printed.
pub fn run(cmd: &str, args: &[String]) -> Result<String, String> {
    let mut cal: Option<String> = None;
    let mut kind: Option<String> = None;
    let mut uid: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        let value = args.get(i + 1).ok_or_else(|| format!("missing value for {flag}"))?;
        match flag {
            "--cal" => cal = Some(value.clone()),
            "--kind" => kind = Some(value.clone()),
            "--uid" => uid = Some(value.clone()),
            _ => return Err("usage: almanac seal|open --cal ID --kind event|todo|note --uid UID".into()),
        }
        i += 2;
    }
    let kind = kind.ok_or("kind is required")?;
    let uid = uid.ok_or("uid is required")?;
    let (calendar_id, key) = load_key(cal.as_deref())?;
    let mut stdin = Vec::new();
    std::io::Read::read_to_end(&mut std::io::stdin(), &mut stdin).map_err(|e| e.to_string())?;
    if cmd == "seal" {
        let wire = seal(&key, &calendar_id, &kind, &uid, &stdin)?;
        Ok(wire)
    } else {
        let text = std::str::from_utf8(&stdin).map_err(|_| "seal is invalid")?;
        let plain = open(&key, &calendar_id, &kind, &uid, text.trim())?;
        let text = String::from_utf8(plain).map_err(|_| "seal was rejected")?;
        Ok(text)
    }
}

fn load_key(want: Option<&str>) -> Result<(String, String), String> {
    let path = std::env::var("ALMANAC_CREDS").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.config/almanac/hosted-calendars.json")
    });
    let text = std::fs::read_to_string(&path).map_err(|_| "could not read the calendar key".to_string())?;
    let value = crate::json::parse(&text).map_err(|_| "could not read the calendar key".to_string())?;
    let rows = match &value {
        Value::Array(rows) => rows.as_slice(),
        Value::Object(_) => {
            return one_row(&value, want);
        }
        _ => return Err("could not read the calendar key".into()),
    };
    if rows.is_empty() {
        return Err("could not read the calendar key".into());
    }
    if let Some(want) = want {
        for row in rows {
            if row.get("id").and_then(Value::as_str) == Some(want) {
                return one_row(row, Some(want));
            }
        }
        return Err("could not read the calendar key".into());
    }
    one_row(&rows[0], None)
}

fn one_row(row: &Value, want: Option<&str>) -> Result<(String, String), String> {
    let id = row
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("could not read the calendar key")?;
    if let Some(want) = want {
        if id != want {
            return Err("could not read the calendar key".into());
        }
    }
    let key = row
        .get("key")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("could not read the calendar key")?;
    Ok((id.to_string(), key.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_hides_the_title() {
        let plain = br#"{"summary":"note-body-plaintext-sentinel-9f3a","start":"2026-08-18T09:00:00-07:00"}"#;
        let wire = seal("test-db-key", "cal_test", "event", "dentist", plain).unwrap();
        assert!(wire.starts_with("alm1."));
        assert!(!wire.contains("sentinel"));
        assert!(!wire.contains("summary"));
        let opened = open("test-db-key", "cal_test", "event", "dentist", &wire).unwrap();
        assert_eq!(opened, plain);
    }

    #[test]
    fn a_wrong_key_or_a_moved_seal_is_rejected() {
        let wire = seal("test-db-key", "cal_test", "note", "ideas", b"{\"title\":\"x\"}").unwrap();
        assert!(open("other-key", "cal_test", "note", "ideas", &wire).is_err());
        assert!(open("test-db-key", "cal_other", "note", "ideas", &wire).is_err());
        assert!(open("test-db-key", "cal_test", "todo", "ideas", &wire).is_err());
        assert!(open("test-db-key", "cal_test", "note", "other", &wire).is_err());
        let file = std::fs::read("src/seal.rs").unwrap_or_default();
        assert!(!file.windows(wire.len()).any(|w| w == wire.as_bytes()));
    }

    #[test]
    fn a_sealed_calendar_rejects_a_field_write() {
        let body = crate::json::parse(r#"{"summary":"Dentist","start":"2026-08-18"}"#).unwrap();
        assert_eq!(classify(&body, "seal").unwrap_err(), "calendar is sealed");
        let wire = seal("k", "c", "todo", "milk", b"{\"title\":\"Buy milk\"}").unwrap();
        let sealed = crate::json::parse(&format!(r#"{{"seal":"{wire}"}}"#)).unwrap();
        assert_eq!(classify(&sealed, "seal").unwrap().as_deref(), Some(wire.as_str()));
        let mixed = crate::json::parse(&format!(r#"{{"seal":"{wire}","title":"x"}}"#)).unwrap();
        assert_eq!(classify(&mixed, "plain").unwrap_err(), "seal replaces the item");
    }
}
