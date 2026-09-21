//! Notes: storage, input parsing, JSON, and the HTTP routes.
//!
//! Same shape as todos: rows in the shared database keyed by calendar id, the
//! calendar's one write key, a feed under its own token. The feed is Atom (see
//! `atom.rs`) because a note has no place in an ICS calendar that apps honour.

use crate::db::{is_uid, CalendarRow, Db};
use crate::http::{json_response, Request, Response};
use crate::json::{stringify, Value};
use crate::server::{gate, is_home, json_err, lock_db, parse_body, query_param, AppState};
use crate::sqlite::{Bind, Connection, Row};
use crate::time::now_iso;
use crate::todos::{pack_tags, parse_tags, unpack_tags};
use crate::util::prefixed_uid;

const NOTE_COLS: &str = "uid, title, body, tags, pinned, sequence, created_at, updated_at, seal";

const MAX_TITLE: usize = 200;

/// Markdown text. Under the 64KB request cap so a note always fits in one PUT.
pub const MAX_BODY: usize = 32 * 1024;

#[derive(Debug, Clone)]
pub struct NoteRow {
    pub uid: String,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub sequence: i64,
    pub created_at: String,
    pub updated_at: String,
    pub seal: String,
}

/// What a write may set. `None` leaves the stored value alone.
#[derive(Debug, Default)]
pub struct NoteFields {
    pub uid: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub tags: Option<Vec<String>>,
    pub pinned: Option<bool>,
}

pub(crate) fn migrate(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS notes (
            calendar_id TEXT NOT NULL,
            uid TEXT NOT NULL,
            title TEXT NOT NULL,
            body TEXT NOT NULL DEFAULT '',
            tags TEXT NOT NULL DEFAULT '',
            pinned INTEGER NOT NULL DEFAULT 0,
            sequence INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (calendar_id, uid)
        );
        CREATE INDEX IF NOT EXISTS idx_notes_cal_updated ON notes(calendar_id, updated_at);",
    )?;
    if !crate::db::column_names(conn, "notes")?.iter().any(|c| c == "seal") {
        conn.execute_batch("ALTER TABLE notes ADD COLUMN seal TEXT NOT NULL DEFAULT '';")?;
    }
    Ok(())
}

fn row_note(row: &Row) -> NoteRow {
    NoteRow {
        uid: row.text(0),
        title: row.text(1),
        body: row.text(2),
        tags: unpack_tags(&row.text(3)),
        pinned: row.i64(4) != 0,
        sequence: row.i64(5),
        created_at: row.text(6),
        updated_at: row.text(7),
        seal: row.text(8),
    }
}

/// `%needle%` for `LIKE ... ESCAPE '\'`, with the wildcards in `needle` made literal.
fn like_pattern(needle: &str) -> String {
    let mut out = String::from("%");
    for c in needle.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

impl Db {
    pub fn count_notes(&self, calendar_id: &str) -> Result<i64, String> {
        Ok(self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM notes WHERE calendar_id = ?1",
                &[Bind::Text(calendar_id)],
                |row| row.i64(0),
            )?
            .unwrap_or(0))
    }

    pub fn get_note(&self, calendar_id: &str, uid: &str) -> Result<Option<NoteRow>, String> {
        self.conn.query_row(
            &format!("SELECT {NOTE_COLS} FROM notes WHERE calendar_id = ?1 AND uid = ?2"),
            &[Bind::Text(calendar_id), Bind::Text(uid)],
            row_note,
        )
    }

    /// Notes whose title or body contains `q` (ASCII case-insensitive; `%` and
    /// `_` are literal). Pinned first, then most recently changed. `limit` of 0
    /// or less means no limit.
    pub fn list_notes(
        &self,
        calendar_id: &str,
        q: Option<&str>,
        limit: i64,
    ) -> Result<Vec<NoteRow>, String> {
        let pattern = q.map(like_pattern);
        let filter = if pattern.is_some() {
            " AND (title LIKE ?2 ESCAPE '\\' OR body LIKE ?2 ESCAPE '\\')"
        } else {
            ""
        };
        let limit = if limit > 0 { format!(" LIMIT {limit}") } else { String::new() };
        let sql = format!(
            "SELECT {NOTE_COLS} FROM notes WHERE calendar_id = ?1{filter}
             ORDER BY pinned DESC, updated_at DESC, uid ASC{limit}"
        );
        match &pattern {
            Some(p) => self
                .conn
                .query(&sql, &[Bind::Text(calendar_id), Bind::Text(p)], row_note),
            None => self.conn.query(&sql, &[Bind::Text(calendar_id)], row_note),
        }
    }

    /// Create or update. Returns the saved row and whether it was created.
    /// `uid` of `None` mints one.
    pub fn upsert_note(
        &self,
        calendar_id: &str,
        uid: Option<&str>,
        fields: &NoteFields,
    ) -> Result<(NoteRow, bool), String> {
        let title = fields.title.clone().ok_or("title is required")?;
        let generated = prefixed_uid("note");
        let uid = uid.unwrap_or(&generated);
        if !is_uid(uid) {
            return Err("uid must be 1-200 chars: letters, digits, . _ @ -".into());
        }
        let tx = self.conn.begin()?;
        let existing = self.get_note(calendar_id, uid)?;
        let now = now_iso();
        let (body, tags, pinned) = match &existing {
            Some(e) => (
                fields.body.clone().unwrap_or_else(|| e.body.clone()),
                fields.tags.clone().unwrap_or_else(|| e.tags.clone()),
                fields.pinned.unwrap_or(e.pinned),
            ),
            None => (
                fields.body.clone().unwrap_or_default(),
                fields.tags.clone().unwrap_or_default(),
                fields.pinned.unwrap_or(false),
            ),
        };
        if existing.is_some() {
            self.conn.execute(
                "UPDATE notes
                 SET title = ?1, body = ?2, tags = ?3, pinned = ?4,
                     sequence = sequence + 1, updated_at = ?5
                 WHERE calendar_id = ?6 AND uid = ?7",
                &[
                    Bind::Text(&title),
                    Bind::Text(&body),
                    Bind::Text(&pack_tags(&tags)),
                    Bind::I64(i64::from(pinned)),
                    Bind::Text(&now),
                    Bind::Text(calendar_id),
                    Bind::Text(uid),
                ],
            )?;
        } else {
            self.conn.execute(
                "INSERT INTO notes (
                    calendar_id, uid, title, body, tags, pinned, sequence, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8)",
                &[
                    Bind::Text(calendar_id),
                    Bind::Text(uid),
                    Bind::Text(&title),
                    Bind::Text(&body),
                    Bind::Text(&pack_tags(&tags)),
                    Bind::I64(i64::from(pinned)),
                    Bind::Text(&now),
                    Bind::Text(&now),
                ],
            )?;
        }
        let saved = self
            .get_note(calendar_id, uid)?
            .ok_or_else(|| "failed to save note".to_string())?;
        tx.commit()?;
        Ok((saved, existing.is_none()))
    }

    pub fn put_note_seal(
        &self,
        calendar_id: &str,
        uid: &str,
        seal: &str,
    ) -> Result<(NoteRow, bool), String> {
        if !is_uid(uid) {
            return Err("uid must be 1-200 chars: letters, digits, . _ @ -".into());
        }
        let tx = self.conn.begin()?;
        let existing = self.get_note(calendar_id, uid)?;
        let now = now_iso();
        let created = existing.is_none();
        if created {
            self.conn.execute(
                "INSERT INTO notes (
                    calendar_id, uid, title, body, tags, pinned, sequence, created_at, updated_at, seal
                 ) VALUES (?1, ?2, '', '', '', 0, 0, ?3, ?3, ?4)",
                &[
                    Bind::Text(calendar_id),
                    Bind::Text(uid),
                    Bind::Text(&now),
                    Bind::Text(seal),
                ],
            )?;
        } else {
            self.conn.execute(
                "UPDATE notes
                 SET title = '', body = '', tags = '', pinned = 0, seal = ?1,
                     sequence = sequence + 1, updated_at = ?2
                 WHERE calendar_id = ?3 AND uid = ?4",
                &[
                    Bind::Text(seal),
                    Bind::Text(&now),
                    Bind::Text(calendar_id),
                    Bind::Text(uid),
                ],
            )?;
        }
        let saved = self
            .get_note(calendar_id, uid)?
            .ok_or_else(|| "failed to save note".to_string())?;
        tx.commit()?;
        Ok((saved, created))
    }

    /// Change only the fields present. `Err("not found")` if there is no such note.
    pub fn patch_note(
        &self,
        calendar_id: &str,
        uid: &str,
        fields: &NoteFields,
    ) -> Result<NoteRow, String> {
        let tx = self.conn.begin()?;
        let existing = self
            .get_note(calendar_id, uid)?
            .ok_or_else(|| "not found".to_string())?;
        let title = fields.title.clone().unwrap_or_else(|| existing.title.clone());
        let body = fields.body.clone().unwrap_or_else(|| existing.body.clone());
        let tags = fields.tags.clone().unwrap_or_else(|| existing.tags.clone());
        let pinned = fields.pinned.unwrap_or(existing.pinned);
        self.conn.execute(
            "UPDATE notes
             SET title = ?1, body = ?2, tags = ?3, pinned = ?4,
                 sequence = sequence + 1, updated_at = ?5
             WHERE calendar_id = ?6 AND uid = ?7",
            &[
                Bind::Text(&title),
                Bind::Text(&body),
                Bind::Text(&pack_tags(&tags)),
                Bind::I64(i64::from(pinned)),
                Bind::Text(&now_iso()),
                Bind::Text(calendar_id),
                Bind::Text(uid),
            ],
        )?;
        let saved = self
            .get_note(calendar_id, uid)?
            .ok_or_else(|| "failed to save note".to_string())?;
        tx.commit()?;
        Ok(saved)
    }

    pub fn delete_note(&self, calendar_id: &str, uid: &str) -> Result<bool, String> {
        let tx = self.conn.begin()?;
        let removed = self.conn.execute(
            "DELETE FROM notes WHERE calendar_id = ?1 AND uid = ?2",
            &[Bind::Text(calendar_id), Bind::Text(uid)],
        )?;
        tx.commit()?;
        Ok(removed > 0)
    }
}

// ---- input --------------------------------------------------------------------

/// Parse a note body. A full body (`partial` false) needs a `title`; a patch
/// needs at least one field. Wrong types are errors, never silently dropped.
pub fn parse_note_fields(body: &Value, partial: bool) -> Result<NoteFields, String> {
    let obj = body.as_object().ok_or("body must be an object")?;
    let mut fields = NoteFields::default();

    match obj.get("title") {
        None if partial => {}
        None => return Err("title is required".into()),
        Some(Value::String(s)) => {
            let title = s.trim();
            if title.is_empty() {
                return Err(if partial { "title cannot be empty" } else { "title is required" }.into());
            }
            if title.len() > MAX_TITLE {
                return Err("title is too long".into());
            }
            fields.title = Some(title.to_string());
        }
        Some(_) => return Err("title must be a string".into()),
    }

    fields.body = match obj.get("body") {
        None => None,
        Some(Value::Null) => Some(String::new()),
        Some(Value::String(s)) if s.len() > MAX_BODY => return Err("body is too long".into()),
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err("body must be a string".into()),
    };

    fields.pinned = match obj.get("pinned") {
        None => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err("pinned must be true or false".into()),
    };

    fields.tags = match obj.get("tags") {
        None => None,
        Some(Value::Null) => Some(Vec::new()),
        Some(Value::Array(items)) => Some(parse_tags(items)?),
        Some(_) => return Err("tags must be a list of strings".into()),
    };

    fields.uid = match obj.get("uid") {
        None => None,
        Some(Value::String(s)) if is_uid(s.trim()) => Some(s.trim().to_string()),
        Some(_) => return Err("uid must be 1-200 chars: letters, digits, . _ @ -".into()),
    };

    if partial
        && fields.title.is_none()
        && fields.body.is_none()
        && fields.pinned.is_none()
        && fields.tags.is_none()
    {
        return Err("no fields to update".into());
    }
    Ok(fields)
}

/// `with_body` false leaves the text out: a list of long notes would otherwise
/// be megabytes.
pub fn json_note(row: &NoteRow, with_body: bool) -> Value {
    if !row.seal.is_empty() {
        return Value::object(&[
            ("uid", Value::String(row.uid.clone())),
            ("seal", Value::String(row.seal.clone())),
            ("createdAt", Value::String(row.created_at.clone())),
            ("updatedAt", Value::String(row.updated_at.clone())),
        ]);
    }
    let mut pairs = vec![
        ("uid", Value::String(row.uid.clone())),
        ("title", Value::String(row.title.clone())),
        ("size", Value::Number(row.body.len() as i64)),
        ("pinned", Value::Bool(row.pinned)),
        (
            "tags",
            Value::Array(row.tags.iter().cloned().map(Value::String).collect()),
        ),
        ("createdAt", Value::String(row.created_at.clone())),
        ("updatedAt", Value::String(row.updated_at.clone())),
    ];
    if with_body {
        pairs.push(("body", Value::String(row.body.clone())));
    }
    Value::object(&pairs)
}

// ---- routes -------------------------------------------------------------------

/// `/v1/c/{id}/notes` (no uid) and `/v1/c/{id}/notes/{uid}`. `/v1/notes` is `home`.
pub(crate) fn route(
    state: &AppState,
    req: &Request,
    cal_id: &str,
    method: &str,
    uid: Option<&str>,
) -> Response {
    let allowed = match uid {
        None => matches!(method, "GET" | "POST"),
        Some(_) => matches!(method, "GET" | "PUT" | "PATCH" | "DELETE"),
    };
    if !allowed {
        return json_err(404, "not found");
    }
    let cal = match gate(state, req, cal_id, method) {
        Ok(cal) => cal,
        Err(res) => return res,
    };
    match (uid, method) {
        (None, "GET") => list(state, req, &cal),
        (None, _) => write(state, req, &cal, None),
        (Some(uid), "GET") => get(state, &cal, uid),
        (Some(uid), "PUT") => write(state, req, &cal, Some(uid)),
        (Some(uid), "PATCH") => patch(state, req, &cal, uid),
        (Some(uid), _) => delete(state, &cal, uid),
    }
}

fn list(state: &AppState, req: &Request, cal: &CalendarRow) -> Response {
    let q = query_param(req, "q").map(|q| q.trim().to_string()).filter(|q| !q.is_empty());
    if q.as_ref().is_some_and(|q| q.len() > 200) {
        return json_err(400, "q is too long");
    }
    let tag = query_param(req, "tag").map(|t| t.trim().to_ascii_lowercase());
    let with_body = matches!(query_param(req, "body").as_deref(), Some("true" | "1"));
    let rows = match lock_db(state).list_notes(&cal.id, q.as_deref(), 0) {
        Ok(rows) => rows,
        Err(e) => return json_err(500, &e),
    };
    let notes: Vec<Value> = rows
        .iter()
        .filter(|row| tag.as_ref().is_none_or(|t| row.tags.contains(t)))
        .map(|row| json_note(row, with_body))
        .collect();
    json_response(200, &stringify(&Value::object(&[("notes", Value::Array(notes))])))
}

fn get(state: &AppState, cal: &CalendarRow, uid: &str) -> Response {
    match lock_db(state).get_note(&cal.id, uid) {
        Ok(Some(row)) => json_response(200, &stringify(&json_note(&row, true))),
        Ok(None) => json_err(404, "not found"),
        Err(e) => json_err(500, &e),
    }
}

/// POST (no uid in the path) and PUT (uid from the path).
fn write(state: &AppState, req: &Request, cal: &CalendarRow, path_uid: Option<&str>) -> Response {
    // Parse before taking the lock: a slow body must not stall other requests.
    let body = match parse_body(req) {
        Ok(v) => v,
        Err(e) => return json_err(400, &e),
    };
    if let Some(wire) = match crate::seal::classify(&body, &cal.feed) {
        Ok(v) => v,
        Err(e) => return json_err(400, &e),
    } {
        let uid = path_uid
            .map(str::to_string)
            .or_else(|| body.get("uid").and_then(Value::as_str).map(str::to_string))
            .filter(|u| !u.is_empty());
        let Some(uid) = uid else {
            return json_err(400, "uid is required");
        };
        let db = lock_db(state);
        if !is_home(cal) {
            let creating = match db.get_note(&cal.id, &uid) {
                Ok(existing) => existing.is_none(),
                Err(e) => return json_err(500, &e),
            };
            if creating {
                match db.count_notes(&cal.id) {
                    Ok(n) if n >= state.limits.max_notes => return json_err(409, "notes are full"),
                    Ok(_) => {}
                    Err(e) => return json_err(500, &e),
                }
            }
        }
        return match db.put_note_seal(&cal.id, &uid, &wire) {
            Ok((row, created)) => json_response(
                if created { 201 } else { 200 },
                &stringify(&json_note(&row, true)),
            ),
            Err(e) => json_err(400, &e),
        };
    }
    let fields = match parse_note_fields(&body, false) {
        Ok(f) => f,
        Err(e) => return json_err(400, &e),
    };
    let uid = path_uid.map(str::to_string).or_else(|| fields.uid.clone());
    let db = lock_db(state);
    if !is_home(cal) {
        let creating = match uid.as_deref() {
            Some(uid) => match db.get_note(&cal.id, uid) {
                Ok(existing) => existing.is_none(),
                Err(e) => return json_err(500, &e),
            },
            None => true,
        };
        if creating {
            match db.count_notes(&cal.id) {
                Ok(n) if n >= state.limits.max_notes => return json_err(409, "notes are full"),
                Ok(_) => {}
                Err(e) => return json_err(500, &e),
            }
        }
    }
    match db.upsert_note(&cal.id, uid.as_deref(), &fields) {
        Ok((row, created)) => json_response(
            if created { 201 } else { 200 },
            &stringify(&json_note(&row, true)),
        ),
        Err(e) => json_err(400, &e),
    }
}

fn patch(state: &AppState, req: &Request, cal: &CalendarRow, uid: &str) -> Response {
    if cal.feed == "seal" {
        return json_err(400, "calendar is sealed");
    }
    let body = match parse_body(req) {
        Ok(v) => v,
        Err(e) => return json_err(400, &e),
    };
    let fields = match parse_note_fields(&body, true) {
        Ok(f) => f,
        Err(e) => return json_err(400, &e),
    };
    match lock_db(state).patch_note(&cal.id, uid, &fields) {
        Ok(row) => json_response(200, &stringify(&json_note(&row, true))),
        Err(e) if e == "not found" => json_err(404, &e),
        Err(e) => json_err(400, &e),
    }
}

fn delete(state: &AppState, cal: &CalendarRow, uid: &str) -> Response {
    match lock_db(state).delete_note(&cal.id, uid) {
        Ok(true) => Response::new(204),
        Ok(false) => json_err(404, "not found"),
        Err(e) => json_err(500, &e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::parse;

    fn fields(json: &str, partial: bool) -> Result<NoteFields, String> {
        parse_note_fields(&parse(json).unwrap(), partial)
    }

    #[test]
    fn a_full_body_needs_a_title() {
        assert!(fields(r#"{"title":"Ideas"}"#, false).is_ok());
        assert_eq!(fields("{}", false).unwrap_err(), "title is required");
        assert!(fields(&format!(r#"{{"title":"{}"}}"#, "x".repeat(201)), false).is_err());
        assert!(fields(&format!(r#"{{"title":"{}"}}"#, "x".repeat(200)), false).is_ok());
    }

    #[test]
    fn the_body_has_a_size_cap() {
        let body = |n: usize| format!(r#"{{"title":"t","body":"{}"}}"#, "x".repeat(n));
        assert!(fields(&body(MAX_BODY), false).is_ok());
        assert_eq!(fields(&body(MAX_BODY + 1), false).unwrap_err(), "body is too long");
    }

    #[test]
    fn wrong_types_are_rejected_not_ignored() {
        for bad in [
            r#"{"title":"x","body":5}"#,
            r#"{"title":"x","pinned":"yes"}"#,
            r#"{"title":"x","pinned":null}"#,
            r#"{"title":"x","tags":"a"}"#,
            r#"{"title":"x","tags":["A B"]}"#,
            r#"{"title":5}"#,
            r#"{"title":"x","uid":"bad uid"}"#,
        ] {
            assert!(fields(bad, false).is_err(), "{bad}");
        }
        assert_eq!(fields("{}", true).unwrap_err(), "no fields to update");
    }

    #[test]
    fn like_wildcards_are_made_literal() {
        assert_eq!(like_pattern("100%"), "%100\\%%");
        assert_eq!(like_pattern("a_b"), "%a\\_b%");
        assert_eq!(like_pattern("c:\\d"), "%c:\\\\d%");
        assert_eq!(like_pattern("it's"), "%it's%");
    }
}
