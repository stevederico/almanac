use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::ics::next_date;

#[derive(Debug, Clone)]
pub struct CalendarRow {
    pub id: String,
    pub feed_token: String,
    pub agent_key: String,
    pub name: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct EventRow {
    pub uid: String,
    pub summary: String,
    pub description: String,
    pub location: String,
    pub dtstart: String,
    pub dtend: String,
    pub all_day: bool,
    pub transparent: bool,
    pub sequence: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default)]
pub struct EventInput {
    pub uid: Option<String>,
    pub summary: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start: String,
    pub end: Option<String>,
    pub all_day: Option<bool>,
    pub transparent: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct EventPatch {
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub all_day: Option<bool>,
    pub transparent: Option<bool>,
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS calendars (
                id TEXT PRIMARY KEY,
                feed_token TEXT NOT NULL UNIQUE,
                agent_key TEXT NOT NULL,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL
            );",
        )
        .map_err(|e| e.to_string())?;
        migrate_events(&conn)?;
        Ok(Self { conn })
    }

    pub fn get_calendar(&self, id: &str) -> Result<Option<CalendarRow>, String> {
        self.conn
            .query_row(
                "SELECT id, feed_token, agent_key, name, created_at FROM calendars WHERE id = ?1",
                [id],
                row_calendar,
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub fn get_calendar_by_feed_token(&self, token: &str) -> Result<Option<CalendarRow>, String> {
        self.conn
            .query_row(
                "SELECT id, feed_token, agent_key, name, created_at FROM calendars WHERE feed_token = ?1",
                [token],
                row_calendar,
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub fn create_calendar(
        &self,
        name: Option<&str>,
        id: Option<&str>,
        feed_token: Option<&str>,
        agent_key: Option<&str>,
    ) -> Result<CalendarRow, String> {
        let name = name.unwrap_or("Almanac").trim();
        let name = if name.is_empty() { "Almanac" } else { name };
        if name.len() > 80 {
            return Err("name is too long".into());
        }
        let generated_id = format!("cal_{}", hex_encode(&random_bytes(8)));
        let id = id.unwrap_or(&generated_id);
        let now = now_iso();
        let feed = feed_token.map(str::to_string).unwrap_or_else(secret);
        let key = agent_key.map(str::to_string).unwrap_or_else(secret);
        self.conn
            .execute(
                "INSERT INTO calendars (id, feed_token, agent_key, name, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, feed, key, name, now],
            )
            .map_err(|e| e.to_string())?;
        self.get_calendar(id)?
            .ok_or_else(|| "failed to create calendar".into())
    }

    pub fn ensure_home_calendar(
        &self,
        feed_token: &str,
        agent_key: &str,
        name: &str,
    ) -> Result<CalendarRow, String> {
        if let Some(existing) = self.get_calendar("home")? {
            return Ok(existing);
        }
        if let Some(by_token) = self.get_calendar_by_feed_token(feed_token)? {
            return Ok(by_token);
        }
        self.create_calendar(Some(name), Some("home"), Some(feed_token), Some(agent_key))
    }

    pub fn list_events(&self, calendar_id: &str) -> Result<Vec<EventRow>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT uid, summary, description, location, dtstart, dtend,
                        all_day, transparent, sequence, created_at, updated_at
                 FROM events WHERE calendar_id = ?1 ORDER BY dtstart ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([calendar_id], row_event)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn get_event(&self, calendar_id: &str, uid: &str) -> Result<Option<EventRow>, String> {
        self.conn
            .query_row(
                "SELECT uid, summary, description, location, dtstart, dtend,
                        all_day, transparent, sequence, created_at, updated_at
                 FROM events WHERE calendar_id = ?1 AND uid = ?2",
                params![calendar_id, uid],
                row_event,
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub fn upsert_event(&self, calendar_id: &str, input: &EventInput) -> Result<EventRow, String> {
        let all_day = input.all_day == Some(true) || is_ymd(&input.start);
        let times = resolve_times(&input.start, input.end.as_deref(), all_day)?;
        let now = now_iso();
        let generated = format!("evt-{}", Uuid::new_v4());
        let uid = input.uid.as_deref().unwrap_or(&generated);
        if !is_uid(uid) {
            return Err("uid must be 1-200 chars: letters, digits, . _ @ -".into());
        }
        let existing = self.get_event(calendar_id, uid)?;
        let description = input
            .description
            .clone()
            .or_else(|| existing.as_ref().map(|e| e.description.clone()))
            .unwrap_or_default();
        let location = input
            .location
            .clone()
            .or_else(|| existing.as_ref().map(|e| e.location.clone()))
            .unwrap_or_default();
        let transparent = input
            .transparent
            .or_else(|| existing.as_ref().map(|e| e.transparent))
            .unwrap_or(true);
        if existing.is_some() {
            self.conn
                .execute(
                    "UPDATE events
                     SET summary = ?1, description = ?2, location = ?3, dtstart = ?4, dtend = ?5,
                         all_day = ?6, transparent = ?7, sequence = sequence + 1, updated_at = ?8
                     WHERE calendar_id = ?9 AND uid = ?10",
                    params![
                        input.summary,
                        description,
                        location,
                        times.dtstart,
                        times.dtend,
                        times.all_day as i64,
                        transparent as i64,
                        now,
                        calendar_id,
                        uid,
                    ],
                )
                .map_err(|e| e.to_string())?;
        } else {
            self.conn
                .execute(
                    "INSERT INTO events (
                        calendar_id, uid, summary, description, location, dtstart, dtend,
                        all_day, transparent, sequence, created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?11)",
                    params![
                        calendar_id,
                        uid,
                        input.summary,
                        description,
                        location,
                        times.dtstart,
                        times.dtend,
                        times.all_day as i64,
                        transparent as i64,
                        now,
                        now,
                    ],
                )
                .map_err(|e| e.to_string())?;
        }
        self.get_event(calendar_id, uid)?
            .ok_or_else(|| "failed to save event".into())
    }

    pub fn patch_event(
        &self,
        calendar_id: &str,
        uid: &str,
        patch: &EventPatch,
    ) -> Result<EventRow, String> {
        let existing = self
            .get_event(calendar_id, uid)?
            .ok_or_else(|| "not found".to_string())?;
        let start = patch.start.as_deref().unwrap_or(&existing.dtstart);
        let all_day = patch.all_day.unwrap_or(existing.all_day);
        let end = patch.end.as_deref().unwrap_or(&existing.dtend);
        let times = resolve_times(start, Some(end), all_day)?;
        let now = now_iso();
        self.conn
            .execute(
                "UPDATE events
                 SET summary = ?1, description = ?2, location = ?3, dtstart = ?4, dtend = ?5,
                     all_day = ?6, transparent = ?7, sequence = sequence + 1, updated_at = ?8
                 WHERE calendar_id = ?9 AND uid = ?10",
                params![
                    patch.summary.as_deref().unwrap_or(&existing.summary),
                    patch
                        .description
                        .as_deref()
                        .unwrap_or(&existing.description),
                    patch.location.as_deref().unwrap_or(&existing.location),
                    times.dtstart,
                    times.dtend,
                    times.all_day as i64,
                    patch.transparent.unwrap_or(existing.transparent) as i64,
                    now,
                    calendar_id,
                    uid,
                ],
            )
            .map_err(|e| e.to_string())?;
        self.get_event(calendar_id, uid)?
            .ok_or_else(|| "failed to save event".into())
    }

    pub fn delete_event(&self, calendar_id: &str, uid: &str) -> Result<bool, String> {
        let n = self
            .conn
            .execute(
                "DELETE FROM events WHERE calendar_id = ?1 AND uid = ?2",
                params![calendar_id, uid],
            )
            .map_err(|e| e.to_string())?;
        Ok(n > 0)
    }
}

fn row_calendar(row: &rusqlite::Row<'_>) -> rusqlite::Result<CalendarRow> {
    Ok(CalendarRow {
        id: row.get(0)?,
        feed_token: row.get(1)?,
        agent_key: row.get(2)?,
        name: row.get(3)?,
        created_at: row.get(4)?,
    })
}

fn row_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRow> {
    let all_day: i64 = row.get(6)?;
    let transparent: i64 = row.get(7)?;
    Ok(EventRow {
        uid: row.get(0)?,
        summary: row.get(1)?,
        description: row.get(2)?,
        location: row.get(3)?,
        dtstart: row.get(4)?,
        dtend: row.get(5)?,
        all_day: all_day == 1,
        transparent: transparent == 1,
        sequence: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

fn migrate_events(db: &Connection) -> Result<(), String> {
    let tables = table_names(db)?;
    if !tables.iter().any(|t| t == "events") {
        db.execute_batch(
            "CREATE TABLE events (
                calendar_id TEXT NOT NULL,
                uid TEXT NOT NULL,
                summary TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                location TEXT NOT NULL DEFAULT '',
                dtstart TEXT NOT NULL,
                dtend TEXT NOT NULL,
                all_day INTEGER NOT NULL DEFAULT 0,
                transparent INTEGER NOT NULL DEFAULT 1,
                sequence INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (calendar_id, uid)
            );
            CREATE INDEX IF NOT EXISTS idx_events_cal_start ON events(calendar_id, dtstart);",
        )
        .map_err(|e| e.to_string())?;
        return Ok(());
    }
    let cols = column_names(db, "events")?;
    if cols.iter().any(|c| c == "calendar_id") {
        return Ok(());
    }
    db.execute_batch(
        "CREATE TABLE events_new (
            calendar_id TEXT NOT NULL,
            uid TEXT NOT NULL,
            summary TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            location TEXT NOT NULL DEFAULT '',
            dtstart TEXT NOT NULL,
            dtend TEXT NOT NULL,
            all_day INTEGER NOT NULL DEFAULT 0,
            transparent INTEGER NOT NULL DEFAULT 1,
            sequence INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (calendar_id, uid)
        );
        INSERT INTO events_new (
            calendar_id, uid, summary, description, location, dtstart, dtend,
            all_day, transparent, sequence, created_at, updated_at
        )
        SELECT 'home', uid, summary, description, location, dtstart, dtend,
               all_day, transparent, sequence, created_at, updated_at
        FROM events;
        DROP TABLE events;
        ALTER TABLE events_new RENAME TO events;
        CREATE INDEX IF NOT EXISTS idx_events_cal_start ON events(calendar_id, dtstart);",
    )
    .map_err(|e| e.to_string())
}

fn table_names(db: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = db
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn column_names(db: &Connection, table: &str) -> Result<Vec<String>, String> {
    let mut stmt = db
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn secret() -> String {
    hex_encode(&random_bytes(24))
}

fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    getrandom::getrandom(&mut buf).expect("rng");
    buf
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn is_uid(value: &str) -> bool {
    value.len() <= 200
        && !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'@' | b'-'))
}

fn is_ymd(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
}

fn parse_instant(value: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(value).ok().map(|d| {
        d.with_timezone(&chrono::Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    })
}

fn add_hour(iso: &str) -> String {
    let dt = chrono::DateTime::parse_from_rfc3339(iso)
        .expect("stored instant")
        .with_timezone(&chrono::Utc);
    (dt + chrono::Duration::hours(1)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

struct Times {
    dtstart: String,
    dtend: String,
    all_day: bool,
}

fn resolve_times(start: &str, end: Option<&str>, all_day: bool) -> Result<Times, String> {
    if all_day || is_ymd(start) {
        if !is_ymd(start) {
            return Err("all-day start must be YYYY-MM-DD".into());
        }
        let end_ymd = match end {
            Some(e) => e.to_string(),
            None => next_date(start)?,
        };
        if !is_ymd(&end_ymd) {
            return Err("all-day end must be YYYY-MM-DD".into());
        }
        if end_ymd.as_str() <= start {
            return Err("end must be after start".into());
        }
        return Ok(Times {
            dtstart: start.to_string(),
            dtend: end_ymd,
            all_day: true,
        });
    }
    let dtstart = parse_instant(start).ok_or("start must be an ISO-8601 datetime")?;
    let dtend = match end {
        Some(e) => parse_instant(e).ok_or("end must be an ISO-8601 datetime")?,
        None => add_hour(&dtstart),
    };
    if dtend <= dtstart {
        return Err("end must be after start".into());
    }
    Ok(Times {
        dtstart,
        dtend,
        all_day: false,
    })
}

pub fn parse_event_input(body: &Value) -> Result<EventInput, String> {
    let obj = body.as_object().ok_or("body must be an object")?;
    let summary = obj
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("summary is required")?;
    if summary.len() > 512 {
        return Err("summary is too long".into());
    }
    let start = obj
        .get("start")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("start is required")?;
    let uid = obj.get("uid").and_then(Value::as_str).map(str::trim);
    if let Some(uid) = uid {
        if !is_uid(uid) {
            return Err("uid must be 1-200 chars: letters, digits, . _ @ -".into());
        }
    }
    let description = obj.get("description").and_then(Value::as_str);
    let location = obj.get("location").and_then(Value::as_str);
    if let Some(d) = description {
        if d.len() > 4000 {
            return Err("description is too long".into());
        }
    }
    if let Some(l) = location {
        if l.len() > 512 {
            return Err("location is too long".into());
        }
    }
    Ok(EventInput {
        uid: uid.map(str::to_string),
        summary: summary.to_string(),
        description: description.map(str::to_string),
        location: location.map(str::to_string),
        start: start.to_string(),
        end: obj.get("end").and_then(Value::as_str).map(str::to_string),
        all_day: obj.get("allDay").and_then(Value::as_bool),
        transparent: obj.get("transparent").and_then(Value::as_bool),
    })
}

pub fn parse_event_patch(body: &Value) -> Result<EventPatch, String> {
    let obj = body.as_object().ok_or("body must be an object")?;
    let mut patch = EventPatch::default();
    if let Some(summary) = obj.get("summary").and_then(Value::as_str) {
        let trimmed = summary.trim();
        if trimmed.is_empty() {
            return Err("summary cannot be empty".into());
        }
        if trimmed.len() > 512 {
            return Err("summary is too long".into());
        }
        patch.summary = Some(trimmed.to_string());
    }
    if let Some(start) = obj.get("start").and_then(Value::as_str) {
        patch.start = Some(start.trim().to_string());
    }
    if let Some(end) = obj.get("end").and_then(Value::as_str) {
        patch.end = Some(end.trim().to_string());
    }
    if let Some(description) = obj.get("description").and_then(Value::as_str) {
        if description.len() > 4000 {
            return Err("description is too long".into());
        }
        patch.description = Some(description.to_string());
    }
    if let Some(location) = obj.get("location").and_then(Value::as_str) {
        if location.len() > 512 {
            return Err("location is too long".into());
        }
        patch.location = Some(location.to_string());
    }
    if let Some(v) = obj.get("allDay").and_then(Value::as_bool) {
        patch.all_day = Some(v);
    }
    if let Some(v) = obj.get("transparent").and_then(Value::as_bool) {
        patch.transparent = Some(v);
    }
    if patch.summary.is_none()
        && patch.description.is_none()
        && patch.location.is_none()
        && patch.start.is_none()
        && patch.end.is_none()
        && patch.all_day.is_none()
        && patch.transparent.is_none()
    {
        return Err("no fields to update".into());
    }
    Ok(patch)
}

pub fn json_calendar(row: &CalendarRow, base: &str) -> Value {
    let origin = base.trim_end_matches('/');
    json!({
        "id": row.id,
        "name": row.name,
        "subscribe": format!("{origin}/feed/{}.ics", row.feed_token),
        "write": format!("{origin}/v1/c/{}/events", row.id),
        "key": row.agent_key,
        "createdAt": row.created_at,
    })
}

pub fn json_event(row: &EventRow) -> Value {
    json!({
        "uid": row.uid,
        "summary": row.summary,
        "description": row.description,
        "location": row.location,
        "start": row.dtstart,
        "end": row.dtend,
        "allDay": row.all_day,
        "transparent": row.transparent,
        "sequence": row.sequence,
        "createdAt": row.created_at,
        "updatedAt": row.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_db() -> (Db, String) {
        let dir = std::env::temp_dir().join(format!("almanac-db-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Db::open(dir.join("calendar.db")).unwrap();
        let cal = db.create_calendar(Some("Test"), None, None, None).unwrap();
        (db, cal.id)
    }

    #[test]
    fn requires_summary_and_start() {
        let miss = parse_event_input(&json!({ "summary": "x" }));
        assert!(miss.is_err());
        let ok = parse_event_input(&json!({
            "summary": "Dinner",
            "start": "2026-08-16T19:00:00-07:00"
        }))
        .unwrap();
        assert_eq!(ok.summary, "Dinner");
    }

    #[test]
    fn rejects_a_bad_uid() {
        let bad = parse_event_input(&json!({
            "uid": "has spaces",
            "summary": "x",
            "start": "2026-08-16T19:00:00Z"
        }));
        assert!(bad.is_err());
    }

    #[test]
    fn creates_then_bumps_sequence_on_the_same_uid() {
        let (db, cal_id) = tmp_db();
        let a = db
            .upsert_event(
                &cal_id,
                &EventInput {
                    uid: Some("dinner-1".into()),
                    summary: "Dinner".into(),
                    start: "2026-08-16T19:00:00.000Z".into(),
                    end: Some("2026-08-16T21:00:00.000Z".into()),
                    ..EventInput::default()
                },
            )
            .unwrap();
        assert_eq!(a.sequence, 0);
        let b = db
            .upsert_event(
                &cal_id,
                &EventInput {
                    uid: Some("dinner-1".into()),
                    summary: "Dinner moved".into(),
                    start: "2026-08-16T20:00:00.000Z".into(),
                    ..EventInput::default()
                },
            )
            .unwrap();
        assert_eq!(b.summary, "Dinner moved");
        assert_eq!(b.sequence, 1);
        assert_eq!(db.list_events(&cal_id).unwrap().len(), 1);
    }

    #[test]
    fn treats_ymd_as_all_day() {
        let (db, cal_id) = tmp_db();
        let ev = db
            .upsert_event(
                &cal_id,
                &EventInput {
                    summary: "Off".into(),
                    start: "2026-08-20".into(),
                    ..EventInput::default()
                },
            )
            .unwrap();
        assert!(ev.all_day);
        assert_eq!(ev.dtend, "2026-08-21");
    }

    #[test]
    fn rejects_end_before_start() {
        let (db, cal_id) = tmp_db();
        let ev = db.upsert_event(
            &cal_id,
            &EventInput {
                summary: "Bad".into(),
                start: "2026-08-16T19:00:00Z".into(),
                end: Some("2026-08-16T18:00:00Z".into()),
                ..EventInput::default()
            },
        );
        assert!(ev.is_err());
    }

    #[test]
    fn patches_one_field_and_deletes() {
        let (db, cal_id) = tmp_db();
        db.upsert_event(
            &cal_id,
            &EventInput {
                uid: Some("n1".into()),
                summary: "Note".into(),
                start: "2026-08-16T12:00:00Z".into(),
                ..EventInput::default()
            },
        )
        .unwrap();
        let patched = db
            .patch_event(
                &cal_id,
                "n1",
                &EventPatch {
                    location: Some("Oracle Park".into()),
                    ..EventPatch::default()
                },
            )
            .unwrap();
        assert_eq!(patched.location, "Oracle Park");
        assert_eq!(patched.sequence, 1);
        assert!(db.delete_event(&cal_id, "n1").unwrap());
        assert!(db.get_event(&cal_id, "n1").unwrap().is_none());
    }
}
