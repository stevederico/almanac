/// RFC 5545 text escaping for SUMMARY / DESCRIPTION / LOCATION.
///
/// Newlines become the `\n` escape and tabs pass through. Any other control
/// character (NUL, bell, escape, DEL, C1) is dropped: TEXT allows none of
/// them, and calendar apps reject or truncate a feed that carries one.
pub fn escape_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .chars()
        .filter(|c| !c.is_control() || *c == '\t')
        .collect()
}

/// Fold a content line at 75 octets. Continuations start with a space.
pub fn fold_line(line: &str) -> String {
    let bytes = line.as_bytes();
    if bytes.len() <= 75 {
        return line.to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    let mut limit = 75;
    while i < bytes.len() {
        let mut end = (i + limit).min(bytes.len());
        while end > i && end < bytes.len() && bytes[end] & 0xc0 == 0x80 {
            end -= 1;
        }
        if end == i {
            end = (i + limit).min(bytes.len());
        }
        parts.push(String::from_utf8_lossy(&bytes[i..end]).into_owned());
        i = end;
        limit = 74;
    }
    let mut out = parts.remove(0);
    for part in parts {
        out.push_str("\r\n ");
        out.push_str(&part);
    }
    out
}

pub use crate::time::{next_date, to_ics_date, to_ics_utc};

#[derive(Debug, Clone)]
pub struct IcsOverride {
    pub recurrence_id: String,
    pub summary: String,
    pub description: String,
    pub location: String,
    pub dtstart: String,
    pub dtend: String,
    pub all_day: bool,
    pub transparent: bool,
    pub sequence: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct IcsEvent {
    pub uid: String,
    pub summary: String,
    pub description: String,
    pub location: String,
    pub dtstart: String,
    pub dtend: String,
    pub all_day: bool,
    pub transparent: bool,
    pub sequence: i64,
    pub updated_at: String,
    pub rrule: String,
    pub tzid: String,
    pub exdates: Vec<String>,
    pub overrides: Vec<IcsOverride>,
}

fn stamp(iso: &str) -> String {
    to_ics_utc(iso).unwrap_or_else(|_| to_ics_utc(&crate::time::now_iso()).expect("now is valid"))
}

fn compact_wall(value: &str) -> String {
    let (date, time) = value.split_once('T').unwrap_or((value, ""));
    let date: String = date.chars().filter(|c| c.is_ascii_digit()).collect();
    let time: String = time
        .chars()
        .filter(|c| c.is_ascii_digit())
        .take(6)
        .collect();
    if time.is_empty() {
        date
    } else {
        format!("{date}T{time}")
    }
}

fn ics_when(name: &str, value: &str, all_day: bool, tzid: &str) -> String {
    if all_day {
        return format!(
            "{name};VALUE=DATE:{}",
            to_ics_date(value).unwrap_or_default()
        );
    }
    if tzid.is_empty() || tzid == "UTC" {
        format!("{name}:{}Z", compact_wall(value))
    } else {
        format!("{name};TZID={tzid}:{}", compact_wall(value))
    }
}

fn vevent_lines(
    uid: &str,
    summary: &str,
    description: &str,
    location: &str,
    dtstart: &str,
    dtend: &str,
    all_day: bool,
    transparent: bool,
    sequence: i64,
    updated_at: &str,
    tzid: &str,
    recurrence_id: Option<&str>,
    rrule: &str,
    exdates: &[String],
) -> Vec<String> {
    let recurring = !rrule.is_empty() || recurrence_id.is_some();
    let zone = if recurring { tzid } else { "" };
    let mut lines = vec![
        "BEGIN:VEVENT".to_string(),
        format!("UID:{uid}"),
        format!("DTSTAMP:{}", stamp(updated_at)),
        format!("LAST-MODIFIED:{}", stamp(updated_at)),
        format!("SEQUENCE:{sequence}"),
        format!("SUMMARY:{}", escape_text(summary)),
    ];
    if let Some(rid) = recurrence_id {
        lines.push(ics_when("RECURRENCE-ID", rid, all_day, zone));
    }
    if recurring {
        lines.push(ics_when("DTSTART", dtstart, all_day, zone));
        lines.push(ics_when("DTEND", dtend, all_day, zone));
    } else if all_day {
        lines.push(format!(
            "DTSTART;VALUE=DATE:{}",
            to_ics_date(dtstart).unwrap_or_default()
        ));
        lines.push(format!(
            "DTEND;VALUE=DATE:{}",
            to_ics_date(dtend).unwrap_or_default()
        ));
    } else {
        lines.push(format!(
            "DTSTART:{}",
            to_ics_utc(dtstart).unwrap_or_default()
        ));
        lines.push(format!("DTEND:{}", to_ics_utc(dtend).unwrap_or_default()));
    }
    if !rrule.is_empty() {
        lines.push(format!("RRULE:{rrule}"));
    }
    if !exdates.is_empty() {
        let dates = exdates
            .iter()
            .map(|d| {
                if all_day {
                    to_ics_date(d).unwrap_or_default()
                } else if zone.is_empty() || zone == "UTC" {
                    format!("{}Z", compact_wall(d))
                } else {
                    compact_wall(d)
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        if all_day {
            lines.push(format!("EXDATE;VALUE=DATE:{dates}"));
        } else if zone.is_empty() || zone == "UTC" {
            lines.push(format!("EXDATE:{dates}"));
        } else {
            lines.push(format!("EXDATE;TZID={zone}:{dates}"));
        }
    }
    if !location.is_empty() {
        lines.push(format!("LOCATION:{}", escape_text(location)));
    }
    if !description.is_empty() {
        lines.push(format!("DESCRIPTION:{}", escape_text(description)));
    }
    lines.push(
        if transparent {
            "TRANSP:TRANSPARENT"
        } else {
            "TRANSP:OPAQUE"
        }
        .to_string(),
    );
    lines.push("END:VEVENT".to_string());
    lines
}

fn vevent(ev: &IcsEvent) -> String {
    let mut lines = vevent_lines(
        &ev.uid,
        &ev.summary,
        &ev.description,
        &ev.location,
        &ev.dtstart,
        &ev.dtend,
        ev.all_day,
        ev.transparent,
        ev.sequence,
        &ev.updated_at,
        &ev.tzid,
        None,
        &ev.rrule,
        &ev.exdates,
    );
    for over in &ev.overrides {
        lines.extend(vevent_lines(
            &ev.uid,
            &over.summary,
            &over.description,
            &over.location,
            &over.dtstart,
            &over.dtend,
            over.all_day,
            over.transparent,
            over.sequence,
            &over.updated_at,
            &ev.tzid,
            Some(&over.recurrence_id),
            "",
            &[],
        ));
    }
    lines
        .into_iter()
        .map(|l| fold_line(&l))
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// Build a PUBLISH calendar. Stable UIDs; SEQUENCE tracks edits.
pub fn render_calendar(cal_name: &str, events: &[IcsEvent]) -> String {
    let name_line = format!("X-WR-CALNAME:{}", escape_text(cal_name));
    let mut lines: Vec<String> = [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "PRODID:-//Steve//almanac//EN",
        "CALSCALE:GREGORIAN",
        "METHOD:PUBLISH",
        name_line.as_str(),
    ]
    .into_iter()
    .map(fold_line)
    .collect();
    let mut zones: Vec<&str> = Vec::new();
    for ev in events {
        if !ev.rrule.is_empty() && !ev.tzid.is_empty() && !zones.contains(&ev.tzid.as_str()) {
            zones.push(ev.tzid.as_str());
        }
    }
    for zone in zones {
        if let Some(block) = crate::tz::vtimezone_lines(zone) {
            lines.extend(block.iter().copied().map(fold_line));
        }
    }
    lines.extend(events.iter().map(vevent));
    lines.push("END:VCALENDAR".into());
    format!("{}\r\n", lines.join("\r\n"))
}

#[derive(Debug, Clone)]
pub struct IcsTodo {
    pub uid: String,
    pub title: String,
    pub description: String,
    /// `""`, `YYYY-MM-DD`, or a UTC instant.
    pub due: String,
    pub done: bool,
    pub completed_at: String,
    pub priority: i64,
    pub tags: Vec<String>,
    pub sequence: i64,
    pub created_at: String,
    pub updated_at: String,
}

fn vtodo(todo: &IcsTodo) -> String {
    let mut lines = vec![
        "BEGIN:VTODO".to_string(),
        format!("UID:{}", todo.uid),
        format!("DTSTAMP:{}", stamp(&todo.updated_at)),
        format!("CREATED:{}", stamp(&todo.created_at)),
        format!("LAST-MODIFIED:{}", stamp(&todo.updated_at)),
        format!("SEQUENCE:{}", todo.sequence),
        format!("SUMMARY:{}", escape_text(&todo.title)),
    ];
    if !todo.description.is_empty() {
        lines.push(format!("DESCRIPTION:{}", escape_text(&todo.description)));
    }
    if crate::time::is_ymd(&todo.due) {
        lines.push(format!(
            "DUE;VALUE=DATE:{}",
            to_ics_date(&todo.due).unwrap_or_default()
        ));
    } else if !todo.due.is_empty() {
        lines.push(format!("DUE:{}", to_ics_utc(&todo.due).unwrap_or_default()));
    }
    if todo.priority > 0 {
        lines.push(format!("PRIORITY:{}", todo.priority));
    }
    if !todo.tags.is_empty() {
        // Each value is escaped; the commas between them are the separator.
        let tags: Vec<String> = todo.tags.iter().map(|t| escape_text(t)).collect();
        lines.push(format!("CATEGORIES:{}", tags.join(",")));
    }
    if todo.done {
        lines.push("STATUS:COMPLETED".to_string());
        lines.push("PERCENT-COMPLETE:100".to_string());
        lines.push(format!("COMPLETED:{}", stamp(&todo.completed_at)));
    } else {
        lines.push("STATUS:NEEDS-ACTION".to_string());
    }
    lines.push("END:VTODO".to_string());
    lines.into_iter().map(|l| fold_line(&l)).collect::<Vec<_>>().join("\r\n")
}

/// A subscribeable calendar holding only `VTODO`s.
pub fn render_todos(cal_name: &str, todos: &[IcsTodo]) -> String {
    let name_line = format!("X-WR-CALNAME:{}", escape_text(&format!("{cal_name} Todos")));
    let mut lines: Vec<String> = [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "PRODID:-//Steve//almanac//EN",
        "CALSCALE:GREGORIAN",
        "METHOD:PUBLISH",
        name_line.as_str(),
    ]
    .into_iter()
    .map(fold_line)
    .collect();
    lines.extend(todos.iter().map(vtodo));
    lines.push("END:VCALENDAR".into());
    format!("{}\r\n", lines.join("\r\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo(title: &str) -> IcsTodo {
        IcsTodo {
            uid: "todo-1".into(),
            title: title.into(),
            description: String::new(),
            due: String::new(),
            done: false,
            completed_at: String::new(),
            priority: 0,
            tags: vec![],
            sequence: 0,
            created_at: "2026-09-01T10:00:00.000Z".into(),
            updated_at: "2026-09-02T11:30:00.000Z".into(),
        }
    }

    #[test]
    fn renders_an_open_todo() {
        let ics = render_todos("Home", &[todo("Buy milk")]);
        assert_eq!(
            ics,
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Steve//almanac//EN\r\n\
             CALSCALE:GREGORIAN\r\nMETHOD:PUBLISH\r\nX-WR-CALNAME:Home Todos\r\n\
             BEGIN:VTODO\r\nUID:todo-1\r\nDTSTAMP:20260902T113000Z\r\n\
             CREATED:20260901T100000Z\r\nLAST-MODIFIED:20260902T113000Z\r\nSEQUENCE:0\r\n\
             SUMMARY:Buy milk\r\nSTATUS:NEEDS-ACTION\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
        );
    }

    #[test]
    fn renders_due_priority_tags_and_completion() {
        let mut t = todo("Ship it");
        t.due = "2026-10-01".into();
        t.priority = 1;
        t.tags = vec!["work".into(), "q4".into()];
        t.done = true;
        t.completed_at = "2026-09-30T18:00:00.000Z".into();
        let ics = render_todos("Home", &[t.clone()]);
        assert!(ics.contains("DUE;VALUE=DATE:20261001\r\n"), "{ics}");
        assert!(ics.contains("PRIORITY:1\r\n"));
        assert!(ics.contains("CATEGORIES:work,q4\r\n"));
        assert!(ics.contains("STATUS:COMPLETED\r\n"));
        assert!(ics.contains("PERCENT-COMPLETE:100\r\n"));
        assert!(ics.contains("COMPLETED:20260930T180000Z\r\n"));
        assert!(!ics.contains("NEEDS-ACTION"));

        t.due = "2026-10-01T16:00:00.000Z".into();
        assert!(render_todos("Home", &[t]).contains("DUE:20261001T160000Z\r\n"));
    }

    #[test]
    fn escapes_and_folds_text_and_never_breaks_a_line() {
        let mut t = todo("Call Bob; then, Alice\\Eve\nnext");
        t.description = "é".repeat(100);
        let ics = render_todos("A;B", &[t]);
        assert!(ics.contains("X-WR-CALNAME:A\\;B Todos\r\n"), "{ics}");
        assert!(ics.contains("SUMMARY:Call Bob\\; then\\, Alice\\\\Eve\\nnext\r\n"), "{ics}");
        for line in ics.split("\r\n") {
            assert!(line.len() <= 75, "unfolded line of {} octets: {line}", line.len());
        }
        // A title cannot inject a property by carrying a newline.
        let evil = render_todos("x", &[todo("a\r\nSTATUS:COMPLETED")]);
        let statuses: Vec<&str> = evil.split("\r\n").filter(|l| l.starts_with("STATUS:")).collect();
        assert_eq!(statuses, ["STATUS:NEEDS-ACTION"], "{evil}");
    }

    #[test]
    fn escapes_ics_specials() {
        assert_eq!(escape_text("a;b,c\\d\ne"), "a\\;b\\,c\\\\d\\ne");
    }

    #[test]
    fn drops_control_characters_but_keeps_tabs_and_newlines() {
        assert_eq!(escape_text("a\u{0}b\u{7}c\u{1b}[0m\u{7f}d\u{85}e"), "abc[0mde");
        assert_eq!(escape_text("a\tb"), "a\tb");
        assert_eq!(escape_text("a\r\nb\rc\nd"), "a\\nb\\nc\\nd");
        assert_eq!(escape_text("é ✓ 日本"), "é ✓ 日本");
    }

    #[test]
    fn leaves_short_lines_alone() {
        assert_eq!(fold_line("SUMMARY:Giants"), "SUMMARY:Giants");
    }

    #[test]
    fn folds_long_lines_at_75_octets() {
        let line = format!("DESCRIPTION:{}", "x".repeat(80));
        let folded = fold_line(&line);
        assert!(folded.contains("\r\n "));
        let first = folded.split("\r\n").next().unwrap();
        assert!(first.len() <= 75);
    }

    #[test]
    fn formats_utc_instants() {
        assert_eq!(
            to_ics_utc("2026-08-16T20:05:00.000Z").unwrap(),
            "20260816T200500Z"
        );
    }

    #[test]
    fn formats_all_day_dates() {
        assert_eq!(to_ics_date("2026-08-16").unwrap(), "20260816");
        assert_eq!(next_date("2026-08-16").unwrap(), "2026-08-17");
    }

    #[test]
    fn emits_a_subscribeable_publish_calendar() {
        let ics = render_calendar(
            "My Calendar",
            &[IcsEvent {
                uid: "game-1@almanac".into(),
                summary: "Rockies @ Giants".into(),
                description: "NBCS BA".into(),
                location: "Oracle Park".into(),
                dtstart: "2026-08-16T20:05:00.000Z".into(),
                dtend: "2026-08-16T23:05:00.000Z".into(),
                all_day: false,
                transparent: true,
                sequence: 0,
                updated_at: "2026-08-13T17:00:00.000Z".into(),
                rrule: String::new(),
                tzid: String::new(),
                exdates: Vec::new(),
                overrides: Vec::new(),
            }],
        );
        assert!(ics.starts_with("BEGIN:VCALENDAR"));
        assert!(ics.contains("X-WR-CALNAME:My Calendar"));
        assert!(ics.contains("UID:game-1@almanac"));
        assert!(ics.contains("DTSTART:20260816T200500Z"));
        assert!(ics.contains("TRANSP:TRANSPARENT"));
        assert!(ics.ends_with("END:VCALENDAR\r\n"));
    }

    #[test]
    fn uses_value_date_for_all_day_events() {
        let ics = render_calendar(
            "My Calendar",
            &[IcsEvent {
                uid: "off-1".into(),
                summary: "Out".into(),
                description: String::new(),
                location: String::new(),
                dtstart: "2026-08-20".into(),
                dtend: "2026-08-21".into(),
                all_day: true,
                transparent: true,
                sequence: 1,
                updated_at: "2026-08-13T17:00:00.000Z".into(),
                rrule: String::new(),
                tzid: String::new(),
                exdates: Vec::new(),
                overrides: Vec::new(),
            }],
        );
        assert!(ics.contains("DTSTART;VALUE=DATE:20260820"));
        assert!(ics.contains("DTEND;VALUE=DATE:20260821"));
        assert!(!ics.contains("RRULE:"));
    }

    #[test]
    fn emits_rrule_timezone_exdate_and_override() {
        let ics = render_calendar(
            "My Calendar",
            &[IcsEvent {
                uid: "standup".into(),
                summary: "Standup".into(),
                description: String::new(),
                location: String::new(),
                dtstart: "2026-09-22T09:00:00".into(),
                dtend: "2026-09-22T09:15:00".into(),
                all_day: false,
                transparent: true,
                sequence: 2,
                updated_at: "2026-09-01T17:00:00.000Z".into(),
                rrule: "FREQ=WEEKLY;BYDAY=TU".into(),
                tzid: "America/Los_Angeles".into(),
                exdates: vec!["2026-10-06T09:00:00".into()],
                overrides: vec![IcsOverride {
                    recurrence_id: "2026-10-13T09:00:00".into(),
                    summary: "Standup late".into(),
                    description: String::new(),
                    location: String::new(),
                    dtstart: "2026-10-13T10:00:00".into(),
                    dtend: "2026-10-13T10:15:00".into(),
                    all_day: false,
                    transparent: true,
                    sequence: 1,
                    updated_at: "2026-09-02T17:00:00.000Z".into(),
                }],
            }],
        );
        assert!(ics.contains("BEGIN:VTIMEZONE"));
        assert!(ics.contains("TZID:America/Los_Angeles"));
        assert!(ics.contains("DTSTART;TZID=America/Los_Angeles:20260922T090000"));
        assert!(ics.contains("DTEND;TZID=America/Los_Angeles:20260922T091500"));
        assert!(ics.contains("RRULE:FREQ=WEEKLY;BYDAY=TU"));
        assert!(ics.contains("EXDATE;TZID=America/Los_Angeles:20261006T090000"));
        assert!(ics.contains("RECURRENCE-ID;TZID=America/Los_Angeles:20261013T090000"));
        assert!(ics.contains("SUMMARY:Standup late"));
        assert_eq!(ics.matches("BEGIN:VTIMEZONE").count(), 1);
        assert!(!ics.contains("DTSTART:20260922T090000Z"));
    }

    #[test]
    fn emits_all_day_rrule_without_a_timezone() {
        let ics = render_calendar(
            "My Calendar",
            &[IcsEvent {
                uid: "off".into(),
                summary: "Off".into(),
                description: String::new(),
                location: String::new(),
                dtstart: "2026-09-22".into(),
                dtend: "2026-09-23".into(),
                all_day: true,
                transparent: true,
                sequence: 0,
                updated_at: "2026-09-01T17:00:00.000Z".into(),
                rrule: "FREQ=WEEKLY".into(),
                tzid: String::new(),
                exdates: vec!["2026-10-06".into()],
                overrides: Vec::new(),
            }],
        );
        assert!(ics.contains("DTSTART;VALUE=DATE:20260922"));
        assert!(ics.contains("RRULE:FREQ=WEEKLY"));
        assert!(ics.contains("EXDATE;VALUE=DATE:20261006"));
        assert!(!ics.contains("TZID="));
        assert!(!ics.contains("VTIMEZONE"));
    }
}
