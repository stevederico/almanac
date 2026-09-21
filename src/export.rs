//! Everything a calendar holds, as one document.
//!
//! The point is that nobody is locked in: a calendar can be read out whole,
//! as JSON for a machine or Markdown for a person, with the one write key.

use crate::db::{json_calendar, json_event, CalendarRow, EventRow};
use crate::json::Value;
use crate::notes::{json_note, NoteRow};
use crate::todos::{json_todo, TodoRow};

pub fn export_json(
    cal: &CalendarRow,
    exported_at: &str,
    events: &[EventRow],
    todos: &[TodoRow],
    notes: &[NoteRow],
) -> Value {
    // The link fields describe hosts and tokens, which an export has no use for.
    let name = json_calendar(cal, "");
    let calendar = Value::object(&[
        ("id", Value::String(cal.id.clone())),
        ("name", name.get("name").cloned().unwrap_or(Value::Null)),
        ("createdAt", Value::String(cal.created_at.clone())),
    ]);
    Value::object(&[
        ("exportedAt", Value::String(exported_at.to_string())),
        ("calendar", calendar),
        ("events", Value::Array(events.iter().map(json_event).collect())),
        ("todos", Value::Array(todos.iter().map(json_todo).collect())),
        (
            "notes",
            Value::Array(notes.iter().map(|n| json_note(n, true)).collect()),
        ),
    ])
}

/// One line: a newline in a title or summary would break the list around it.
fn one_line(text: &str) -> String {
    text.split(['\r', '\n'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn export_markdown(
    cal: &CalendarRow,
    exported_at: &str,
    events: &[EventRow],
    todos: &[TodoRow],
    notes: &[NoteRow],
) -> String {
    let mut out = format!("# {}\n\nExported {exported_at}\n", one_line(&cal.name));

    out.push_str("\n## Events\n\n");
    if events.is_empty() {
        out.push_str("None.\n");
    }
    for ev in events {
        if !ev.seal.is_empty() {
            out.push_str(&format!("- sealed · `{}`\n", ev.uid));
            continue;
        }
        out.push_str(&format!("- {} to {} — {}", ev.dtstart, ev.dtend, one_line(&ev.summary)));
        if !ev.location.is_empty() {
            out.push_str(&format!(" ({})", one_line(&ev.location)));
        }
        if !ev.rrule.is_empty() {
            out.push_str(&format!(" · repeats {}", ev.rrule));
            if !ev.tzid.is_empty() {
                out.push_str(&format!(" {}", ev.tzid));
            }
        }
        out.push_str(&format!(" · `{}`\n", ev.uid));
    }

    out.push_str("\n## Todos\n\n");
    if todos.is_empty() {
        out.push_str("None.\n");
    }
    for todo in todos {
        if !todo.seal.is_empty() {
            out.push_str(&format!("- sealed · `{}`\n", todo.uid));
            continue;
        }
        out.push_str(&format!(
            "- [{}] {}",
            if todo.done { "x" } else { " " },
            one_line(&todo.title)
        ));
        if !todo.due.is_empty() {
            out.push_str(&format!(" — due {}", todo.due));
        }
        if todo.priority > 0 {
            out.push_str(&format!(" · priority {}", todo.priority));
        }
        for tag in &todo.tags {
            out.push_str(&format!(" · #{tag}"));
        }
        out.push('\n');
        if !todo.description.is_empty() {
            for line in todo.description.lines() {
                out.push_str(&format!("  {line}\n"));
            }
        }
    }

    out.push_str("\n## Notes\n");
    if notes.is_empty() {
        out.push_str("\nNone.\n");
    }
    for note in notes {
        if !note.seal.is_empty() {
            out.push_str(&format!("\n### sealed\n\n`{}`\n", note.uid));
            continue;
        }
        out.push_str(&format!("\n### {}\n\n", one_line(&note.title)));
        let mut facts = vec![format!("updated {}", note.updated_at)];
        if note.pinned {
            facts.insert(0, "pinned".to_string());
        }
        if !note.tags.is_empty() {
            facts.insert(0, format!("tags: {}", note.tags.join(", ")));
        }
        out.push_str(&format!("*{}*\n", facts.join(" · ")));
        if !note.body.is_empty() {
            out.push_str(&format!("\n{}\n", note.body.trim_end()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cal() -> CalendarRow {
        CalendarRow {
            id: "cal_1".into(),
            feed_token: "SECRET-FEED".into(),
            key_hash: "SECRET-HASH".into(),
            name: "Trip\nPlan".into(),
            created_at: "2026-09-01T00:00:00.000Z".into(),
            feed: "plain".into(),
        }
    }

    fn todo(title: &str, done: bool) -> TodoRow {
        TodoRow {
            uid: "t".into(),
            title: title.into(),
            description: "line one\nline two".into(),
            due: "2026-10-01".into(),
            priority: 1,
            done,
            completed_at: String::new(),
            tags: vec!["home".into(), "q4".into()],
            sequence: 0,
            created_at: "c".into(),
            updated_at: "u".into(),
            seal: String::new(),
        }
    }

    fn note() -> NoteRow {
        NoteRow {
            uid: "n".into(),
            title: "Ideas\nfor later".into(),
            body: "# Heading\n- one\n\n".into(),
            tags: vec!["work".into()],
            pinned: true,
            sequence: 0,
            created_at: "c".into(),
            updated_at: "2026-09-02T00:00:00.000Z".into(),
            seal: String::new(),
        }
    }

    #[test]
    fn json_carries_everything_but_no_secrets() {
        let v = export_json(&cal(), "now", &[], &[todo("Milk", false)], &[note()]);
        let text = crate::json::stringify(&v);
        assert!(!text.contains("SECRET"), "{text}");
        assert!(!text.contains("subscribe"), "{text}");
        assert_eq!(v.get("todos").and_then(Value::as_array).unwrap().len(), 1);
        let notes = v.get("notes").and_then(Value::as_array).unwrap();
        assert_eq!(notes[0].get("body").and_then(Value::as_str), Some("# Heading\n- one\n\n"));
    }

    #[test]
    fn markdown_reads_like_a_document() {
        let md = export_markdown(&cal(), "2026-09-03T00:00:00.000Z", &[], &[todo("Milk", false), todo("Done", true)], &[note()]);
        assert_eq!(
            md,
            "# Trip Plan\n\nExported 2026-09-03T00:00:00.000Z\n\
             \n## Events\n\nNone.\n\
             \n## Todos\n\n\
             - [ ] Milk — due 2026-10-01 · priority 1 · #home · #q4\n  line one\n  line two\n\
             - [x] Done — due 2026-10-01 · priority 1 · #home · #q4\n  line one\n  line two\n\
             \n## Notes\n\
             \n### Ideas for later\n\n\
             *tags: work · pinned · updated 2026-09-02T00:00:00.000Z*\n\
             \n# Heading\n- one\n"
        );
        assert!(!md.contains("SECRET"));
    }

    #[test]
    fn an_empty_calendar_still_exports() {
        let md = export_markdown(&cal(), "now", &[], &[], &[]);
        assert_eq!(md.matches("None.").count(), 3);
    }
}
