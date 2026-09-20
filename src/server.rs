use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::db::{
    json_calendar, json_event, parse_event_input, parse_event_patch, parse_override_input,
    parse_recurrence_id, CalendarRow, Db,
};
use crate::http::{html_response, json_response, text_response, Request, Response};
use crate::ics::{render_calendar, IcsEvent, IcsOverride};
use crate::json::{parse as parse_json, stringify, Value};
use crate::landing::{html_created, html_home, json_index, llms_txt};
use crate::time::now_iso;

const OG_PNG: &[u8] = include_bytes!("../public/og.png");
const MASCOT_WEBP: &[u8] = include_bytes!("../public/mascot.webp");

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<Db>>,
    pub public_base: String,
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
        ("GET", "/health") => json_response(
            200,
            &stringify(&Value::object(&[("ok", Value::Bool(true))])),
        ),
        ("GET", "/llms.txt") => {
            let base = request_base(req, &state.public_base);
            text_response(200, "text/plain; charset=utf-8", llms_txt(&base))
        }
        ("GET", "/og.png") => text_response(200, "image/png", OG_PNG.to_vec()),
        ("GET", "/mascot.webp") => text_response(200, "image/webp", MASCOT_WEBP.to_vec()),
        ("POST", "/calendars") => create_calendar(state, req),
        (m, p) if p.starts_with("/feed/") => {
            if m != "GET" {
                return json_err(404, "not found");
            }
            feed(state, &p["/feed/".len()..])
        }
        (m, p) if p.starts_with("/v1/c/") => scoped(state, req, m, &p["/v1/c/".len()..]),
        (m, "/v1/events") => home_events(state, req, m, None),
        (m, p) if let Some(rest) = p.strip_prefix("/v1/events/") => match event_path(rest) {
            EventPath::Event(uid) => home_events(state, req, m, Some(uid)),
            EventPath::Exdates(uid) => exception(state, req, None, m, uid, true),
            EventPath::Overrides(uid) => exception(state, req, None, m, uid, false),
        },
        _ => json_err(404, "not found"),
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

fn create_calendar(state: &AppState, req: &Request) -> Response {
    let ctype = req.header("content-type");
    let name = parse_calendar_name(ctype, &req.body);
    let db = state.db.lock().expect("db");
    let saved = match db.create_calendar(name.as_deref(), None, None, None) {
        Ok(row) => row,
        Err(e) => return json_err(400, &e),
    };
    let base = request_base(req, &state.public_base);
    if wants_html(req) || ctype.contains("application/x-www-form-urlencoded") {
        return html_response(201, html_created(&saved, &base));
    }
    json_response(201, &stringify(&json_calendar(&saved, &base)))
}

fn parse_calendar_name(ctype: &str, body: &[u8]) -> Option<String> {
    if ctype.contains("application/json") {
        let value = parse_json(std::str::from_utf8(body).ok()?).ok()?;
        return value
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    if ctype.contains("application/x-www-form-urlencoded") {
        let text = String::from_utf8_lossy(body);
        for pair in text.split('&') {
            let mut it = pair.splitn(2, '=');
            let key = it.next().unwrap_or("");
            let val = it.next().unwrap_or("");
            if key == "name" {
                return Some(urlencoding_decode(val));
            }
        }
    }
    None
}

fn urlencoding_decode(value: &str) -> String {
    let plus = value.replace('+', " ");
    let mut out = Vec::new();
    let bytes = plus.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&plus[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn feed(state: &AppState, token: &str) -> Response {
    let token = feed_token_of(token);
    let db = state.db.lock().expect("db");
    let cal = match db.get_calendar_by_feed_token(&token) {
        Ok(Some(cal)) if safe_equal(&token, &cal.feed_token) => cal,
        _ => return json_err(404, "not found"),
    };
    let events = match db.list_events(&cal.id) {
        Ok(rows) => rows,
        Err(e) => return json_err(500, &e),
    };
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
    text_response(
        200,
        "text/calendar; charset=utf-8",
        render_calendar(&cal.name, &ics_events),
    )
    .header("cache-control", "no-cache")
    .header("content-disposition", "inline; filename=\"calendar.ics\"")
}

fn scoped(state: &AppState, req: &Request, method: &str, rest: &str) -> Response {
    let (id, rest) = match rest.split_once('/') {
        Some((id, rest)) => (id, rest),
        None => {
            if method != "GET" {
                return json_err(404, "not found");
            }
            return calendar_info(state, req, rest);
        }
    };
    if rest == "events" {
        return match method {
            "GET" => list_events(state, req, id),
            "POST" => post_event(state, req, id),
            _ => json_err(404, "not found"),
        };
    }
    if let Some(rest) = rest.strip_prefix("events/") {
        return match event_path(rest) {
            EventPath::Event(uid) => match method {
                "GET" => get_event(state, req, id, uid),
                "PUT" => put_event(state, req, id, uid),
                "PATCH" => patch_event(state, req, id, uid),
                "DELETE" => delete_event(state, req, id, uid),
                _ => json_err(404, "not found"),
            },
            EventPath::Exdates(uid) => exception(state, req, Some(id), method, uid, true),
            EventPath::Overrides(uid) => exception(state, req, Some(id), method, uid, false),
        };
    }
    json_err(404, "not found")
}

fn calendar_info(state: &AppState, req: &Request, id: &str) -> Response {
    let db = state.db.lock().expect("db");
    let Some(cal) = calendar_auth(&db, id, req.header("authorization")) else {
        return json_err(401, "unauthorized");
    };
    let base = request_base(req, &state.public_base);
    json_response(200, &stringify(&json_calendar(&cal, &base)))
}

fn list_events(state: &AppState, req: &Request, id: &str) -> Response {
    let db = state.db.lock().expect("db");
    let Some(cal) = calendar_auth(&db, id, req.header("authorization")) else {
        return json_err(401, "unauthorized");
    };
    match db.list_events(&cal.id) {
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

fn get_event(state: &AppState, req: &Request, id: &str, uid: &str) -> Response {
    let db = state.db.lock().expect("db");
    let Some(cal) = calendar_auth(&db, id, req.header("authorization")) else {
        return json_err(401, "unauthorized");
    };
    match db.get_event(&cal.id, uid) {
        Ok(Some(row)) => json_response(200, &stringify(&json_event(&row))),
        Ok(None) => json_err(404, "not found"),
        Err(e) => json_err(500, &e),
    }
}

fn post_event(state: &AppState, req: &Request, id: &str) -> Response {
    let db = state.db.lock().expect("db");
    let Some(cal) = calendar_auth(&db, id, req.header("authorization")) else {
        return json_err(401, "unauthorized");
    };
    let body = match parse_body(req) {
        Ok(v) => v,
        Err(e) => return json_err(400, &e),
    };
    upsert_response(&db, &cal.id, &body, None)
}

fn put_event(state: &AppState, req: &Request, id: &str, uid: &str) -> Response {
    let db = state.db.lock().expect("db");
    let Some(cal) = calendar_auth(&db, id, req.header("authorization")) else {
        return json_err(401, "unauthorized");
    };
    let body = match parse_body(req) {
        Ok(v) => v,
        Err(e) => return json_err(400, &e),
    };
    upsert_response(&db, &cal.id, &body, Some(uid.to_string()))
}

fn patch_event(state: &AppState, req: &Request, id: &str, uid: &str) -> Response {
    let db = state.db.lock().expect("db");
    let Some(cal) = calendar_auth(&db, id, req.header("authorization")) else {
        return json_err(401, "unauthorized");
    };
    let body = match parse_body(req) {
        Ok(v) => v,
        Err(e) => return json_err(400, &e),
    };
    let patch = match parse_event_patch(&body) {
        Ok(p) => p,
        Err(e) => return json_err(400, &e),
    };
    match db.patch_event(&cal.id, uid, &patch) {
        Ok(row) => json_response(200, &stringify(&json_event(&row))),
        Err(e) if e == "not found" => json_err(404, &e),
        Err(e) => json_err(400, &e),
    }
}

fn delete_event(state: &AppState, req: &Request, id: &str, uid: &str) -> Response {
    let db = state.db.lock().expect("db");
    let Some(cal) = calendar_auth(&db, id, req.header("authorization")) else {
        return json_err(401, "unauthorized");
    };
    match db.delete_event(&cal.id, uid) {
        Ok(true) => Response::new(204),
        Ok(false) => json_err(404, "not found"),
        Err(e) => json_err(500, &e),
    }
}

fn home_auth(db: &Db, req: &Request) -> Option<CalendarRow> {
    let home = db.get_calendar("home").ok().flatten()?;
    if safe_equal(&bearer(req.header("authorization")), &home.agent_key) {
        Some(home)
    } else {
        None
    }
}

fn home_events(state: &AppState, req: &Request, method: &str, uid: Option<&str>) -> Response {
    let db = state.db.lock().expect("db");
    if home_auth(&db, req).is_none() {
        return json_err(401, "unauthorized");
    }
    match (method, uid) {
        ("GET", None) => match db.list_events("home") {
            Ok(rows) => json_response(
                200,
                &stringify(&Value::object(&[(
                    "events",
                    Value::Array(rows.iter().map(json_event).collect()),
                )])),
            ),
            Err(e) => json_err(500, &e),
        },
        ("POST", None) => {
            let body = match parse_body(req) {
                Ok(v) => v,
                Err(e) => return json_err(400, &e),
            };
            upsert_response(&db, "home", &body, None)
        }
        ("GET", Some(uid)) => match db.get_event("home", uid) {
            Ok(Some(row)) => json_response(200, &stringify(&json_event(&row))),
            Ok(None) => json_err(404, "not found"),
            Err(e) => json_err(500, &e),
        },
        ("PUT", Some(uid)) => {
            let body = match parse_body(req) {
                Ok(v) => v,
                Err(e) => return json_err(400, &e),
            };
            upsert_response(&db, "home", &body, Some(uid.to_string()))
        }
        ("PATCH", Some(uid)) => {
            let body = match parse_body(req) {
                Ok(v) => v,
                Err(e) => return json_err(400, &e),
            };
            let patch = match parse_event_patch(&body) {
                Ok(p) => p,
                Err(e) => return json_err(400, &e),
            };
            match db.patch_event("home", uid, &patch) {
                Ok(row) => json_response(200, &stringify(&json_event(&row))),
                Err(e) if e == "not found" => json_err(404, &e),
                Err(e) => json_err(400, &e),
            }
        }
        ("DELETE", Some(uid)) => match db.delete_event("home", uid) {
            Ok(true) => Response::new(204),
            Ok(false) => json_err(404, "not found"),
            Err(e) => json_err(500, &e),
        },
        _ => json_err(404, "not found"),
    }
}

fn upsert_response(db: &Db, calendar_id: &str, body: &Value, uid: Option<String>) -> Response {
    let mut input = match parse_event_input(body) {
        Ok(i) => i,
        Err(e) => return json_err(400, &e),
    };
    if let Some(uid) = uid {
        input.uid = Some(uid);
    }
    match db.upsert_event(calendar_id, &input) {
        Ok(saved) => {
            let status = if saved.sequence == 0 { 201 } else { 200 };
            json_response(status, &stringify(&json_event(&saved)))
        }
        Err(e) => json_err(400, &e),
    }
}

fn parse_body(req: &Request) -> Result<Value, String> {
    let text = std::str::from_utf8(&req.body).map_err(|_| "body must be an object")?;
    if text.is_empty() {
        return Err("body must be an object".into());
    }
    parse_json(text).map_err(|_| "body must be an object".into())
}

fn calendar_auth(db: &Db, id: &str, header: &str) -> Option<CalendarRow> {
    let cal = db.get_calendar(id).ok().flatten()?;
    if safe_equal(&bearer(header), &cal.agent_key) {
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
    if raw.len() >= 4 && raw.to_ascii_lowercase().ends_with(".ics") {
        raw[..raw.len() - 4].to_string()
    } else {
        raw.to_string()
    }
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

fn json_err(status: u16, msg: &str) -> Response {
    json_response(
        status,
        &stringify(&Value::object(&[("error", Value::String(msg.into()))])),
    )
}

enum EventPath<'a> {
    Event(&'a str),
    Exdates(&'a str),
    Overrides(&'a str),
}

fn event_path(rest: &str) -> EventPath<'_> {
    match rest.split_once('/') {
        Some((uid, "exdates")) => EventPath::Exdates(uid),
        Some((uid, "overrides")) => EventPath::Overrides(uid),
        Some(_) => EventPath::Event(rest),
        None => EventPath::Event(rest),
    }
}

fn exception(
    state: &AppState,
    req: &Request,
    calendar_id: Option<&str>,
    method: &str,
    uid: &str,
    exdate: bool,
) -> Response {
    if method != "PUT" && method != "DELETE" {
        return json_err(404, "not found");
    }
    let db = state.db.lock().expect("db");
    let Some(cal) = authed_calendar(&db, req, calendar_id) else {
        return json_err(401, "unauthorized");
    };
    let body = match parse_body(req) {
        Ok(v) => v,
        Err(e) => return json_err(400, &e),
    };
    let result = if exdate {
        let recurrence_id = match parse_recurrence_id(&body) {
            Ok(id) => id,
            Err(e) => return json_err(400, &e),
        };
        if method == "PUT" {
            db.put_exdate(&cal.id, uid, &recurrence_id)
                .map(|(row, created)| (row, created))
        } else {
            match db.delete_exdate(&cal.id, uid, &recurrence_id) {
                Ok(true) => return Response::new(204),
                Ok(false) => return json_err(404, "not found"),
                Err(e) if e == "not found" => return json_err(404, &e),
                Err(e) => return json_err(400, &e),
            }
        }
    } else if method == "PUT" {
        let input = match parse_override_input(&body) {
            Ok(input) => input,
            Err(e) => return json_err(400, &e),
        };
        db.put_override(&cal.id, uid, &input)
    } else {
        let recurrence_id = match parse_recurrence_id(&body) {
            Ok(id) => id,
            Err(e) => return json_err(400, &e),
        };
        match db.delete_override(&cal.id, uid, &recurrence_id) {
            Ok(true) => return Response::new(204),
            Ok(false) => return json_err(404, "not found"),
            Err(e) if e == "not found" => return json_err(404, &e),
            Err(e) => return json_err(400, &e),
        }
    };
    match result {
        Ok((row, created)) => json_response(
            if created { 201 } else { 200 },
            &stringify(&json_event(&row)),
        ),
        Err(e) if e == "not found" => json_err(404, &e),
        Err(e) => json_err(400, &e),
    }
}

fn authed_calendar(db: &Db, req: &Request, id: Option<&str>) -> Option<CalendarRow> {
    match id {
        Some(id) => calendar_auth(db, id, req.header("authorization")),
        None => home_auth(db, req),
    }
}
