use std::sync::{Arc, Mutex};

use almanac::db::Db;
use almanac::http::Request;
use almanac::json::{parse, Value};
use almanac::server::{handle, AppState};
use almanac::util::{hex_encode, random_bytes};

const FEED: &str = "feed-secret";
const KEY: &str = "agent-secret";
const BASE: &str = "http://example.test";

fn test_app() -> AppState {
    let dir = std::env::temp_dir().join(format!("almanac-srv-{}", hex_encode(&random_bytes(8))));
    std::fs::create_dir_all(&dir).unwrap();
    let db = Db::open(dir.join("calendar.db")).unwrap();
    db.ensure_home_calendar(FEED, KEY, "My Calendar").unwrap();
    AppState {
        db: Arc::new(Mutex::new(db)),
        public_base: BASE.into(),
    }
}

fn send(state: &AppState, req: Request) -> (u16, Vec<u8>, almanac::http::Response) {
    let res = handle(state, &req);
    (res.status, res.body.clone(), res)
}

fn auth_get(path: &str) -> Request {
    Request::new("GET", path).with_header("authorization", &format!("Bearer {KEY}"))
}

#[test]
fn returns_json_for_agents() {
    let (status, body, _) = send(
        &test_app(),
        Request::new("GET", "/").with_header("accept", "application/json"),
    );
    assert_eq!(status, 200);
    let v = parse(std::str::from_utf8(&body).unwrap()).unwrap();
    assert!(v.get("create").is_some());
}

#[test]
fn returns_html_for_browsers() {
    let (status, body, _) = send(
        &test_app(),
        Request::new("GET", "/").with_header("accept", "text/html"),
    );
    assert_eq!(status, 200);
    let html = String::from_utf8(body).unwrap();
    assert!(html.contains("An Agent Calendar"));
    assert!(html.contains("Send your agent here"));
    assert!(html.contains("Copy Prompt"));
    assert!(html.contains("/llms.txt"));
    assert!(html.contains("/calendars"));
    assert!(html.contains("Subscribe from web"));
    assert!(html.contains("Do not add events unless I ask"));
    assert!(html.contains("og.png"));
    assert!(html.contains("og:image"));
    assert!(html.contains("twitter:card"));
    assert!(!html.contains("Create Calendar"));
}

#[test]
fn requires_a_user_agent_in_machine_docs() {
    let (status, body, _) = send(&test_app(), Request::new("GET", "/llms.txt"));
    assert_eq!(status, 200);
    let text = String::from_utf8(body).unwrap();
    assert!(text.contains("User-Agent"));
}

#[test]
fn creates_a_calendar_and_returns_subscribe_and_key() {
    let (status, body, _) = send(
        &test_app(),
        Request::new("POST", "/calendars")
            .with_header("content-type", "application/json")
            .with_header("accept", "application/json")
            .with_body(br#"{"name":"Giants"}"#.to_vec()),
    );
    assert_eq!(status, 201);
    let v = parse(std::str::from_utf8(&body).unwrap()).unwrap();
    assert!(v
        .get("id")
        .and_then(Value::as_str)
        .unwrap()
        .starts_with("cal_"));
    assert!(v.get("key").and_then(Value::as_str).is_some());
    let sub = v.get("subscribe").and_then(Value::as_str).unwrap();
    assert!(sub.starts_with("http://example.test/feed/"));
    assert!(sub.ends_with(".ics"));
}

#[test]
fn isolates_two_calendars() {
    let app = test_app();
    let mk = || {
        Request::new("POST", "/calendars")
            .with_header("content-type", "application/json")
            .with_header("accept", "application/json")
            .with_body(b"{}".to_vec())
    };
    let (_, a_body, _) = send(&app, mk());
    let (_, b_body, _) = send(&app, mk());
    let a = parse(std::str::from_utf8(&a_body).unwrap()).unwrap();
    let b = parse(std::str::from_utf8(&b_body).unwrap()).unwrap();
    let a_id = a.get("id").and_then(Value::as_str).unwrap();
    let a_key = a.get("key").and_then(Value::as_str).unwrap();
    let b_id = b.get("id").and_then(Value::as_str).unwrap();
    let b_key = b.get("key").and_then(Value::as_str).unwrap();

    let (put_status, _, _) = send(
        &app,
        Request::new("PUT", &format!("/v1/c/{a_id}/events/only-a"))
            .with_header("authorization", &format!("Bearer {a_key}"))
            .with_header("content-type", "application/json")
            .with_body(br#"{"summary":"Only A","start":"2026-08-16T12:00:00Z"}"#.to_vec()),
    );
    assert_eq!(put_status, 201);

    let (_, listed, _) = send(
        &app,
        Request::new("GET", &format!("/v1/c/{b_id}/events"))
            .with_header("authorization", &format!("Bearer {b_key}")),
    );
    let listed = parse(std::str::from_utf8(&listed).unwrap()).unwrap();
    assert_eq!(
        listed
            .get("events")
            .and_then(Value::as_array)
            .unwrap()
            .len(),
        0
    );

    let (steal, _, _) = send(
        &app,
        Request::new("GET", &format!("/v1/c/{a_id}/events"))
            .with_header("authorization", &format!("Bearer {b_key}")),
    );
    assert_eq!(steal, 401);
}

#[test]
fn feed_404s_a_wrong_token() {
    let (status, _, _) = send(&test_app(), Request::new("GET", "/feed/nope.ics"));
    assert_eq!(status, 404);
}

#[test]
fn returns_text_calendar_for_the_home_token() {
    let app = test_app();
    let _ = send(
        &app,
        Request::new("POST", "/v1/events")
            .with_header("authorization", &format!("Bearer {KEY}"))
            .with_header("content-type", "application/json")
            .with_body(
                br#"{"uid":"giants-20260816","summary":"Rockies @ Giants","start":"2026-08-16T13:05:00-07:00","location":"Oracle Park"}"#
                    .to_vec(),
            ),
    );
    let (status, body, res) = send(&app, Request::new("GET", &format!("/feed/{FEED}.ics")));
    assert_eq!(status, 200);
    let ctype = res.header_value("content-type").unwrap_or("");
    assert!(ctype.contains("text/calendar"));
    let text = String::from_utf8(body).unwrap();
    assert!(text.contains("BEGIN:VCALENDAR"));
    assert!(text.contains("UID:giants-20260816"));
    assert!(text.contains("SUMMARY:Rockies @ Giants"));
}

#[test]
fn rejects_missing_bearer() {
    let (status, _, _) = send(&test_app(), Request::new("GET", "/v1/events"));
    assert_eq!(status, 401);
}

#[test]
fn creates_lists_patches_deletes() {
    let app = test_app();
    let (created_status, created_body, _) = send(
        &app,
        Request::new("POST", "/v1/events")
            .with_header("authorization", &format!("Bearer {KEY}"))
            .with_header("content-type", "application/json")
            .with_body(
                br#"{"summary":"Dentist","start":"2026-08-18T09:00:00-07:00","end":"2026-08-18T09:45:00-07:00"}"#
                    .to_vec(),
            ),
    );
    assert_eq!(created_status, 201);
    let created = parse(std::str::from_utf8(&created_body).unwrap()).unwrap();
    let uid = created.get("uid").and_then(Value::as_str).unwrap();

    let (list_status, list_body, _) = send(&app, auth_get("/v1/events"));
    assert_eq!(list_status, 200);
    let listed = parse(std::str::from_utf8(&list_body).unwrap()).unwrap();
    assert_eq!(
        listed
            .get("events")
            .and_then(Value::as_array)
            .unwrap()
            .len(),
        1
    );

    let (patched, _, _) = send(
        &app,
        Request::new("PATCH", &format!("/v1/events/{uid}"))
            .with_header("authorization", &format!("Bearer {KEY}"))
            .with_header("content-type", "application/json")
            .with_body(br#"{"location":"Fillmore"}"#.to_vec()),
    );
    assert_eq!(patched, 200);

    let (gone, _, _) = send(
        &app,
        Request::new("DELETE", &format!("/v1/events/{uid}"))
            .with_header("authorization", &format!("Bearer {KEY}")),
    );
    assert_eq!(gone, 204);
}

#[test]
fn upserts_by_uid_via_put() {
    let app = test_app();
    let (first, _, _) = send(
        &app,
        Request::new("PUT", "/v1/events/standup")
            .with_header("authorization", &format!("Bearer {KEY}"))
            .with_header("content-type", "application/json")
            .with_body(br#"{"summary":"Standup","start":"2026-08-14T09:30:00-07:00"}"#.to_vec()),
    );
    assert_eq!(first, 201);
    let (again, body, _) = send(
        &app,
        Request::new("PUT", "/v1/events/standup")
            .with_header("authorization", &format!("Bearer {KEY}"))
            .with_header("content-type", "application/json")
            .with_body(
                br#"{"summary":"Standup moved","start":"2026-08-14T10:00:00-07:00"}"#.to_vec(),
            ),
    );
    assert_eq!(again, 200);
    let v = parse(std::str::from_utf8(&body).unwrap()).unwrap();
    assert_eq!(v.get("sequence"), Some(&Value::Number(1)));
}
