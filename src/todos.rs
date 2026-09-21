//! Todos: storage, input parsing, JSON, and the HTTP routes.
//!
//! They live in the same database as events, keyed by calendar id, so the one
//! write key for a calendar covers them too. The feed is a `VTODO` calendar
//! behind its own token (see `Db::feed_token`), separate from the event feed.

use crate::db::{is_uid, CalendarRow, Db};
use crate::http::{json_response, Request, Response};
use crate::json::{stringify, Value};
use crate::server::{gate, is_home, json_err, lock_db, parse_body, query_param, AppState};
use crate::sqlite::{Bind, Connection, Row};
use crate::time::{is_ymd, now_iso, parse_instant};
use crate::util::{prefixed_uid, secret};

const TODO_COLS: &str =
    "uid, title, description, due, priority, done, completed_at, tags, sequence, created_at, updated_at, seal";

const MAX_TAGS: usize = 10;

#[derive(Debug, Clone)]
pub struct TodoRow {
    pub uid: String,
    pub title: String,
    pub description: String,
    /// `""` for none, `YYYY-MM-DD` for a day, else a UTC instant.
    pub due: String,
    /// RFC 5545: 0 undefined, 1 highest .. 9 lowest.
    pub priority: i64,
    pub done: bool,
    /// `""` unless done.
    pub completed_at: String,
    pub tags: Vec<String>,
    pub sequence: i64,
    pub created_at: String,
    pub updated_at: String,
    pub seal: String,
}

/// What a write may set. `None` leaves the stored value alone; for text fields
/// `Some("")` clears it.
#[derive(Debug, Default)]
pub struct TodoFields {
    pub uid: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub due: Option<String>,
    pub priority: Option<i64>,
    pub done: Option<bool>,
    pub tags: Option<Vec<String>>,
}

pub(crate) fn migrate(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS todos (
            calendar_id TEXT NOT NULL,
            uid TEXT NOT NULL,
            title TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            due TEXT NOT NULL DEFAULT '',
            priority INTEGER NOT NULL DEFAULT 0,
            done INTEGER NOT NULL DEFAULT 0,
            completed_at TEXT NOT NULL DEFAULT '',
            tags TEXT NOT NULL DEFAULT '',
            sequence INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (calendar_id, uid)
        );
        CREATE INDEX IF NOT EXISTS idx_todos_cal_done_due ON todos(calendar_id, done, due);
        CREATE TABLE IF NOT EXISTS feeds (
            token TEXT PRIMARY KEY,
            calendar_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            UNIQUE (calendar_id, kind)
        );",
    )?;
    if !crate::db::column_names(conn, "todos")?.iter().any(|c| c == "seal") {
        conn.execute_batch("ALTER TABLE todos ADD COLUMN seal TEXT NOT NULL DEFAULT '';")?;
    }
    Ok(())
}

/// Tags are stored as `,a,b,` so every one is delimited on both sides.
pub(crate) fn pack_tags(tags: &[String]) -> String {
    if tags.is_empty() {
        String::new()
    } else {
        format!(",{},", tags.join(","))
    }
}

pub(crate) fn unpack_tags(raw: &str) -> Vec<String> {
    raw.split(',')
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

fn row_todo(row: &Row) -> TodoRow {
    TodoRow {
        uid: row.text(0),
        title: row.text(1),
        description: row.text(2),
        due: row.text(3),
        priority: row.i64(4),
        done: row.i64(5) != 0,
        completed_at: row.text(6),
        tags: unpack_tags(&row.text(7)),
        sequence: row.i64(8),
        created_at: row.text(9),
        updated_at: row.text(10),
        seal: row.text(11),
    }
}

impl Db {
    pub fn count_todos(&self, calendar_id: &str) -> Result<i64, String> {
        Ok(self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM todos WHERE calendar_id = ?1",
                &[Bind::Text(calendar_id)],
                |row| row.i64(0),
            )?
            .unwrap_or(0))
    }

    pub fn get_todo(&self, calendar_id: &str, uid: &str) -> Result<Option<TodoRow>, String> {
        self.conn.query_row(
            &format!("SELECT {TODO_COLS} FROM todos WHERE calendar_id = ?1 AND uid = ?2"),
            &[Bind::Text(calendar_id), Bind::Text(uid)],
            row_todo,
        )
    }

    /// Open items first, then by due date with undated last, then oldest.
    pub fn list_todos(&self, calendar_id: &str, done: Option<bool>) -> Result<Vec<TodoRow>, String> {
        let filter = match done {
            Some(true) => " AND done = 1 AND seal = ''",
            Some(false) => " AND done = 0 AND seal = ''",
            None => "",
        };
        self.conn.query(
            &format!(
                "SELECT {TODO_COLS} FROM todos WHERE calendar_id = ?1{filter}
                 ORDER BY done ASC, (due = '') ASC, due ASC, created_at ASC, uid ASC"
            ),
            &[Bind::Text(calendar_id)],
            row_todo,
        )
    }

    /// Create or update. Returns the saved row and whether it was created.
    /// `uid` of `None` mints one.
    pub fn upsert_todo(
        &self,
        calendar_id: &str,
        uid: Option<&str>,
        fields: &TodoFields,
    ) -> Result<(TodoRow, bool), String> {
        let title = fields.title.clone().ok_or("title is required")?;
        let generated = prefixed_uid("todo");
        let uid = uid.unwrap_or(&generated);
        if !is_uid(uid) {
            return Err("uid must be 1-200 chars: letters, digits, . _ @ -".into());
        }
        let tx = self.conn.begin()?;
        let existing = self.get_todo(calendar_id, uid)?;
        let now = now_iso();
        let (description, due, priority, tags) = match &existing {
            Some(e) => (
                fields.description.clone().unwrap_or_else(|| e.description.clone()),
                fields.due.clone().unwrap_or_else(|| e.due.clone()),
                fields.priority.unwrap_or(e.priority),
                fields.tags.clone().unwrap_or_else(|| e.tags.clone()),
            ),
            None => (
                fields.description.clone().unwrap_or_default(),
                fields.due.clone().unwrap_or_default(),
                fields.priority.unwrap_or(0),
                fields.tags.clone().unwrap_or_default(),
            ),
        };
        let done = fields
            .done
            .or_else(|| existing.as_ref().map(|e| e.done))
            .unwrap_or(false);
        let completed_at = completion(done, existing.as_ref(), &now);
        if existing.is_some() {
            self.conn.execute(
                "UPDATE todos
                 SET title = ?1, description = ?2, due = ?3, priority = ?4, done = ?5,
                     completed_at = ?6, tags = ?7, sequence = sequence + 1, updated_at = ?8
                 WHERE calendar_id = ?9 AND uid = ?10",
                &[
                    Bind::Text(&title),
                    Bind::Text(&description),
                    Bind::Text(&due),
                    Bind::I64(priority),
                    Bind::I64(i64::from(done)),
                    Bind::Text(&completed_at),
                    Bind::Text(&pack_tags(&tags)),
                    Bind::Text(&now),
                    Bind::Text(calendar_id),
                    Bind::Text(uid),
                ],
            )?;
        } else {
            self.conn.execute(
                "INSERT INTO todos (
                    calendar_id, uid, title, description, due, priority, done, completed_at,
                    tags, sequence, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?11)",
                &[
                    Bind::Text(calendar_id),
                    Bind::Text(uid),
                    Bind::Text(&title),
                    Bind::Text(&description),
                    Bind::Text(&due),
                    Bind::I64(priority),
                    Bind::I64(i64::from(done)),
                    Bind::Text(&completed_at),
                    Bind::Text(&pack_tags(&tags)),
                    Bind::Text(&now),
                    Bind::Text(&now),
                ],
            )?;
        }
        let saved = self
            .get_todo(calendar_id, uid)?
            .ok_or_else(|| "failed to save todo".to_string())?;
        tx.commit()?;
        Ok((saved, existing.is_none()))
    }

    pub fn put_todo_seal(
        &self,
        calendar_id: &str,
        uid: &str,
        seal: &str,
    ) -> Result<(TodoRow, bool), String> {
        if !is_uid(uid) {
            return Err("uid must be 1-200 chars: letters, digits, . _ @ -".into());
        }
        let tx = self.conn.begin()?;
        let existing = self.get_todo(calendar_id, uid)?;
        let now = now_iso();
        let created = existing.is_none();
        if created {
            self.conn.execute(
                "INSERT INTO todos (
                    calendar_id, uid, title, description, due, priority, done, completed_at,
                    tags, sequence, created_at, updated_at, seal
                 ) VALUES (?1, ?2, '', '', '', 0, 0, '', '', 0, ?3, ?3, ?4)",
                &[
                    Bind::Text(calendar_id),
                    Bind::Text(uid),
                    Bind::Text(&now),
                    Bind::Text(seal),
                ],
            )?;
        } else {
            self.conn.execute(
                "UPDATE todos
                 SET title = '', description = '', due = '', priority = 0, done = 0,
                     completed_at = '', tags = '', seal = ?1, sequence = sequence + 1, updated_at = ?2
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
            .get_todo(calendar_id, uid)?
            .ok_or_else(|| "failed to save todo".to_string())?;
        tx.commit()?;
        Ok((saved, created))
    }

    /// Change only the fields present. `Err("not found")` if there is no such todo.
    pub fn patch_todo(
        &self,
        calendar_id: &str,
        uid: &str,
        fields: &TodoFields,
    ) -> Result<TodoRow, String> {
        let tx = self.conn.begin()?;
        let existing = self
            .get_todo(calendar_id, uid)?
            .ok_or_else(|| "not found".to_string())?;
        let now = now_iso();
        let title = fields.title.clone().unwrap_or_else(|| existing.title.clone());
        let description = fields
            .description
            .clone()
            .unwrap_or_else(|| existing.description.clone());
        let due = fields.due.clone().unwrap_or_else(|| existing.due.clone());
        let priority = fields.priority.unwrap_or(existing.priority);
        let tags = fields.tags.clone().unwrap_or_else(|| existing.tags.clone());
        let done = fields.done.unwrap_or(existing.done);
        let completed_at = completion(done, Some(&existing), &now);
        self.conn.execute(
            "UPDATE todos
             SET title = ?1, description = ?2, due = ?3, priority = ?4, done = ?5,
                 completed_at = ?6, tags = ?7, sequence = sequence + 1, updated_at = ?8
             WHERE calendar_id = ?9 AND uid = ?10",
            &[
                Bind::Text(&title),
                Bind::Text(&description),
                Bind::Text(&due),
                Bind::I64(priority),
                Bind::I64(i64::from(done)),
                Bind::Text(&completed_at),
                Bind::Text(&pack_tags(&tags)),
                Bind::Text(&now),
                Bind::Text(calendar_id),
                Bind::Text(uid),
            ],
        )?;
        let saved = self
            .get_todo(calendar_id, uid)?
            .ok_or_else(|| "failed to save todo".to_string())?;
        tx.commit()?;
        Ok(saved)
    }

    pub fn delete_todo(&self, calendar_id: &str, uid: &str) -> Result<bool, String> {
        let tx = self.conn.begin()?;
        let removed = self.conn.execute(
            "DELETE FROM todos WHERE calendar_id = ?1 AND uid = ?2",
            &[Bind::Text(calendar_id), Bind::Text(uid)],
        )?;
        tx.commit()?;
        Ok(removed > 0)
    }

    /// The feed token for one of a calendar's extra feeds (`"todos"`),
    /// minted on first use. Each kind has its own so a leaked token exposes
    /// one feed, not all of them.
    pub fn feed_token(&self, calendar_id: &str, kind: &str) -> Result<String, String> {
        const FIND: &str = "SELECT token FROM feeds WHERE calendar_id = ?1 AND kind = ?2";
        let binds = [Bind::Text(calendar_id), Bind::Text(kind)];
        if let Some(token) = self.conn.query_row(FIND, &binds, |row| row.text(0))? {
            return Ok(token);
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO feeds (token, calendar_id, kind) VALUES (?1, ?2, ?3)",
            &[Bind::Text(&secret()), Bind::Text(calendar_id), Bind::Text(kind)],
        )?;
        self.conn
            .query_row(FIND, &binds, |row| row.text(0))?
            .ok_or_else(|| "failed to create feed".to_string())
    }

    /// Which calendar and kind a feed token belongs to.
    pub fn feed_target(&self, token: &str) -> Result<Option<(String, String)>, String> {
        self.conn.query_row(
            "SELECT calendar_id, kind FROM feeds WHERE token = ?1",
            &[Bind::Text(token)],
            |row| (row.text(0), row.text(1)),
        )
    }
}

/// `completed_at` for a todo about to be saved. Finishing stamps the time;
/// staying finished keeps the original; reopening clears it.
fn completion(done: bool, existing: Option<&TodoRow>, now: &str) -> String {
    if !done {
        return String::new();
    }
    match existing {
        Some(e) if e.done && !e.completed_at.is_empty() => e.completed_at.clone(),
        _ => now.to_string(),
    }
}

// ---- input --------------------------------------------------------------------

/// Parse a todo body. A full body (`partial` false) needs a `title`; a patch
/// needs at least one field. Wrong types are errors, never silently dropped.
pub fn parse_todo_fields(body: &Value, partial: bool) -> Result<TodoFields, String> {
    let obj = body.as_object().ok_or("body must be an object")?;
    let mut fields = TodoFields::default();

    match obj.get("title") {
        None if partial => {}
        None => return Err("title is required".into()),
        Some(Value::String(s)) => {
            let title = s.trim();
            if title.is_empty() {
                return Err(if partial { "title cannot be empty" } else { "title is required" }.into());
            }
            if title.len() > 512 {
                return Err("title is too long".into());
            }
            fields.title = Some(title.to_string());
        }
        Some(_) => return Err("title must be a string".into()),
    }

    fields.description = match obj.get("description") {
        None => None,
        Some(Value::Null) => Some(String::new()),
        Some(Value::String(s)) if s.len() > 4000 => return Err("description is too long".into()),
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err("description must be a string".into()),
    };

    fields.due = match obj.get("due") {
        None => None,
        Some(Value::Null) => Some(String::new()),
        Some(Value::String(s)) => Some(parse_due(s.trim())?),
        Some(_) => return Err("due must be a string".into()),
    };

    fields.priority = match obj.get("priority") {
        None => None,
        Some(Value::Null) => Some(0),
        Some(Value::Number(n)) if (0..=9).contains(n) => Some(*n),
        Some(_) => return Err("priority must be a whole number from 0 to 9".into()),
    };

    fields.done = match obj.get("done") {
        None => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err("done must be true or false".into()),
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
        && fields.description.is_none()
        && fields.due.is_none()
        && fields.priority.is_none()
        && fields.done.is_none()
        && fields.tags.is_none()
    {
        return Err("no fields to update".into());
    }
    Ok(fields)
}

/// `""` clears, a date stays a date, anything else becomes a UTC instant.
fn parse_due(value: &str) -> Result<String, String> {
    if value.is_empty() || is_ymd(value) {
        return Ok(value.to_string());
    }
    parse_instant(value).ok_or_else(|| "due must be YYYY-MM-DD or an ISO-8601 datetime".to_string())
}

/// Lowercased, deduplicated, `[a-z0-9_-]{1,32}`, at most ten.
pub(crate) fn parse_tags(items: &[Value]) -> Result<Vec<String>, String> {
    let mut tags: Vec<String> = Vec::new();
    for item in items {
        let raw = item.as_str().ok_or("tags must be a list of strings")?;
        let tag = raw.trim().to_ascii_lowercase();
        let valid = !tag.is_empty()
            && tag.len() <= 32
            && tag
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
        if !valid {
            return Err("tags are 1-32 chars: letters, digits, _ -".into());
        }
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    if tags.len() > MAX_TAGS {
        return Err(format!("at most {MAX_TAGS} tags"));
    }
    Ok(tags)
}

pub fn json_todo(row: &TodoRow) -> Value {
    if !row.seal.is_empty() {
        return Value::object(&[
            ("uid", Value::String(row.uid.clone())),
            ("seal", Value::String(row.seal.clone())),
            ("createdAt", Value::String(row.created_at.clone())),
            ("updatedAt", Value::String(row.updated_at.clone())),
        ]);
    }
    let optional = |s: &str| {
        if s.is_empty() {
            Value::Null
        } else {
            Value::String(s.to_string())
        }
    };
    Value::object(&[
        ("uid", Value::String(row.uid.clone())),
        ("title", Value::String(row.title.clone())),
        ("description", Value::String(row.description.clone())),
        ("due", optional(&row.due)),
        ("priority", Value::Number(row.priority)),
        ("done", Value::Bool(row.done)),
        ("completedAt", optional(&row.completed_at)),
        (
            "tags",
            Value::Array(row.tags.iter().cloned().map(Value::String).collect()),
        ),
        ("createdAt", Value::String(row.created_at.clone())),
        ("updatedAt", Value::String(row.updated_at.clone())),
    ])
}

// ---- routes -------------------------------------------------------------------

/// `/v1/c/{id}/todos` (no uid) and `/v1/c/{id}/todos/{uid}`. `/v1/todos` is `home`.
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
    let done = match query_param(req, "status").as_deref() {
        None | Some("all") => None,
        Some("open") => Some(false),
        Some("done") => Some(true),
        Some(_) => return json_err(400, "status must be open, done or all"),
    };
    let tag = query_param(req, "tag").map(|t| t.trim().to_ascii_lowercase());
    let rows = match lock_db(state).list_todos(&cal.id, done) {
        Ok(rows) => rows,
        Err(e) => return json_err(500, &e),
    };
    let todos: Vec<Value> = rows
        .iter()
        .filter(|row| tag.as_ref().is_none_or(|t| row.tags.contains(t)))
        .map(json_todo)
        .collect();
    json_response(200, &stringify(&Value::object(&[("todos", Value::Array(todos))])))
}

fn get(state: &AppState, cal: &CalendarRow, uid: &str) -> Response {
    match lock_db(state).get_todo(&cal.id, uid) {
        Ok(Some(row)) => json_response(200, &stringify(&json_todo(&row))),
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
            let creating = match db.get_todo(&cal.id, &uid) {
                Ok(existing) => existing.is_none(),
                Err(e) => return json_err(500, &e),
            };
            if creating {
                match db.count_todos(&cal.id) {
                    Ok(n) if n >= state.limits.max_todos => return json_err(409, "todo list is full"),
                    Ok(_) => {}
                    Err(e) => return json_err(500, &e),
                }
            }
        }
        return match db.put_todo_seal(&cal.id, &uid, &wire) {
            Ok((row, created)) => json_response(
                if created { 201 } else { 200 },
                &stringify(&json_todo(&row)),
            ),
            Err(e) => json_err(400, &e),
        };
    }
    let fields = match parse_todo_fields(&body, false) {
        Ok(f) => f,
        Err(e) => return json_err(400, &e),
    };
    let uid = path_uid.map(str::to_string).or_else(|| fields.uid.clone());
    let db = lock_db(state);
    if !is_home(cal) {
        let creating = match uid.as_deref() {
            Some(uid) => match db.get_todo(&cal.id, uid) {
                Ok(existing) => existing.is_none(),
                Err(e) => return json_err(500, &e),
            },
            None => true,
        };
        if creating {
            match db.count_todos(&cal.id) {
                Ok(n) if n >= state.limits.max_todos => return json_err(409, "todo list is full"),
                Ok(_) => {}
                Err(e) => return json_err(500, &e),
            }
        }
    }
    match db.upsert_todo(&cal.id, uid.as_deref(), &fields) {
        Ok((row, created)) => json_response(
            if created { 201 } else { 200 },
            &stringify(&json_todo(&row)),
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
    let fields = match parse_todo_fields(&body, true) {
        Ok(f) => f,
        Err(e) => return json_err(400, &e),
    };
    match lock_db(state).patch_todo(&cal.id, uid, &fields) {
        Ok(row) => json_response(200, &stringify(&json_todo(&row))),
        Err(e) if e == "not found" => json_err(404, &e),
        Err(e) => json_err(400, &e),
    }
}

fn delete(state: &AppState, cal: &CalendarRow, uid: &str) -> Response {
    match lock_db(state).delete_todo(&cal.id, uid) {
        Ok(true) => Response::new(204),
        Ok(false) => json_err(404, "not found"),
        Err(e) => json_err(500, &e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::parse;

    fn fields(json: &str, partial: bool) -> Result<TodoFields, String> {
        parse_todo_fields(&parse(json).unwrap(), partial)
    }

    #[test]
    fn a_full_body_needs_a_title() {
        assert!(fields(r#"{"title":"Buy milk"}"#, false).is_ok());
        assert_eq!(fields(r#"{}"#, false).unwrap_err(), "title is required");
        assert_eq!(fields(r#"{"title":"  "}"#, false).unwrap_err(), "title is required");
        assert!(fields(&format!(r#"{{"title":"{}"}}"#, "x".repeat(513)), false).is_err());
    }

    #[test]
    fn a_patch_needs_something_to_change() {
        assert_eq!(fields(r#"{}"#, true).unwrap_err(), "no fields to update");
        assert_eq!(fields(r#"{"title":""}"#, true).unwrap_err(), "title cannot be empty");
        assert!(fields(r#"{"done":true}"#, true).is_ok());
    }

    #[test]
    fn wrong_types_are_rejected_not_ignored() {
        for bad in [
            r#"{"title":"x","description":5}"#,
            r#"{"title":"x","due":5}"#,
            r#"{"title":"x","priority":"high"}"#,
            r#"{"title":"x","priority":10}"#,
            r#"{"title":"x","priority":-1}"#,
            r#"{"title":"x","done":"yes"}"#,
            r#"{"title":"x","done":null}"#,
            r#"{"title":"x","tags":"a"}"#,
            r#"{"title":"x","tags":[1]}"#,
            r#"{"title":5}"#,
            r#"{"title":"x","uid":"has space"}"#,
        ] {
            assert!(fields(bad, false).is_err(), "{bad}");
        }
    }

    #[test]
    fn due_is_a_day_an_instant_or_cleared() {
        let due = |v: &str| fields(&format!(r#"{{"title":"x","due":{v}}}"#), false).map(|f| f.due);
        assert_eq!(due(r#""2026-10-01""#).unwrap(), Some("2026-10-01".into()));
        assert_eq!(
            due(r#""2026-10-01T09:00:00-07:00""#).unwrap(),
            Some("2026-10-01T16:00:00.000Z".into())
        );
        assert_eq!(due(r#""""#).unwrap(), Some(String::new()));
        assert_eq!(due("null").unwrap(), Some(String::new()));
        assert!(due(r#""tomorrow""#).is_err());
        assert!(due(r#""2026-13-01""#).is_err());
        assert!(due(r#""2026-10-0é""#).is_err());
    }

    #[test]
    fn tags_are_normalized_and_bounded() {
        let tags = |v: &str| fields(&format!(r#"{{"title":"x","tags":{v}}}"#), false).map(|f| f.tags);
        assert_eq!(
            tags(r#"[" Home ","work","HOME","a_b-1"]"#).unwrap(),
            Some(vec!["home".into(), "work".into(), "a_b-1".into()])
        );
        assert_eq!(tags("[]").unwrap(), Some(vec![]));
        assert!(tags(r#"["has space"]"#).is_err());
        assert!(tags(r#"["a,b"]"#).is_err());
        assert!(tags(r#"[""]"#).is_err());
        assert!(tags(&format!(r#"["{}"]"#, "x".repeat(33))).is_err());
        let eleven: Vec<String> = (0..11).map(|i| format!("\"t{i}\"")).collect();
        assert!(tags(&format!("[{}]", eleven.join(","))).is_err());
    }

    #[test]
    fn tags_survive_the_round_trip_through_storage() {
        assert_eq!(unpack_tags(&pack_tags(&["a".into(), "b-c".into()])), vec!["a", "b-c"]);
        assert_eq!(pack_tags(&[]), "");
        assert!(unpack_tags("").is_empty());
    }
}
