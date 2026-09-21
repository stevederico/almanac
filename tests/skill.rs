//! The agent skill lives in the repo so it changes with the API. These keep it
//! honest: it must describe every resource, and it must never carry a secret.

use std::fs;
use std::path::PathBuf;

fn skill() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills/almanac/SKILL.md");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn has_the_front_matter_a_skill_loader_needs() {
    let text = skill();
    let head: Vec<&str> = text.lines().take(8).collect();
    assert_eq!(head[0], "---");
    assert!(head.contains(&"name: almanac"), "{head:?}");
    assert!(head.iter().any(|l| l.starts_with("description:")), "{head:?}");
    assert!(head.iter().skip(1).any(|l| *l == "---"), "front matter is never closed");
}

#[test]
fn documents_every_resource_the_api_serves() {
    let text = skill();
    for kind in ["events", "todos", "notes", "export"] {
        assert!(
            text.contains(&format!("/v1/c/$ID/{kind}")),
            "the skill does not show how to use {kind}"
        );
    }
    // The rules an agent trips over.
    for word in ["User-Agent", "PUT", "429", "shown once"] {
        assert!(text.contains(word), "the skill never mentions {word}");
    }
}

#[test]
fn carries_no_secret_and_no_hardcoded_calendar() {
    let text = skill();
    // Keys and feed tokens are long runs of hex; a calendar id is `cal_` + 16 hex.
    let mut run = 0;
    for c in text.chars() {
        run = if c.is_ascii_hexdigit() && !c.is_ascii_uppercase() { run + 1 } else { 0 };
        assert!(run < 32, "a key-length hex string is in the skill");
    }
    assert!(!text.contains("cal_1"), "a real calendar id is hardcoded");
    assert!(text.contains("hosted-calendars.json"), "credentials must come from the file");
}

#[test]
fn every_skill_file_is_covered_by_these_checks() {
    // A second file in the skill directory would escape the secret scan above.
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills/almanac");
    let names: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["SKILL.md"], "add new skill files to the checks in tests/skill.rs");
}
