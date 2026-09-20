use std::path::Path;

use crate::ics::next_date;
use crate::json::Value;
use crate::rrule::canonicalize;
use crate::sqlite::{Bind, Connection};
use crate::time::{add_hour, is_ymd, now_iso, parse_instant};
use crate::tz;
use crate::util::{event_uid, hex_encode, random_bytes, secret};

const EVENT_COLS: &str = "uid, summary, description, location, dtstart, dtend, \
     all_day, transparent, sequence, created_at, updated_at, rrule, tzid";

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
    pub rrule: String,
    pub tzid: String,
    pub exdates: Vec<String>,
    pub overrides: Vec<EventOverride>,
}

#[derive(Debug, Clone)]
pub struct EventOverride {
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
    pub rrule: Option<String>,
    pub time_zone: Option<String>,
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
    pub rrule: Option<String>,
    pub time_zone: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct OverrideInput {
    pub recurrence_id: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start: String,
    pub end: Option<String>,
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
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS calendars (
                id TEXT PRIMARY KEY,
                feed_token TEXT NOT NULL UNIQUE,
                agent_key TEXT NOT NULL,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL
            );",
        )?;
        migrate_events(&conn)?;
        Ok(Self { conn })
    }

    pub fn get_calendar(&self, id: &str) -> Result<Option<CalendarRow>, String> {
        self.conn.query_row(
            "SELECT id, feed_token, agent_key, name, created_at FROM calendars WHERE id = ?1",
            &[Bind::Text(id)],
            row_calendar,
        )
    }

    pub fn get_calendar_by_feed_token(&self, token: &str) -> Result<Option<CalendarRow>, String> {
        self.conn.query_row(
            "SELECT id, feed_token, agent_key, name, created_at FROM calendars WHERE feed_token = ?1",
            &[Bind::Text(token)],
            row_calendar,
        )
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
        self.conn.execute(
            "INSERT INTO calendars (id, feed_token, agent_key, name, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            &[
                Bind::Text(id),
                Bind::Text(&feed),
                Bind::Text(&key),
                Bind::Text(name),
                Bind::Text(&now),
            ],
        )?;
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
        let mut rows = self.conn.query(
            &format!("SELECT {EVENT_COLS} FROM events WHERE calendar_id = ?1 ORDER BY dtstart ASC"),
            &[Bind::Text(calendar_id)],
            row_event,
        )?;
        let exceptions = self.list_exceptions(calendar_id, None)?;
        for row in &mut rows {
            attach_exceptions(row, &exceptions);
        }
        Ok(rows)
    }

    pub fn get_event(&self, calendar_id: &str, uid: &str) -> Result<Option<EventRow>, String> {
        let mut row = self.conn.query_row(
            &format!("SELECT {EVENT_COLS} FROM events WHERE calendar_id = ?1 AND uid = ?2"),
            &[Bind::Text(calendar_id), Bind::Text(uid)],
            row_event,
        )?;
        if let Some(row) = row.as_mut() {
            let exceptions = self.list_exceptions(calendar_id, Some(uid))?;
            attach_exceptions(row, &exceptions);
        }
        Ok(row)
    }

    pub fn upsert_event(&self, calendar_id: &str, input: &EventInput) -> Result<EventRow, String> {
        let all_day = input.all_day == Some(true) || is_ymd(&input.start);
        let now = now_iso();
        let generated = event_uid();
        let uid = input.uid.as_deref().unwrap_or(&generated);
        if !is_uid(uid) {
            return Err("uid must be 1-200 chars: letters, digits, . _ @ -".into());
        }
        let existing = self.get_event(calendar_id, uid)?;
        let rrule_raw = input.rrule.as_deref().unwrap_or("");
        let tz_explicit = input.time_zone.is_some();
        let tz_raw = input.time_zone.as_deref().unwrap_or("");
        let had_exceptions = existing
            .as_ref()
            .is_some_and(|e| !e.exdates.is_empty() || !e.overrides.is_empty());
        let all_day_changed = existing.as_ref().is_some_and(|e| e.all_day != all_day);
        let (rrule, tzid) = finalize_recurrence(
            all_day,
            rrule_raw,
            tz_raw,
            tz_explicit,
            had_exceptions,
            all_day_changed,
        )?;
        let leaving_series =
            existing.as_ref().is_some_and(|e| !e.rrule.is_empty()) && rrule.is_empty();
        let previous_tz = existing.as_ref().map(|e| e.tzid.as_str()).unwrap_or("");
        let times = if !rrule.is_empty() {
            resolve_for_event(&input.start, input.end.as_deref(), all_day, true)?
        } else if leaving_series {
            resolve_after_series(&input.start, input.end.as_deref(), all_day, previous_tz)?
        } else {
            resolve_times(&input.start, input.end.as_deref(), all_day)?
        };
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
            self.conn.execute(
                "UPDATE events
                 SET summary = ?1, description = ?2, location = ?3, dtstart = ?4, dtend = ?5,
                     all_day = ?6, transparent = ?7, sequence = sequence + 1, updated_at = ?8,
                     rrule = ?9, tzid = ?10
                 WHERE calendar_id = ?11 AND uid = ?12",
                &[
                    Bind::Text(&input.summary),
                    Bind::Text(&description),
                    Bind::Text(&location),
                    Bind::Text(&times.dtstart),
                    Bind::Text(&times.dtend),
                    Bind::I64(i64::from(times.all_day)),
                    Bind::I64(i64::from(transparent)),
                    Bind::Text(&now),
                    Bind::Text(&rrule),
                    Bind::Text(&tzid),
                    Bind::Text(calendar_id),
                    Bind::Text(uid),
                ],
            )?;
        } else {
            self.conn.execute(
                "INSERT INTO events (
                    calendar_id, uid, summary, description, location, dtstart, dtend,
                    all_day, transparent, sequence, created_at, updated_at, rrule, tzid
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?11, ?12, ?13)",
                &[
                    Bind::Text(calendar_id),
                    Bind::Text(uid),
                    Bind::Text(&input.summary),
                    Bind::Text(&description),
                    Bind::Text(&location),
                    Bind::Text(&times.dtstart),
                    Bind::Text(&times.dtend),
                    Bind::I64(i64::from(times.all_day)),
                    Bind::I64(i64::from(transparent)),
                    Bind::Text(&now),
                    Bind::Text(&now),
                    Bind::Text(&rrule),
                    Bind::Text(&tzid),
                ],
            )?;
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
        let rrule_raw = patch.rrule.as_deref().unwrap_or(&existing.rrule);
        let tz_explicit = patch.time_zone.is_some();
        let tz_raw = patch.time_zone.as_deref().unwrap_or(&existing.tzid);
        let had_exceptions = !existing.exdates.is_empty() || !existing.overrides.is_empty();
        let all_day_changed = existing.all_day != all_day;
        let (rrule, tzid) = finalize_recurrence(
            all_day,
            rrule_raw,
            tz_raw,
            tz_explicit,
            had_exceptions,
            all_day_changed,
        )?;
        let leaving_series = !existing.rrule.is_empty() && rrule.is_empty();
        let times = if !rrule.is_empty() {
            resolve_for_event(start, Some(end), all_day, true)?
        } else if leaving_series {
            resolve_after_series(start, Some(end), all_day, &existing.tzid)?
        } else {
            resolve_times(start, Some(end), all_day)?
        };
        let now = now_iso();
        let summary = patch.summary.as_deref().unwrap_or(&existing.summary);
        let description = patch
            .description
            .as_deref()
            .unwrap_or(&existing.description);
        let location = patch.location.as_deref().unwrap_or(&existing.location);
        let transparent = patch.transparent.unwrap_or(existing.transparent);
        self.conn.execute(
            "UPDATE events
             SET summary = ?1, description = ?2, location = ?3, dtstart = ?4, dtend = ?5,
                 all_day = ?6, transparent = ?7, sequence = sequence + 1, updated_at = ?8,
                 rrule = ?9, tzid = ?10
             WHERE calendar_id = ?11 AND uid = ?12",
            &[
                Bind::Text(summary),
                Bind::Text(description),
                Bind::Text(location),
                Bind::Text(&times.dtstart),
                Bind::Text(&times.dtend),
                Bind::I64(i64::from(times.all_day)),
                Bind::I64(i64::from(transparent)),
                Bind::Text(&now),
                Bind::Text(&rrule),
                Bind::Text(&tzid),
                Bind::Text(calendar_id),
                Bind::Text(uid),
            ],
        )?;
        self.get_event(calendar_id, uid)?
            .ok_or_else(|| "failed to save event".into())
    }

    pub fn delete_event(&self, calendar_id: &str, uid: &str) -> Result<bool, String> {
        if self.get_event(calendar_id, uid)?.is_none() {
            return Ok(false);
        }
        self.conn.execute(
            "DELETE FROM event_exceptions WHERE calendar_id = ?1 AND uid = ?2",
            &[Bind::Text(calendar_id), Bind::Text(uid)],
        )?;
        let n = self.conn.execute(
            "DELETE FROM events WHERE calendar_id = ?1 AND uid = ?2",
            &[Bind::Text(calendar_id), Bind::Text(uid)],
        )?;
        Ok(n > 0)
    }

    pub fn put_exdate(
        &self,
        calendar_id: &str,
        uid: &str,
        recurrence_id: &str,
    ) -> Result<(EventRow, bool), String> {
        let event = self.require_series(calendar_id, uid)?;
        let recurrence_id = normalize_recurrence_id(recurrence_id, event.all_day)?;
        let prior = self.find_exception(calendar_id, uid, &recurrence_id)?;
        let created = !matches!(prior, Some(row) if row.excluded);
        let now = now_iso();
        self.conn.execute(
            "DELETE FROM event_exceptions
             WHERE calendar_id = ?1 AND uid = ?2 AND recurrence_id = ?3",
            &[
                Bind::Text(calendar_id),
                Bind::Text(uid),
                Bind::Text(&recurrence_id),
            ],
        )?;
        self.conn.execute(
            "INSERT INTO event_exceptions (
                calendar_id, uid, recurrence_id, excluded, summary, description, location,
                dtstart, dtend, all_day, transparent, sequence, created_at, updated_at
             ) VALUES (?1, ?2, ?3, 1, '', '', '', '', '', ?4, 1, 0, ?5, ?6)",
            &[
                Bind::Text(calendar_id),
                Bind::Text(uid),
                Bind::Text(&recurrence_id),
                Bind::I64(i64::from(event.all_day)),
                Bind::Text(&now),
                Bind::Text(&now),
            ],
        )?;
        self.bump_master(calendar_id, uid, &now)?;
        let saved = self
            .get_event(calendar_id, uid)?
            .ok_or_else(|| "failed to save event".to_string())?;
        Ok((saved, created))
    }

    pub fn delete_exdate(
        &self,
        calendar_id: &str,
        uid: &str,
        recurrence_id: &str,
    ) -> Result<bool, String> {
        let event = self.require_series(calendar_id, uid)?;
        let recurrence_id = normalize_recurrence_id(recurrence_id, event.all_day)?;
        let n = self.conn.execute(
            "DELETE FROM event_exceptions
             WHERE calendar_id = ?1 AND uid = ?2 AND recurrence_id = ?3 AND excluded = 1",
            &[
                Bind::Text(calendar_id),
                Bind::Text(uid),
                Bind::Text(&recurrence_id),
            ],
        )?;
        if n == 0 {
            return Ok(false);
        }
        self.bump_master(calendar_id, uid, &now_iso())?;
        Ok(true)
    }

    pub fn put_override(
        &self,
        calendar_id: &str,
        uid: &str,
        input: &OverrideInput,
    ) -> Result<(EventRow, bool), String> {
        let event = self.require_series(calendar_id, uid)?;
        let recurrence_id = normalize_recurrence_id(&input.recurrence_id, event.all_day)?;
        let times = resolve_for_event(&input.start, input.end.as_deref(), event.all_day, true)?;
        if let Some(summary) = input.summary.as_deref() {
            if summary.trim().is_empty() {
                return Err("summary cannot be empty".into());
            }
            if summary.len() > 512 {
                return Err("summary is too long".into());
            }
        }
        if let Some(d) = input.description.as_deref() {
            if d.len() > 4000 {
                return Err("description is too long".into());
            }
        }
        if let Some(l) = input.location.as_deref() {
            if l.len() > 512 {
                return Err("location is too long".into());
            }
        }
        let prior = self.find_exception(calendar_id, uid, &recurrence_id)?;
        let created = !matches!(prior, Some(ref row) if !row.excluded);
        let sequence = match &prior {
            Some(row) if !row.excluded => row.sequence + 1,
            _ => 0,
        };
        let summary = input
            .summary
            .clone()
            .unwrap_or_else(|| event.summary.clone());
        let description = input
            .description
            .clone()
            .unwrap_or_else(|| event.description.clone());
        let location = input
            .location
            .clone()
            .unwrap_or_else(|| event.location.clone());
        let transparent = input.transparent.unwrap_or(event.transparent);
        let now = now_iso();
        self.conn.execute(
            "DELETE FROM event_exceptions
             WHERE calendar_id = ?1 AND uid = ?2 AND recurrence_id = ?3",
            &[
                Bind::Text(calendar_id),
                Bind::Text(uid),
                Bind::Text(&recurrence_id),
            ],
        )?;
        self.conn.execute(
            "INSERT INTO event_exceptions (
                calendar_id, uid, recurrence_id, excluded, summary, description, location,
                dtstart, dtend, all_day, transparent, sequence, created_at, updated_at
             ) VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            &[
                Bind::Text(calendar_id),
                Bind::Text(uid),
                Bind::Text(&recurrence_id),
                Bind::Text(&summary),
                Bind::Text(&description),
                Bind::Text(&location),
                Bind::Text(&times.dtstart),
                Bind::Text(&times.dtend),
                Bind::I64(i64::from(times.all_day)),
                Bind::I64(i64::from(transparent)),
                Bind::I64(sequence),
                Bind::Text(&now),
                Bind::Text(&now),
            ],
        )?;
        self.bump_master(calendar_id, uid, &now)?;
        let saved = self
            .get_event(calendar_id, uid)?
            .ok_or_else(|| "failed to save event".to_string())?;
        Ok((saved, created))
    }

    pub fn delete_override(
        &self,
        calendar_id: &str,
        uid: &str,
        recurrence_id: &str,
    ) -> Result<bool, String> {
        let event = self.require_series(calendar_id, uid)?;
        let recurrence_id = normalize_recurrence_id(recurrence_id, event.all_day)?;
        let n = self.conn.execute(
            "DELETE FROM event_exceptions
             WHERE calendar_id = ?1 AND uid = ?2 AND recurrence_id = ?3 AND excluded = 0",
            &[
                Bind::Text(calendar_id),
                Bind::Text(uid),
                Bind::Text(&recurrence_id),
            ],
        )?;
        if n == 0 {
            return Ok(false);
        }
        self.bump_master(calendar_id, uid, &now_iso())?;
        Ok(true)
    }

    fn require_series(&self, calendar_id: &str, uid: &str) -> Result<EventRow, String> {
        let event = self
            .get_event(calendar_id, uid)?
            .ok_or_else(|| "not found".to_string())?;
        if event.rrule.is_empty() {
            return Err("event is not recurring".into());
        }
        Ok(event)
    }

    fn bump_master(&self, calendar_id: &str, uid: &str, now: &str) -> Result<(), String> {
        self.conn.execute(
            "UPDATE events SET sequence = sequence + 1, updated_at = ?1
             WHERE calendar_id = ?2 AND uid = ?3",
            &[Bind::Text(now), Bind::Text(calendar_id), Bind::Text(uid)],
        )?;
        Ok(())
    }

    fn find_exception(
        &self,
        calendar_id: &str,
        uid: &str,
        recurrence_id: &str,
    ) -> Result<Option<ExceptionRow>, String> {
        Ok(self
            .list_exceptions(calendar_id, Some(uid))?
            .into_iter()
            .find(|row| row.recurrence_id == recurrence_id))
    }

    fn list_exceptions(
        &self,
        calendar_id: &str,
        uid: Option<&str>,
    ) -> Result<Vec<ExceptionRow>, String> {
        if let Some(uid) = uid {
            self.conn.query(
                "SELECT uid, recurrence_id, excluded, summary, description, location,
                        dtstart, dtend, all_day, transparent, sequence, updated_at
                 FROM event_exceptions
                 WHERE calendar_id = ?1 AND uid = ?2
                 ORDER BY recurrence_id ASC",
                &[Bind::Text(calendar_id), Bind::Text(uid)],
                row_exception,
            )
        } else {
            self.conn.query(
                "SELECT uid, recurrence_id, excluded, summary, description, location,
                        dtstart, dtend, all_day, transparent, sequence, updated_at
                 FROM event_exceptions
                 WHERE calendar_id = ?1
                 ORDER BY recurrence_id ASC",
                &[Bind::Text(calendar_id)],
                row_exception,
            )
        }
    }
}

fn row_calendar(row: &crate::sqlite::Row) -> CalendarRow {
    CalendarRow {
        id: row.text(0),
        feed_token: row.text(1),
        agent_key: row.text(2),
        name: row.text(3),
        created_at: row.text(4),
    }
}

fn row_event(row: &crate::sqlite::Row) -> EventRow {
    EventRow {
        uid: row.text(0),
        summary: row.text(1),
        description: row.text(2),
        location: row.text(3),
        dtstart: row.text(4),
        dtend: row.text(5),
        all_day: row.i64(6) == 1,
        transparent: row.i64(7) == 1,
        sequence: row.i64(8),
        created_at: row.text(9),
        updated_at: row.text(10),
        rrule: row.text(11),
        tzid: row.text(12),
        exdates: Vec::new(),
        overrides: Vec::new(),
    }
}

struct ExceptionRow {
    uid: String,
    recurrence_id: String,
    excluded: bool,
    summary: String,
    description: String,
    location: String,
    dtstart: String,
    dtend: String,
    all_day: bool,
    transparent: bool,
    sequence: i64,
    updated_at: String,
}

fn row_exception(row: &crate::sqlite::Row) -> ExceptionRow {
    ExceptionRow {
        uid: row.text(0),
        recurrence_id: row.text(1),
        excluded: row.i64(2) == 1,
        summary: row.text(3),
        description: row.text(4),
        location: row.text(5),
        dtstart: row.text(6),
        dtend: row.text(7),
        all_day: row.i64(8) == 1,
        transparent: row.i64(9) == 1,
        sequence: row.i64(10),
        updated_at: row.text(11),
    }
}

fn attach_exceptions(event: &mut EventRow, rows: &[ExceptionRow]) {
    for row in rows.iter().filter(|row| row.uid == event.uid) {
        if row.excluded {
            event.exdates.push(row.recurrence_id.clone());
        } else {
            event.overrides.push(EventOverride {
                recurrence_id: row.recurrence_id.clone(),
                summary: row.summary.clone(),
                description: row.description.clone(),
                location: row.location.clone(),
                dtstart: row.dtstart.clone(),
                dtend: row.dtend.clone(),
                all_day: row.all_day,
                transparent: row.transparent,
                sequence: row.sequence,
                updated_at: row.updated_at.clone(),
            });
        }
    }
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
                rrule TEXT NOT NULL DEFAULT '',
                tzid TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (calendar_id, uid)
            );
            CREATE INDEX IF NOT EXISTS idx_events_cal_start ON events(calendar_id, dtstart);",
        )?;
    } else {
        let cols = column_names(db, "events")?;
        if !cols.iter().any(|c| c == "calendar_id") {
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
                    rrule TEXT NOT NULL DEFAULT '',
                    tzid TEXT NOT NULL DEFAULT '',
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
            )?;
        }
        let cols = column_names(db, "events")?;
        if !cols.iter().any(|c| c == "rrule") {
            db.execute_batch("ALTER TABLE events ADD COLUMN rrule TEXT NOT NULL DEFAULT '';")?;
        }
        if !cols.iter().any(|c| c == "tzid") {
            db.execute_batch("ALTER TABLE events ADD COLUMN tzid TEXT NOT NULL DEFAULT '';")?;
        }
    }
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS event_exceptions (
            calendar_id TEXT NOT NULL,
            uid TEXT NOT NULL,
            recurrence_id TEXT NOT NULL,
            excluded INTEGER NOT NULL DEFAULT 0,
            summary TEXT NOT NULL DEFAULT '',
            description TEXT NOT NULL DEFAULT '',
            location TEXT NOT NULL DEFAULT '',
            dtstart TEXT NOT NULL DEFAULT '',
            dtend TEXT NOT NULL DEFAULT '',
            all_day INTEGER NOT NULL DEFAULT 0,
            transparent INTEGER NOT NULL DEFAULT 1,
            sequence INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (calendar_id, uid, recurrence_id)
        );",
    )
}

fn table_names(db: &Connection) -> Result<Vec<String>, String> {
    db.query(
        "SELECT name FROM sqlite_master WHERE type = 'table'",
        &[],
        |row| row.text(0),
    )
}

fn column_names(db: &Connection, table: &str) -> Result<Vec<String>, String> {
    db.query(&format!("PRAGMA table_info({table})"), &[], |row| {
        row.text(1)
    })
}

fn is_uid(value: &str) -> bool {
    value.len() <= 200
        && !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'@' | b'-'))
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

fn resolve_for_event(
    start: &str,
    end: Option<&str>,
    all_day: bool,
    recurring: bool,
) -> Result<Times, String> {
    if !recurring {
        return resolve_times(start, end, all_day);
    }
    if all_day {
        return resolve_times(start, end, true);
    }
    let dtstart =
        parse_wall(start).map_err(|_| "start must be an ISO-8601 datetime".to_string())?;
    let dtend = match end {
        Some(value) => {
            parse_wall(value).map_err(|_| "end must be an ISO-8601 datetime".to_string())?
        }
        None => add_wall_hour(&dtstart)?,
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

fn resolve_after_series(
    start: &str,
    end: Option<&str>,
    all_day: bool,
    previous_tz: &str,
) -> Result<Times, String> {
    if all_day || is_ymd(start) {
        return resolve_times(start, end, true);
    }
    let as_absolute = |value: &str| -> Result<String, String> {
        if parse_instant(value).is_some() {
            return Ok(value.to_string());
        }
        if previous_tz == "UTC" && value.len() == 19 && parse_wall(value).is_ok() {
            return Ok(format!("{}Z", parse_wall(value)?));
        }
        Err("start and end must include an offset when removing rrule".into())
    };
    let start = as_absolute(start)?;
    let end = match end {
        Some(value) => Some(as_absolute(value)?),
        None => None,
    };
    resolve_times(&start, end.as_deref(), false)
}

fn finalize_recurrence(
    all_day: bool,
    rrule_raw: &str,
    tz_raw: &str,
    tz_explicit: bool,
    had_exceptions: bool,
    all_day_changed: bool,
) -> Result<(String, String), String> {
    if had_exceptions && (all_day_changed || rrule_raw.is_empty()) {
        return Err("clear exceptions first".into());
    }
    if rrule_raw.is_empty() {
        if tz_explicit && !tz_raw.is_empty() {
            return Err("timeZone is only allowed on a recurring event".into());
        }
        return Ok((String::new(), String::new()));
    }
    let rrule = canonicalize(rrule_raw, all_day)?;
    if all_day {
        if tz_explicit && !tz_raw.is_empty() {
            return Err("timeZone is not allowed on an all-day series".into());
        }
        return Ok((rrule, String::new()));
    }
    if tz_raw.is_empty() {
        return Err("timeZone is required".into());
    }
    tz::check(tz_raw)?;
    Ok((rrule, tz_raw.to_string()))
}

fn parse_wall(value: &str) -> Result<String, String> {
    let err = "start must be an ISO-8601 datetime";
    let bytes = value.as_bytes();
    if bytes.len() < 19 || !is_ymd(&value[..10]) || bytes[10] != b'T' {
        return Err(err.into());
    }
    let hh: u32 = value[11..13].parse().map_err(|_| err.to_string())?;
    let mm: u32 = value[14..16].parse().map_err(|_| err.to_string())?;
    let ss: u32 = value[17..19].parse().map_err(|_| err.to_string())?;
    if bytes[13] != b':' || bytes[16] != b':' || hh > 23 || mm > 59 || ss > 59 {
        return Err(err.into());
    }
    let mut i = 19;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return Err(err.into());
        }
    }
    if i < bytes.len() && parse_instant(value).is_none() {
        return Err(err.into());
    }
    Ok(format!("{}T{hh:02}:{mm:02}:{ss:02}", &value[..10]))
}

fn add_wall_hour(wall: &str) -> Result<String, String> {
    let (date, time) = wall
        .split_once('T')
        .ok_or_else(|| "start must be an ISO-8601 datetime".to_string())?;
    let hh: u32 = time[0..2]
        .parse()
        .map_err(|_| "start must be an ISO-8601 datetime".to_string())?;
    let mm: u32 = time[3..5]
        .parse()
        .map_err(|_| "start must be an ISO-8601 datetime".to_string())?;
    let ss: u32 = time[6..8]
        .parse()
        .map_err(|_| "start must be an ISO-8601 datetime".to_string())?;
    let total = hh * 3600 + mm * 60 + ss + 3600;
    let extra_days = total / 86_400;
    let rem = total % 86_400;
    let mut day = date.to_string();
    for _ in 0..extra_days {
        day = next_date(&day)?;
    }
    Ok(format!(
        "{day}T{:02}:{:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    ))
}

pub fn normalize_recurrence_id(value: &str, all_day: bool) -> Result<String, String> {
    let value = value.trim();
    if all_day {
        if is_ymd(value) {
            return Ok(value.to_string());
        }
        return Err("recurrenceId must be YYYY-MM-DD".into());
    }
    if value.len() == 19
        && !value.contains('Z')
        && !value.contains('+')
        && !value[10..].contains('-')
    {
        return parse_wall(value)
            .map_err(|_| "recurrenceId must be YYYY-MM-DDTHH:MM:SS".to_string());
    }
    Err("recurrenceId must be YYYY-MM-DDTHH:MM:SS".into())
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
        rrule: optional_string(obj, "rrule")?,
        time_zone: optional_string(obj, "timeZone")?,
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
    if obj.contains_key("rrule") {
        patch.rrule = optional_string(obj, "rrule")?;
    }
    if obj.contains_key("timeZone") {
        patch.time_zone = optional_string(obj, "timeZone")?;
    }
    if patch.summary.is_none()
        && patch.description.is_none()
        && patch.location.is_none()
        && patch.start.is_none()
        && patch.end.is_none()
        && patch.all_day.is_none()
        && patch.transparent.is_none()
        && patch.rrule.is_none()
        && patch.time_zone.is_none()
    {
        return Err("no fields to update".into());
    }
    Ok(patch)
}

fn optional_string(
    obj: &std::collections::BTreeMap<String, Value>,
    key: &str,
) -> Result<Option<String>, String> {
    match obj.get(key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.trim().to_string())),
        Some(_) => Err(format!("{key} must be a string")),
    }
}

pub fn parse_recurrence_id(body: &Value) -> Result<String, String> {
    let obj = body.as_object().ok_or("body must be an object")?;
    obj.get("recurrenceId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "recurrenceId is required".to_string())
}

pub fn parse_override_input(body: &Value) -> Result<OverrideInput, String> {
    let obj = body.as_object().ok_or("body must be an object")?;
    let recurrence_id = parse_recurrence_id(body)?;
    let start = obj
        .get("start")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("start is required")?;
    let summary = match obj.get("summary") {
        None => None,
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err("summary cannot be empty".into());
            }
            if trimmed.len() > 512 {
                return Err("summary is too long".into());
            }
            Some(trimmed.to_string())
        }
        Some(_) => return Err("summary must be a string".into()),
    };
    let description = match obj.get("description") {
        None => None,
        Some(Value::String(s)) => {
            if s.len() > 4000 {
                return Err("description is too long".into());
            }
            Some(s.to_string())
        }
        Some(_) => return Err("description must be a string".into()),
    };
    let location = match obj.get("location") {
        None => None,
        Some(Value::String(s)) => {
            if s.len() > 512 {
                return Err("location is too long".into());
            }
            Some(s.to_string())
        }
        Some(_) => return Err("location must be a string".into()),
    };
    Ok(OverrideInput {
        recurrence_id,
        summary,
        description,
        location,
        start: start.to_string(),
        end: obj.get("end").and_then(Value::as_str).map(str::to_string),
        transparent: obj.get("transparent").and_then(Value::as_bool),
    })
}

pub fn json_calendar(row: &CalendarRow, base: &str) -> Value {
    let origin = base.trim_end_matches('/');
    Value::object(&[
        ("id", Value::String(row.id.clone())),
        ("name", Value::String(row.name.clone())),
        (
            "subscribe",
            Value::String(format!("{origin}/feed/{}.ics", row.feed_token)),
        ),
        (
            "write",
            Value::String(format!("{origin}/v1/c/{}/events", row.id)),
        ),
        ("key", Value::String(row.agent_key.clone())),
        ("createdAt", Value::String(row.created_at.clone())),
    ])
}

pub fn json_event(row: &EventRow) -> Value {
    let mut pairs = vec![
        ("uid", Value::String(row.uid.clone())),
        ("summary", Value::String(row.summary.clone())),
        ("description", Value::String(row.description.clone())),
        ("location", Value::String(row.location.clone())),
        ("start", Value::String(row.dtstart.clone())),
        ("end", Value::String(row.dtend.clone())),
        ("allDay", Value::Bool(row.all_day)),
        ("transparent", Value::Bool(row.transparent)),
        ("sequence", Value::Number(row.sequence)),
        ("createdAt", Value::String(row.created_at.clone())),
        ("updatedAt", Value::String(row.updated_at.clone())),
    ];
    if !row.rrule.is_empty() {
        pairs.push(("rrule", Value::String(row.rrule.clone())));
    }
    if !row.tzid.is_empty() {
        pairs.push(("timeZone", Value::String(row.tzid.clone())));
    }
    if !row.exdates.is_empty() {
        pairs.push((
            "exdates",
            Value::Array(row.exdates.iter().cloned().map(Value::String).collect()),
        ));
    }
    if !row.overrides.is_empty() {
        let overrides = row
            .overrides
            .iter()
            .map(|over| {
                Value::object(&[
                    ("recurrenceId", Value::String(over.recurrence_id.clone())),
                    ("summary", Value::String(over.summary.clone())),
                    ("description", Value::String(over.description.clone())),
                    ("location", Value::String(over.location.clone())),
                    ("start", Value::String(over.dtstart.clone())),
                    ("end", Value::String(over.dtend.clone())),
                    ("transparent", Value::Bool(over.transparent)),
                    ("sequence", Value::Number(over.sequence)),
                ])
            })
            .collect();
        pairs.push(("overrides", Value::Array(overrides)));
    }
    Value::object(&pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::parse;
    use crate::util::hex_encode;

    fn tmp_db() -> (Db, String) {
        let dir = std::env::temp_dir().join(format!("almanac-db-{}", hex_encode(&random_bytes(8))));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Db::open(dir.join("calendar.db")).unwrap();
        let cal = db.create_calendar(Some("Test"), None, None, None).unwrap();
        (db, cal.id)
    }

    #[test]
    fn requires_summary_and_start() {
        let miss = parse_event_input(&parse(r#"{"summary":"x"}"#).unwrap());
        assert!(miss.is_err());
        let ok = parse_event_input(
            &parse(r#"{"summary":"Dinner","start":"2026-08-16T19:00:00-07:00"}"#).unwrap(),
        )
        .unwrap();
        assert_eq!(ok.summary, "Dinner");
    }

    #[test]
    fn rejects_a_bad_uid() {
        let bad = parse_event_input(
            &parse(r#"{"uid":"has spaces","summary":"x","start":"2026-08-16T19:00:00Z"}"#).unwrap(),
        );
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

    #[test]
    fn stores_a_series_as_wall_clock_and_blocks_clearing_exceptions() {
        let (db, cal_id) = tmp_db();
        let ev = db
            .upsert_event(
                &cal_id,
                &EventInput {
                    uid: Some("standup".into()),
                    summary: "Standup".into(),
                    start: "2026-09-22T09:00:00-07:00".into(),
                    end: Some("2026-09-22T09:15:00-07:00".into()),
                    rrule: Some("freq=weekly;byday=tu".into()),
                    time_zone: Some("America/Los_Angeles".into()),
                    ..EventInput::default()
                },
            )
            .unwrap();
        assert_eq!(ev.dtstart, "2026-09-22T09:00:00");
        assert_eq!(ev.rrule, "FREQ=WEEKLY;BYDAY=TU");
        db.put_exdate(&cal_id, "standup", "2026-10-06T09:00:00")
            .unwrap();
        let cleared = db.patch_event(
            &cal_id,
            "standup",
            &EventPatch {
                rrule: Some(String::new()),
                ..EventPatch::default()
            },
        );
        assert!(cleared.is_err());
    }
}
