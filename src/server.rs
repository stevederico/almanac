use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use crate::db::{
    json_calendar, json_calendar_created, json_event, normalize_recurrence_id, parse_event_input,
    parse_event_patch, parse_override_input, parse_recurrence_id, CalendarRow, Db, EventRow,
};
use crate::atom::{render_atom, AtomEntry};
use crate::export::{export_json, export_markdown};
use crate::http::{html_response, json_response, text_response, Request, Response};
use crate::ics::{render_calendar, render_todos, IcsEvent, IcsOverride, IcsTodo};
use crate::json::{parse as parse_json, stringify, Value};
use crate::landing::{html_created, html_home, json_index, llms_txt};
use crate::limits::{client_id, Limiter, Limits, HOUR_MS, MINUTE_MS};
use crate::sha256::sha256_hex;
use crate::time::now_iso;
use crate::notes::{self, NoteRow};
use crate::todos::{self, TodoRow};

const OG_PNG: &[u8] = include_bytes!("../public/og.png");

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<Db>>,
    pub public_base: String,
    pub limits: Limits,
    pub limiter: Arc<Limiter>,
}

impl AppState {
    pub fn new(db: Db, public_base: String) -> Self {
        Self {
            db: Arc::new(Mutex::new(db)),
            public_base,
            limits: Limits::default(),
            limiter: Arc::new(Limiter::default()),
        }
    }
}

/// Lock the database, surviving a poisoned mutex.
///
/// A panic while a handler held the guard used to poison it, and every later
/// `.expect("db")` then panicked too: one bad request took every DB route down
/// until a restart. The connection holds no half-applied state we rely on, so
/// recovering the guard is safe.
pub(crate) fn lock_db(state: &AppState) -> MutexGuard<'_, Db> {
    state.db.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn handle(state: &AppState, req: &Request) -> Response {
    let t0 = Instant::now();
    let res = dispatch(state, req);
    let log = Value::object(&[
        ("t", Value::String(now_iso())),
        ("msg", Value::String("req".into())),
        ("method", Value::String(req.method.clone())),
        ("path", Value::String(redact_feed(&req.path))),
        ("status", Value::Number(i64::from(res.status))),
        ("ms", Value::Number(t0.elapsed().as_millis() as i64)),
        ("ua", Value::String(req.header("user-agent").to_string())),
    ]);
    println!("{}", stringify(&log));
    res
}

fn dispatch(state: &AppState, req: &Request) -> Response {
    let method = req.method.as_str();
    let path = req.path.split('?').next().unwrap_or(req.path.as_str());
    match (method, path) {
        ("GET", "/") => home(state, req),
        ("GET", "/health") => health(state),
        ("GET", "/llms.txt") => {
            let base = request_base(req, &state.public_base);
            text_response(200, "text/plain; charset=utf-8", llms_txt(&base))
        }
        ("GET", "/og.png") => text_response(200, "image/png", OG_PNG.to_vec()),
        ("POST", "/calendars") => create_calendar(state, req),
        (m, p) if p.starts_with("/feed/") => {
            if m != "GET" && m != "HEAD" {
                return json_err(404, "not found");
            }
            let res = feed(state, &p["/feed/".len()..]);
            if m == "HEAD" {
                return head_of(res);
            }
            res
        }
        (m, p) if p.starts_with("/v1/c/") => scoped(state, req, m, &p["/v1/c/".len()..]),
        (m, "/v1/events") => events(state, req, "home", m, Target::Collection),
        (m, "/v1/todos") => todos::route(state, req, "home", m, None),
        (m, p) if let Some(uid) = p.strip_prefix("/v1/todos/") => {
            item_route(state, req, "home", m, uid, Resource::Todos)
        }
        (m, "/v1/export") => export(state, req, "home", m),
        (m, "/v1/notes") => notes::route(state, req, "home", m, None),
        (m, p) if let Some(uid) = p.strip_prefix("/v1/notes/") => {
            item_route(state, req, "home", m, uid, Resource::Notes)
        }
        (m, p) if let Some(rest) = p.strip_prefix("/v1/events/") => {
            events(state, req, "home", m, Target::of(rest))
        }
        _ => json_err(404, "not found"),
    }
}

/// Same status and headers as the GET, no body. The length is the GET's, as
/// HTTP requires.
fn head_of(mut res: Response) -> Response {
    let len = res.body.len();
    res.body = Vec::new();
    res.header("content-length", &len.to_string())
}

fn health(state: &AppState) -> Response {
    match lock_db(state).ping() {
        Ok(()) => json_response(200, &stringify(&Value::object(&[("ok", Value::Bool(true))]))),
        Err(e) => json_err(503, &e),
    }
}

fn home(state: &AppState, req: &Request) -> Response {
    let base = request_base(req, &state.public_base);
    if wants_html(req) {
        html_response(200, html_home(&base))
    } else {
        json_response(200, &stringify(&json_index(&base)))
    }
}

fn too_many(retry_after: u64) -> Response {
    json_err(429, "rate limit exceeded").header("retry-after", &retry_after.to_string())
}

fn create_calendar(state: &AppState, req: &Request) -> Response {
    // Cheapest checks first, before the body is parsed. Per-client goes ahead
    // of the global counter so one abusive client cannot spend everyone's budget.
    let client = client_id(
        req.header("x-forwarded-for"),
        &req.peer,
        state.limits.proxy_hops,
    );
    let limits = &state.limits;
    if let Err(retry) = state
        .limiter
        .check(&format!("c:{client}"), limits.creates_per_client_hour, HOUR_MS)
    {
        return too_many(retry);
    }
    if let Err(retry) = state
        .limiter
        .check("c:*", limits.creates_global_hour, HOUR_MS)
    {
        // Nobody can create a calendar while this holds, so say so loudly:
        // it means either real growth or a caller picking its own bucket.
        eprintln!(
            "almanac: GLOBAL create limit of {}/hour reached, creation is blocked for {retry}s",
            limits.creates_global_hour
        );
        return too_many(retry);
    }
    let ctype = req.header("content-type");
    let name = match parse_calendar_name(ctype, &req.body) {
        Ok(name) => name,
        Err(e) => return json_err(400, &e),
    };
    let db = lock_db(state);
    match db.count_calendars() {
        Ok(n) if n >= limits.max_calendars => return json_err(503, "calendar limit reached"),
        Ok(_) => {}
        Err(e) => return json_err(500, &e),
    }
    let (saved, key) = match db.create_calendar(name.as_deref(), None, None, None) {
        Ok(created) => created,
        Err(e) => return json_err(400, &e),
    };
    let base = request_base(req, &state.public_base);
    let view = match calendar_view(&db, &saved, Some(&key), &base) {
        Ok(v) => v,
        Err(e) => return json_err(500, &e),
    };
    drop(db);
    if wants_html(req) || ctype.contains("application/x-www-form-urlencoded") {
        let feed = |kind: &str| {
            view.get(kind)
                .and_then(|t| t.get("subscribe"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let page = html_created(&saved, &key, &feed("todos"), &feed("notes"), &base);
        return html_response(201, page);
    }
    json_response(201, &stringify(&view))
}

fn parse_calendar_name(ctype: &str, body: &[u8]) -> Result<Option<String>, String> {
    if ctype.contains("application/json") {
        let text = std::str::from_utf8(body).map_err(|_| "body must be valid JSON")?;
        if text.trim().is_empty() {
            return Ok(None);
        }
        let value = parse_json(text).map_err(|_| "body must be valid JSON")?;
        if !matches!(value, Value::Object(_)) {
            return Err("body must be an object".into());
        }
        return match value.get("name") {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_str()
                .map(|s| Some(s.to_string()))
                .ok_or_else(|| "name must be a string".to_string()),
        };
    }
    if ctype.contains("application/x-www-form-urlencoded") {
        let text = String::from_utf8_lossy(body);
        for pair in text.split('&') {
            let mut it = pair.splitn(2, '=');
            let key = it.next().unwrap_or("");
            let val = it.next().unwrap_or("");
            if key == "name" {
                return Ok(Some(urlencoding_decode(val)));
            }
        }
    }
    Ok(None)
}

pub(crate) fn urlencoding_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => match bytes
                .get(i + 1)
                .zip(bytes.get(i + 2))
                .and_then(|(hi, lo)| hex_pair(*hi, *lo))
            {
                Some(b) => {
                    out.push(b);
                    i += 2;
                }
                None => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_pair(hi: u8, lo: u8) -> Option<u8> {
    let digit = |b: u8| (b as char).to_digit(16);
    Some((digit(hi)? * 16 + digit(lo)?) as u8)
}

/// Newest notes an Atom feed carries. Readers only show recent entries, and a
/// full calendar of long notes would be megabytes to poll.
const NOTES_IN_FEED: i64 = 200;

/// What a feed token resolved to, read under the lock.
enum FeedBody {
    Events(String, Vec<EventRow>),
    Todos(String, Vec<TodoRow>),
    Notes(String, String, Vec<NoteRow>),
}

fn feed(state: &AppState, token: &str) -> Response {
    let token = feed_token_of(token);
    // Read under the lock, render after it is released: rendering a large
    // calendar should not stall every other request.
    let body = {
        let db = lock_db(state);
        match db.get_calendar_by_feed_token(&token) {
            Ok(Some(cal)) if safe_equal(&token, &cal.feed_token) => match db.list_events(&cal.id) {
                Ok(rows) => FeedBody::Events(cal.name, rows),
                Err(e) => return json_err(500, &e),
            },
            _ => match db.feed_target(&token) {
                Ok(Some((cal_id, kind))) if kind == "todos" || kind == "notes" => {
                    let name = match db.get_calendar(&cal_id) {
                        Ok(Some(cal)) => cal.name,
                        _ => return json_err(404, "not found"),
                    };
                    if kind == "todos" {
                        match db.list_todos(&cal_id, None) {
                            Ok(rows) => FeedBody::Todos(name, rows),
                            Err(e) => return json_err(500, &e),
                        }
                    } else {
                        match db.list_notes(&cal_id, None, NOTES_IN_FEED) {
                            Ok(rows) => FeedBody::Notes(cal_id, name, rows),
                            Err(e) => return json_err(500, &e),
                        }
                    }
                }
                Ok(_) => return json_err(404, "not found"),
                Err(e) => return json_err(500, &e),
            },
        }
    };
    let (filename, ctype, ics) = match body {
        FeedBody::Events(name, events) => (
            "calendar.ics",
            "text/calendar; charset=utf-8",
            render_events(&name, events),
        ),
        FeedBody::Notes(cal_id, name, notes) => {
            let updated = notes
                .iter()
                .map(|n| n.updated_at.as_str())
                .max()
                .map(str::to_string)
                .unwrap_or_else(now_iso);
            let entries: Vec<AtomEntry> = notes
                .into_iter()
                .map(|n| AtomEntry {
                    id: format!("urn:almanac:{cal_id}:{}", n.uid),
                    title: n.title,
                    updated: n.updated_at,
                    published: n.created_at,
                    content: n.body,
                    tags: n.tags,
                })
                .collect();
            (
                "notes.atom",
                "application/atom+xml; charset=utf-8",
                render_atom(
                    &format!("urn:almanac:{cal_id}:notes"),
                    &format!("{name} Notes"),
                    &updated,
                    &entries,
                ),
            )
        }
        FeedBody::Todos(name, todos) => {
            let items: Vec<IcsTodo> = todos
                .into_iter()
                .map(|t| IcsTodo {
                    uid: t.uid,
                    title: t.title,
                    description: t.description,
                    due: t.due,
                    done: t.done,
                    completed_at: t.completed_at,
                    priority: t.priority,
                    tags: t.tags,
                    sequence: t.sequence,
                    created_at: t.created_at,
                    updated_at: t.updated_at,
                })
                .collect();
            ("todos.ics", "text/calendar; charset=utf-8", render_todos(&name, &items))
        }
    };
    text_response(200, ctype, ics)
        .header("cache-control", "no-cache")
        .header("content-disposition", &format!("inline; filename=\"{filename}\""))
}

fn render_events(name: &str, events: Vec<EventRow>) -> String {
    let ics_events: Vec<IcsEvent> = events
        .into_iter()
        .map(|ev| IcsEvent {
            uid: ev.uid,
            summary: ev.summary,
            description: ev.description,
            location: ev.location,
            dtstart: ev.dtstart,
            dtend: ev.dtend,
            all_day: ev.all_day,
            transparent: ev.transparent,
            sequence: ev.sequence,
            updated_at: ev.updated_at,
            rrule: ev.rrule,
            tzid: ev.tzid,
            exdates: ev.exdates,
            overrides: ev
                .overrides
                .into_iter()
                .map(|over| IcsOverride {
                    recurrence_id: over.recurrence_id,
                    summary: over.summary,
                    description: over.description,
                    location: over.location,
                    dtstart: over.dtstart,
                    dtend: over.dtend,
                    all_day: over.all_day,
                    transparent: over.transparent,
                    sequence: over.sequence,
                    updated_at: over.updated_at,
                })
                .collect(),
        })
        .collect();
    render_calendar(name, &ics_events)
}

fn scoped(state: &AppState, req: &Request, method: &str, rest: &str) -> Response {
    let Some((id, rest)) = rest.split_once('/') else {
        return match method {
            "GET" => calendar_info(state, req, rest),
            "DELETE" => delete_calendar(state, req, rest),
            _ => json_err(404, "not found"),
        };
    };
    if rest == "events" {
        return events(state, req, id, method, Target::Collection);
    }
    if let Some(rest) = rest.strip_prefix("events/") {
        return events(state, req, id, method, Target::of(rest));
    }
    if rest == "todos" {
        return todos::route(state, req, id, method, None);
    }
    if let Some(uid) = rest.strip_prefix("todos/") {
        return item_route(state, req, id, method, uid, Resource::Todos);
    }
    if rest == "export" {
        return export(state, req, id, method);
    }
    if rest == "notes" {
        return notes::route(state, req, id, method, None);
    }
    if let Some(uid) = rest.strip_prefix("notes/") {
        return item_route(state, req, id, method, uid, Resource::Notes);
    }
    json_err(404, "not found")
}

/// `GET /v1/c/{id}/export?format=json|md`: everything in the calendar, in one
/// document. `/v1/export` is `home`.
fn export(state: &AppState, req: &Request, cal_id: &str, method: &str) -> Response {
    if method != "GET" {
        return json_err(404, "not found");
    }
    let markdown = match query_param(req, "format").as_deref() {
        None | Some("json") => false,
        Some("md" | "markdown") => true,
        Some(_) => return json_err(400, "format must be json or md"),
    };
    let cal = match gate(state, req, cal_id, method) {
        Ok(cal) => cal,
        Err(res) => return res,
    };
    // Reading everything takes the database lock for a while, so exports are
    // metered. The owner's own calendar is exempt, like its other limits.
    if !is_home(&cal) {
        if let Err(retry) = state.limiter.check(
            &format!("x:{}", cal.id),
            state.limits.exports_per_hour,
            HOUR_MS,
        ) {
            return too_many(retry);
        }
    }
    let (events, todos, notes) = {
        let db = lock_db(state);
        let events = db.list_events(&cal.id);
        let todos = db.list_todos(&cal.id, None);
        let notes = db.list_notes(&cal.id, None, 0);
        match (events, todos, notes) {
            (Ok(e), Ok(t), Ok(n)) => (e, t, n),
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => return json_err(500, &e),
        }
    };
    let exported_at = now_iso();
    let (ctype, ext, body) = if markdown {
        (
            "text/markdown; charset=utf-8",
            "md",
            export_markdown(&cal, &exported_at, &events, &todos, &notes),
        )
    } else {
        (
            "application/json; charset=utf-8",
            "json",
            stringify(&export_json(&cal, &exported_at, &events, &todos, &notes)),
        )
    };
    text_response(200, ctype, body)
        .header("cache-control", "no-store")
        .header(
            "content-disposition",
            &format!("attachment; filename=\"almanac-{}.{ext}\"", cal.id),
        )
}

#[derive(Clone, Copy)]
enum Resource {
    Todos,
    Notes,
}

/// `todos/{uid}` or `notes/{uid}`. A uid never contains a slash, so anything
/// deeper is a 404.
fn item_route(
    state: &AppState,
    req: &Request,
    cal_id: &str,
    method: &str,
    uid: &str,
    resource: Resource,
) -> Response {
    if uid.is_empty() || uid.contains('/') {
        return json_err(404, "not found");
    }
    match resource {
        Resource::Todos => todos::route(state, req, cal_id, method, Some(uid)),
        Resource::Notes => notes::route(state, req, cal_id, method, Some(uid)),
    }
}

fn delete_calendar(state: &AppState, req: &Request, id: &str) -> Response {
    let cal = match gate(state, req, id, "DELETE") {
        Ok(cal) => cal,
        Err(res) => return res,
    };
    // After auth, so a wrong key is 401 and does not reveal that `home` exists.
    // The env recreates `home` on the next boot, so deleting it would not stick.
    if is_home(&cal) {
        return json_err(403, "home calendar cannot be deleted");
    }
    match lock_db(state).delete_calendar(&cal.id) {
        Ok(true) => Response::new(204),
        Ok(false) => json_err(404, "not found"),
        Err(e) => json_err(500, &e),
    }
}

fn calendar_info(state: &AppState, req: &Request, id: &str) -> Response {
    let Some(cal) = authenticate(state, req, id) else {
        return json_err(401, "unauthorized");
    };
    let base = request_base(req, &state.public_base);
    let view = calendar_view(&lock_db(state), &cal, None, &base);
    match view {
        Ok(v) => json_response(200, &stringify(&v)),
        Err(e) => json_err(500, &e),
    }
}

/// The calendar as the API shows it: its own fields plus where its todos and
/// notes are written and subscribed. `key` is only ever passed at creation.
fn calendar_view(db: &Db, cal: &CalendarRow, key: Option<&str>, base: &str) -> Result<Value, String> {
    let mut view = match key {
        Some(key) => json_calendar_created(cal, key, base),
        None => json_calendar(cal, base),
    };
    let origin = base.trim_end_matches('/');
    let todos_token = db.feed_token(&cal.id, "todos")?;
    let notes_token = db.feed_token(&cal.id, "notes")?;
    if let Value::Object(map) = &mut view {
        map.insert(
            "todos".into(),
            Value::object(&[
                ("write", Value::String(format!("{origin}/v1/c/{}/todos", cal.id))),
                ("subscribe", Value::String(format!("{origin}/feed/{todos_token}.ics"))),
            ]),
        );
        map.insert(
            "notes".into(),
            Value::object(&[
                ("write", Value::String(format!("{origin}/v1/c/{}/notes", cal.id))),
                ("subscribe", Value::String(format!("{origin}/feed/{notes_token}.atom"))),
            ]),
        );
    }
    Ok(view)
}

enum Target<'a> {
    Collection,
    Event(&'a str),
    Exdates(&'a str),
    Overrides(&'a str),
}

impl<'a> Target<'a> {
    /// What follows `events/`.
    fn of(rest: &'a str) -> Self {
        match rest.split_once('/') {
            Some((uid, "exdates")) => Target::Exdates(uid),
            Some((uid, "overrides")) => Target::Overrides(uid),
            _ => Target::Event(rest),
        }
    }
}

/// Every events route, for a named calendar. `/v1/events` is `home`.
fn events(
    state: &AppState,
    req: &Request,
    cal_id: &str,
    method: &str,
    target: Target,
) -> Response {
    let allowed = matches!(
        (&target, method),
        (Target::Collection, "GET" | "POST")
            | (Target::Event(_), "GET" | "PUT" | "PATCH" | "DELETE")
            | (Target::Exdates(_) | Target::Overrides(_), "PUT" | "DELETE")
    );
    if !allowed {
        return json_err(404, "not found");
    }
    let cal = match gate(state, req, cal_id, method) {
        Ok(cal) => cal,
        Err(res) => return res,
    };
    match (target, method) {
        (Target::Collection, "GET") => list_events(state, &cal),
        (Target::Collection, _) => write_event(state, req, &cal, None),
        (Target::Event(uid), "GET") => get_event(state, &cal, uid),
        (Target::Event(uid), "PUT") => write_event(state, req, &cal, Some(uid)),
        (Target::Event(uid), "PATCH") => patch_event(state, req, &cal, uid),
        (Target::Event(uid), _) => delete_event(state, &cal, uid),
        (Target::Exdates(uid), m) => exception(state, req, &cal, m, uid, true),
        (Target::Overrides(uid), m) => exception(state, req, &cal, m, uid, false),
    }
}

/// Authenticate, then charge the calendar's write budget. One budget covers
/// every resource on a calendar, so events and todos share it.
pub(crate) fn gate(
    state: &AppState,
    req: &Request,
    cal_id: &str,
    method: &str,
) -> Result<CalendarRow, Response> {
    let Some(cal) = authenticate(state, req, cal_id) else {
        return Err(json_err(401, "unauthorized"));
    };
    if method != "GET" && !is_home(&cal) {
        // After auth, so a stranger cannot spend someone else's budget.
        if let Err(retry) = state.limiter.check(
            &format!("w:{}", cal.id),
            state.limits.writes_per_minute,
            MINUTE_MS,
        ) {
            return Err(too_many(retry));
        }
    }
    Ok(cal)
}

/// One query-string parameter, decoded.
pub(crate) fn query_param(req: &Request, key: &str) -> Option<String> {
    let query = req.path.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (urlencoding_decode(k) == key).then(|| urlencoding_decode(v))
    })
}

/// The owner's own calendar is not subject to the caps meant for strangers.
pub(crate) fn is_home(cal: &CalendarRow) -> bool {
    cal.id == "home"
}

fn authenticate(state: &AppState, req: &Request, id: &str) -> Option<CalendarRow> {
    let db = lock_db(state);
    calendar_auth(&db, id, req.header("authorization"))
}

fn list_events(state: &AppState, cal: &CalendarRow) -> Response {
    match lock_db(state).list_events(&cal.id) {
        Ok(rows) => json_response(
            200,
            &stringify(&Value::object(&[(
                "events",
                Value::Array(rows.iter().map(json_event).collect()),
            )])),
        ),
        Err(e) => json_err(500, &e),
    }
}

fn get_event(state: &AppState, cal: &CalendarRow, uid: &str) -> Response {
    match lock_db(state).get_event(&cal.id, uid) {
        Ok(Some(row)) => json_response(200, &stringify(&json_event(&row))),
        Ok(None) => json_err(404, "not found"),
        Err(e) => json_err(500, &e),
    }
}

/// POST (no uid) and PUT (uid from the path).
fn write_event(state: &AppState, req: &Request, cal: &CalendarRow, uid: Option<&str>) -> Response {
    // Parse before taking the lock: a slow body must not stall other requests.
    let body = match parse_body(req) {
        Ok(v) => v,
        Err(e) => return json_err(400, &e),
    };
    let mut input = match parse_event_input(&body) {
        Ok(i) => i,
        Err(e) => return json_err(400, &e),
    };
    if let Some(uid) = uid {
        input.uid = Some(uid.to_string());
    }
    let db = lock_db(state);
    if !is_home(cal) {
        let creating = match input.uid.as_deref() {
            Some(uid) => match db.get_event(&cal.id, uid) {
                Ok(existing) => existing.is_none(),
                Err(e) => return json_err(500, &e),
            },
            None => true,
        };
        if creating {
            match db.count_events(&cal.id) {
                Ok(n) if n >= state.limits.max_events => {
                    return json_err(409, "calendar is full");
                }
                Ok(_) => {}
                Err(e) => return json_err(500, &e),
            }
        }
    }
    match db.upsert_event(&cal.id, &input) {
        Ok(saved) => {
            let status = if saved.sequence == 0 { 201 } else { 200 };
            json_response(status, &stringify(&json_event(&saved)))
        }
        Err(e) => json_err(400, &e),
    }
}

fn patch_event(state: &AppState, req: &Request, cal: &CalendarRow, uid: &str) -> Response {
    let body = match parse_body(req) {
        Ok(v) => v,
        Err(e) => return json_err(400, &e),
    };
    let patch = match parse_event_patch(&body) {
        Ok(p) => p,
        Err(e) => return json_err(400, &e),
    };
    match lock_db(state).patch_event(&cal.id, uid, &patch) {
        Ok(row) => json_response(200, &stringify(&json_event(&row))),
        Err(e) if e == "not found" => json_err(404, &e),
        Err(e) => json_err(400, &e),
    }
}

fn delete_event(state: &AppState, cal: &CalendarRow, uid: &str) -> Response {
    match lock_db(state).delete_event(&cal.id, uid) {
        Ok(true) => Response::new(204),
        Ok(false) => json_err(404, "not found"),
        Err(e) => json_err(500, &e),
    }
}

/// `exdates` (skip one date) and `overrides` (change one date), PUT or DELETE.
fn exception(
    state: &AppState,
    req: &Request,
    cal: &CalendarRow,
    method: &str,
    uid: &str,
    exdate: bool,
) -> Response {
    let body = match parse_body(req) {
        Ok(v) => v,
        Err(e) => return json_err(400, &e),
    };
    let put_override = if !exdate && method == "PUT" {
        match parse_override_input(&body) {
            Ok(input) => Some(input),
            Err(e) => return json_err(400, &e),
        }
    } else {
        None
    };
    let recurrence_id = match &put_override {
        Some(input) => input.recurrence_id.clone(),
        None => match parse_recurrence_id(&body) {
            Ok(id) => id,
            Err(e) => return json_err(400, &e),
        },
    };
    let db = lock_db(state);
    if method == "PUT" && !is_home(cal) {
        if let Some(full) = exceptions_full(&db, cal, uid, &recurrence_id, state.limits.max_exceptions)
        {
            return full;
        }
    }
    let saved = if method == "DELETE" {
        let removed = if exdate {
            db.delete_exdate(&cal.id, uid, &recurrence_id)
        } else {
            db.delete_override(&cal.id, uid, &recurrence_id)
        };
        return match removed {
            Ok(true) => Response::new(204),
            Ok(false) => json_err(404, "not found"),
            Err(e) if e == "not found" => json_err(404, &e),
            Err(e) => json_err(400, &e),
        };
    } else if let Some(input) = &put_override {
        db.put_override(&cal.id, uid, input)
    } else {
        db.put_exdate(&cal.id, uid, &recurrence_id)
    };
    match saved {
        Ok((row, created)) => json_response(
            if created { 201 } else { 200 },
            &stringify(&json_event(&row)),
        ),
        Err(e) if e == "not found" => json_err(404, &e),
        Err(e) => json_err(400, &e),
    }
}

/// `Some(409)` when adding this exception would pass the per-series cap.
/// Replacing one that already exists is always allowed. Anything that fails
/// here is left for the db call to report with its usual error.
fn exceptions_full(
    db: &Db,
    cal: &CalendarRow,
    uid: &str,
    recurrence_id: &str,
    max: usize,
) -> Option<Response> {
    let event = db.get_event(&cal.id, uid).ok().flatten()?;
    if event.exdates.len() + event.overrides.len() < max {
        return None;
    }
    let rid = normalize_recurrence_id(recurrence_id, event.all_day).ok()?;
    let exists = event.exdates.contains(&rid)
        || event.overrides.iter().any(|o| o.recurrence_id == rid);
    (!exists).then(|| json_err(409, "too many exceptions on this event"))
}

pub(crate) fn parse_body(req: &Request) -> Result<Value, String> {
    let text = std::str::from_utf8(&req.body).map_err(|_| "body must be an object")?;
    if text.is_empty() {
        return Err("body must be an object".into());
    }
    parse_json(text).map_err(|_| "body must be an object".into())
}

fn calendar_auth(db: &Db, id: &str, header: &str) -> Option<CalendarRow> {
    let cal = db.get_calendar(id).ok().flatten()?;
    // Keys are stored hashed; compare hashes. `safe_equal` refuses an empty
    // expected value, so a row without a hash can never authenticate.
    if safe_equal(&sha256_hex(&bearer(header)), &cal.key_hash) {
        Some(cal)
    } else {
        None
    }
}

fn bearer(header: &str) -> String {
    header.strip_prefix("Bearer ").unwrap_or("").to_string()
}

fn safe_equal(given: &str, expected: &str) -> bool {
    if expected.is_empty() || given.len() != expected.len() {
        return false;
    }
    given
        .bytes()
        .zip(expected.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

fn feed_token_of(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    for ext in [".ics", ".atom"] {
        if lower.ends_with(ext) && raw.len() > ext.len() {
            return raw[..raw.len() - ext.len()].to_string();
        }
    }
    raw.to_string()
}

fn request_base(req: &Request, fallback: &str) -> String {
    if !fallback.is_empty() {
        return fallback.trim_end_matches('/').to_string();
    }
    let proto = req.header("x-forwarded-proto");
    let proto = if proto.is_empty() { "http" } else { proto };
    let host = req.header("host");
    let host = if host.is_empty() { "localhost" } else { host };
    format!("{proto}://{host}")
}

fn wants_html(req: &Request) -> bool {
    let accept = req.header("accept");
    accept.contains("text/html") && !accept.contains("application/json")
}

fn redact_feed(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    if let Some(idx) = lower.find("/feed/") {
        let start = idx + "/feed/".len();
        let rest = &path[start..];
        let end = rest.find('/').unwrap_or(rest.len());
        let mut out = String::new();
        out.push_str(&path[..start]);
        out.push_str("<token>");
        out.push_str(&rest[end..]);
        return out;
    }
    path.to_string()
}

pub(crate) fn json_err(status: u16, msg: &str) -> Response {
    json_response(
        status,
        &stringify(&Value::object(&[("error", Value::String(msg.into()))])),
    )
}
