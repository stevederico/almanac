//! Shared harness for the in-process HTTP tests: a scratch app, a `send` that
//! goes through `handle()`, and a way to make a calendar and call it.
#![allow(dead_code)]

use almanac::db::Db;
use almanac::http::{Request, Response};
use almanac::json::{parse, Value};
use almanac::server::{handle, AppState};
use almanac::util::{hex_encode, random_bytes};

pub const HOME_KEY: &str = "home-secret";
pub const HOME_FEED: &str = "home-feed";
pub const BASE: &str = "http://example.test";

pub fn app() -> AppState {
    let dir = std::env::temp_dir().join(format!("almanac-it-{}", hex_encode(&random_bytes(8))));
    std::fs::create_dir_all(&dir).unwrap();
    let db = Db::open(dir.join("calendar.db")).unwrap();
    db.ensure_home_calendar(HOME_FEED, HOME_KEY, "Home").unwrap();
    let mut app = AppState::new(db, BASE.into());
    // Creating calendars in these tests must not trip the create limit.
    app.limits.creates_per_client_hour = 1000;
    app
}

pub fn send(app: &AppState, req: Request) -> (u16, String, Response) {
    let res = handle(app, &req);
    (res.status, String::from_utf8_lossy(&res.body).into_owned(), res)
}

pub fn json(body: &str) -> Value {
    parse(body).unwrap_or_else(|e| panic!("not json ({e}): {body}"))
}

pub struct Cal {
    pub id: String,
    pub key: String,
    pub events_feed_path: String,
    pub todos_feed_path: String,
    pub notes_feed_path: String,
}

pub fn create(app: &AppState) -> Cal {
    let (status, body, _) = send(
        app,
        Request::new("POST", "/calendars")
            .with_header("content-type", "application/json")
            .with_header("accept", "application/json")
            .with_body(br#"{"name":"Trip","feed":"plain"}"#.to_vec()),
    );
    assert_eq!(status, 201, "{body}");
    let v = json(&body);
    let path = |v: &Value| v.as_str().unwrap().strip_prefix(BASE).unwrap().to_string();
    Cal {
        id: text(&v, "id").to_string(),
        key: text(&v, "key").to_string(),
        events_feed_path: path(field(&v, "subscribe")),
        todos_feed_path: path(field(field(&v, "todos"), "subscribe")),
        notes_feed_path: path(field(field(&v, "notes"), "subscribe")),
    }
}

pub fn call(app: &AppState, key: &str, method: &str, path: &str, body: Option<&str>) -> (u16, Value) {
    let mut req = Request::new(method, path).with_header("authorization", &format!("Bearer {key}"));
    if let Some(body) = body {
        req = req
            .with_header("content-type", "application/json")
            .with_body(body.as_bytes().to_vec());
    }
    let (status, text, _) = send(app, req);
    let value = if text.is_empty() { Value::Null } else { json(&text) };
    (status, value)
}

pub fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    v.get(key).unwrap_or_else(|| panic!("no {key} in {v:?}"))
}

pub fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    field(v, key)
        .as_str()
        .unwrap_or_else(|| panic!("{key} is not a string in {v:?}"))
}

/// The `title` of every item under `list_key` (`"todos"` or `"notes"`).
pub fn titles(v: &Value, list_key: &str) -> Vec<String> {
    field(v, list_key)
        .as_array()
        .unwrap()
        .iter()
        .map(|t| text(t, "title").to_string())
        .collect()
}
