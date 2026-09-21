//! Nothing that works as a credential may be committed. Failures name the file
//! and never the secret.

use std::path::PathBuf;
use std::process::Command;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn git(args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(repo())
        .output()
        .expect("git is needed to check what is tracked")
}

fn tracked() -> Vec<(String, Vec<u8>)> {
    let out = git(&["ls-files", "-z"]);
    out.stdout
        .split(|b| *b == 0)
        .filter(|n| !n.is_empty())
        .map(|n| String::from_utf8_lossy(n).into_owned())
        // Deleted but not yet committed, or a submodule: nothing to read.
        .filter_map(|name| std::fs::read(repo().join(&name)).ok().map(|bytes| (name, bytes)))
        .collect()
}

/// Does `bytes` hold a run of exactly `len` lowercase hex digits?
fn has_hex_run(bytes: &[u8], len: usize) -> bool {
    let is_hex = |b: u8| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || (b'A'..=b'F').contains(&b);
    let mut run = 0;
    let mut lower_only = true;
    for b in bytes.iter().copied().chain(std::iter::once(b' ')) {
        if is_hex(b) {
            run += 1;
            lower_only &= !b.is_ascii_uppercase();
        } else {
            if run == len && lower_only {
                return true;
            }
            run = 0;
            lower_only = true;
        }
    }
    false
}

#[test]
fn no_tracked_file_holds_a_key_sized_hex_string() {
    // Every key and feed token is 24 random bytes as hex: 48 characters.
    // (SHA-256 hashes are 64, so test vectors and stored hashes pass.)
    let bad: Vec<String> = tracked()
        .into_iter()
        .filter(|(_, bytes)| has_hex_run(bytes, 48))
        .map(|(name, _)| name)
        .collect();
    assert!(bad.is_empty(), "a key-sized hex string is committed in: {bad:?}");
}

#[test]
fn none_of_this_machines_credentials_are_in_a_tracked_file() {
    let Some(home) = std::env::var_os("HOME") else { return };
    let path = PathBuf::from(home).join(".config/almanac/hosted-calendars.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return; // No credentials here (CI): the hex check above still applies.
    };
    let list = almanac::json::parse(&text).expect("credentials file is JSON");
    let mut secrets: Vec<String> = Vec::new();
    for cal in list.as_array().unwrap_or(&[]) {
        if let Some(key) = cal.get("key").and_then(almanac::json::Value::as_str) {
            secrets.push(key.to_string());
        }
        let subscribe = cal.get("subscribe").and_then(almanac::json::Value::as_str);
        if let Some(token) = subscribe.and_then(|s| s.split("/feed/").nth(1)) {
            secrets.push(token.split(['.', '/', '?']).next().unwrap_or("").to_string());
        }
    }
    secrets.retain(|s| s.len() >= 16);
    assert!(!secrets.is_empty(), "the credentials file has no key to check against");
    let leaked: Vec<String> = tracked()
        .into_iter()
        .filter(|(_, bytes)| {
            secrets
                .iter()
                .any(|s| bytes.windows(s.len()).any(|w| w == s.as_bytes()))
        })
        .map(|(name, _)| name)
        .collect();
    assert!(leaked.is_empty(), "one of your credentials is committed in: {leaked:?}");
}

#[test]
fn credential_and_data_paths_are_git_ignored() {
    for path in [
        ".env",
        ".env.production",
        "hosted-calendars.json",
        "skills/almanac/hosted-calendars.json",
        "calendar.db",
        "data/calendar.db",
        "calendar.db.pre-0.8.0",
        "calendar.db-journal",
        "backups/almanac-20260921T000000000Z.db",
    ] {
        let ignored = git(&["check-ignore", "-q", path]).status.success();
        assert!(ignored, "{path} would be committed by `git add .`");
    }
    // The example file has to stay committable.
    assert!(!git(&["check-ignore", "-q", ".env.example"]).status.success());
}

#[test]
fn the_scanner_agrees_with_these_tests() {
    // Built at run time: a literal 48-hex fixture here would trip the very
    // check this file runs, and the commit hook.
    let key = "0123456789abcdef".repeat(3);
    assert_eq!(key.len(), 48);
    assert!(has_hex_run(format!("key={key}.").as_bytes(), 48));
    assert!(!has_hex_run(format!("{key}0").as_bytes(), 48), "49 is not a key");
    assert!(!has_hex_run("a".repeat(64).as_bytes(), 48), "a 64-char sha256 is not a key");
    assert!(!has_hex_run(b"cal_ab12cd34ef56ab78", 48));
}
