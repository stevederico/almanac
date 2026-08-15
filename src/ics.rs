/// RFC 5545 text escaping for SUMMARY / DESCRIPTION / LOCATION.
pub fn escape_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,")
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

/// Instant → `YYYYMMDDTHHMMSSZ`.
pub fn to_ics_utc(iso: &str) -> Result<String, String> {
    let dt =
        chrono::DateTime::parse_from_rfc3339(iso).map_err(|_| format!("invalid date: {iso}"))?;
    Ok(dt
        .with_timezone(&chrono::Utc)
        .format("%Y%m%dT%H%M%SZ")
        .to_string())
}

/// Calendar date `YYYY-MM-DD` → `YYYYMMDD`.
pub fn to_ics_date(ymd: &str) -> Result<String, String> {
    if ymd.len() != 10
        || ymd.as_bytes().get(4) != Some(&b'-')
        || ymd.as_bytes().get(7) != Some(&b'-')
    {
        return Err(format!("invalid all-day date: {ymd}"));
    }
    let y = &ymd[0..4];
    let m = &ymd[5..7];
    let d = &ymd[8..10];
    if !y.bytes().all(|b| b.is_ascii_digit())
        || !m.bytes().all(|b| b.is_ascii_digit())
        || !d.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(format!("invalid all-day date: {ymd}"));
    }
    Ok(format!("{y}{m}{d}"))
}

/// Exclusive next DATE for all-day DTEND.
pub fn next_date(ymd: &str) -> Result<String, String> {
    let d = chrono::NaiveDate::parse_from_str(ymd, "%Y-%m-%d")
        .map_err(|_| format!("invalid all-day date: {ymd}"))?;
    let next = d
        .succ_opt()
        .ok_or_else(|| format!("invalid all-day date: {ymd}"))?;
    Ok(next.format("%Y-%m-%d").to_string())
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
}

fn stamp(iso: &str) -> String {
    to_ics_utc(iso).unwrap_or_else(|_| {
        to_ics_utc(&chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
            .expect("now is valid")
    })
}

fn vevent(ev: &IcsEvent) -> String {
    let mut lines = vec![
        "BEGIN:VEVENT".to_string(),
        format!("UID:{}", ev.uid),
        format!("DTSTAMP:{}", stamp(&ev.updated_at)),
        format!("LAST-MODIFIED:{}", stamp(&ev.updated_at)),
        format!("SEQUENCE:{}", ev.sequence),
        format!("SUMMARY:{}", escape_text(&ev.summary)),
    ];
    if ev.all_day {
        lines.push(format!(
            "DTSTART;VALUE=DATE:{}",
            to_ics_date(&ev.dtstart).unwrap_or_default()
        ));
        lines.push(format!(
            "DTEND;VALUE=DATE:{}",
            to_ics_date(&ev.dtend).unwrap_or_default()
        ));
    } else {
        lines.push(format!(
            "DTSTART:{}",
            to_ics_utc(&ev.dtstart).unwrap_or_default()
        ));
        lines.push(format!(
            "DTEND:{}",
            to_ics_utc(&ev.dtend).unwrap_or_default()
        ));
    }
    if !ev.location.is_empty() {
        lines.push(format!("LOCATION:{}", escape_text(&ev.location)));
    }
    if !ev.description.is_empty() {
        lines.push(format!("DESCRIPTION:{}", escape_text(&ev.description)));
    }
    lines.push(
        if ev.transparent {
            "TRANSP:TRANSPARENT"
        } else {
            "TRANSP:OPAQUE"
        }
        .to_string(),
    );
    lines.push("END:VEVENT".to_string());
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
    lines.extend(events.iter().map(vevent));
    lines.push("END:VCALENDAR".into());
    format!("{}\r\n", lines.join("\r\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_ics_specials() {
        assert_eq!(escape_text("a;b,c\\d\ne"), "a\\;b\\,c\\\\d\\ne");
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
            }],
        );
        assert!(ics.contains("DTSTART;VALUE=DATE:20260820"));
        assert!(ics.contains("DTEND;VALUE=DATE:20260821"));
    }
}
