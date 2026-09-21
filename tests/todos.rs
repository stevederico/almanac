//! Todos over the HTTP surface, in process through `handle()`.

use almanac::db::Db;
use almanac::http::{Request, Response};
use almanac::json::{parse, Value};
use almanac::server::{handle, AppState};
use almanac::util::{hex_encode, random_bytes};

const HOME_KEY: &str = "home-secret";
const HOME_FEED: &str = "home-feed";
const BASE: &str = "http://example.test";

fn app() -> AppState {
    let dir = std::env::temp_dir().join(format!("almanac-todos-{}", hex_encode(&random_bytes(8))));
    std::fs::create_dir_all(&dir).unwrap();
    let db = Db::open(dir.join("calendar.db")).unwrap();
    db.ensure_home_calendar(HOME_FEED, HOME_KEY, "Home").unwrap();
    let mut app = AppState::new(db, BASE.into());
    // Creating calendars in these tests must not trip the create limit.
    app.limits.creates_per_client_hour = 1000;
    app
}

fn send(app: &AppState, req: Request) -> (u16, String, Response) {
    let res = handle(app, &req);
    (res.status, String::from_utf8_lossy(&res.body).into_owned(), res)
}

fn json(body: &str) -> Value {
    parse(body).unwrap_or_else(|e| panic!("not json ({e}): {body}"))
}

struct Cal {
    id: String,
    key: String,
    todos_feed_path: String,
}

fn create(app: &AppState) -> Cal {
    let (status, body, _) = send(
        app,
        Request::new("POST", "/calendars")
            .with_header("content-type", "application/json")
            .with_header("accept", "application/json")
            .with_body(b"{\"name\":\"Trip\"}".to_vec()),
    );
    assert_eq!(status, 201, "{body}");
    let v = json(&body);
    let todos = v.get("todos").expect("create response lists the todos endpoints");
    let subscribe = todos.get("subscribe").and_then(Value::as_str).unwrap();
    Cal {
        id: v.get("id").and_then(Value::as_str).unwrap().to_string(),
        key: v.get("key").and_then(Value::as_str).unwrap().to_string(),
        todos_feed_path: subscribe.strip_prefix(BASE).unwrap().to_string(),
    }
}

fn call(app: &AppState, key: &str, method: &str, path: &str, body: Option<&str>) -> (u16, Value) {
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

fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    v.get(key).unwrap_or_else(|| panic!("no {key} in {v:?}"))
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    field(v, key).as_str().unwrap_or_else(|| panic!("{key} is not a string in {v:?}"))
}

fn titles(v: &Value) -> Vec<String> {
    field(v, "todos")
        .as_array()
        .unwrap()
        .iter()
        .map(|t| text(t, "title").to_string())
        .collect()
}

// ---- CRUD -------------------------------------------------------------------

#[test]
fn creates_reads_updates_and_deletes_a_todo() {
    let app = app();
    let cal = create(&app);
    let coll = format!("/v1/c/{}/todos", cal.id);

    let (status, v) = call(&app, &cal.key, "POST", &coll, Some(r#"{"title":"Buy milk","priority":5,"tags":["Home"]}"#));
    assert_eq!(status, 201);
    let uid = text(&v, "uid").to_string();
    assert!(uid.starts_with("todo-"), "{uid}");
    assert_eq!(text(&v, "title"), "Buy milk");
    assert_eq!(field(&v, "done"), &Value::Bool(false));
    assert_eq!(field(&v, "due"), &Value::Null);
    assert_eq!(field(&v, "completedAt"), &Value::Null);
    assert_eq!(field(&v, "priority"), &Value::Number(5));

    let item = format!("{coll}/{uid}");
    let (status, got) = call(&app, &cal.key, "GET", &item, None);
    assert_eq!(status, 200);
    assert_eq!(text(&got, "title"), "Buy milk");

    let (status, list) = call(&app, &cal.key, "GET", &coll, None);
    assert_eq!(status, 200);
    assert_eq!(titles(&list), ["Buy milk"]);

    let (status, patched) = call(&app, &cal.key, "PATCH", &item, Some(r#"{"due":"2026-10-01"}"#));
    assert_eq!(status, 200);
    assert_eq!(text(&patched, "due"), "2026-10-01");
    assert_eq!(text(&patched, "title"), "Buy milk", "untouched fields stay");

    assert_eq!(call(&app, &cal.key, "DELETE", &item, None).0, 204);
    assert_eq!(call(&app, &cal.key, "GET", &item, None).0, 404);
    assert_eq!(call(&app, &cal.key, "DELETE", &item, None).0, 404);
    assert_eq!(call(&app, &cal.key, "PATCH", &item, Some(r#"{"done":true}"#)).0, 404);
}

#[test]
fn put_is_idempotent_and_keeps_fields_it_does_not_mention() {
    let app = app();
    let cal = create(&app);
    let item = format!("/v1/c/{}/todos/pack-bags", cal.id);

    let (status, v) = call(
        &app,
        &cal.key,
        "PUT",
        &item,
        Some(r#"{"title":"Pack","description":"passport","due":"2026-10-01T09:00:00-07:00","priority":1}"#),
    );
    assert_eq!(status, 201);
    assert_eq!(text(&v, "uid"), "pack-bags");
    assert_eq!(text(&v, "due"), "2026-10-01T16:00:00.000Z");

    let (status, v) = call(&app, &cal.key, "PUT", &item, Some(r#"{"title":"Pack bags"}"#));
    assert_eq!(status, 200, "same uid replaces in place");
    assert_eq!(text(&v, "title"), "Pack bags");
    assert_eq!(text(&v, "description"), "passport");
    assert_eq!(field(&v, "priority"), &Value::Number(1));

    // Clearing is explicit.
    let (_, v) = call(&app, &cal.key, "PUT", &item, Some(r#"{"title":"Pack bags","due":null,"description":""}"#));
    assert_eq!(field(&v, "due"), &Value::Null);
    assert_eq!(text(&v, "description"), "");

    let (_, list) = call(&app, &cal.key, "GET", &format!("/v1/c/{}/todos", cal.id), None);
    assert_eq!(titles(&list), ["Pack bags"], "still one todo");
}

#[test]
fn finishing_stamps_the_time_and_reopening_clears_it() {
    let app = app();
    let cal = create(&app);
    let item = format!("/v1/c/{}/todos/t1", cal.id);
    call(&app, &cal.key, "PUT", &item, Some(r#"{"title":"Do it"}"#));

    let (_, done) = call(&app, &cal.key, "PATCH", &item, Some(r#"{"done":true}"#));
    assert_eq!(field(&done, "done"), &Value::Bool(true));
    let stamped = text(&done, "completedAt").to_string();
    assert!(stamped.ends_with('Z'), "{stamped}");

    // Saving it again while still done keeps the original time.
    let (_, again) = call(&app, &cal.key, "PUT", &item, Some(r#"{"title":"Do it well"}"#));
    assert_eq!(text(&again, "completedAt"), stamped);

    let (_, open) = call(&app, &cal.key, "PATCH", &item, Some(r#"{"done":false}"#));
    assert_eq!(field(&open, "completedAt"), &Value::Null);
}

// ---- validation -------------------------------------------------------------

#[test]
fn rejects_bad_input_with_a_400() {
    let app = app();
    let cal = create(&app);
    let coll = format!("/v1/c/{}/todos", cal.id);
    for bad in [
        "{}",
        r#"{"title":""}"#,
        r#"{"title":5}"#,
        r#"{"title":"x","priority":10}"#,
        r#"{"title":"x","priority":"high"}"#,
        r#"{"title":"x","done":"yes"}"#,
        r#"{"title":"x","due":"tomorrow"}"#,
        r#"{"title":"x","tags":["has space"]}"#,
        r#"{"title":"x","description":7}"#,
        "[]",
        "not json",
    ] {
        assert_eq!(call(&app, &cal.key, "POST", &coll, Some(bad)).0, 400, "{bad}");
    }
    assert_eq!(call(&app, &cal.key, "PUT", &format!("{coll}/bad%20uid"), Some(r#"{"title":"x"}"#)).0, 400);
    assert_eq!(call(&app, &cal.key, "PATCH", &format!("{coll}/x"), Some("{}")).0, 400);
    assert_eq!(call(&app, &cal.key, "GET", &format!("{coll}?status=nope"), None).0, 400);
    let (_, list) = call(&app, &cal.key, "GET", &coll, None);
    assert!(titles(&list).is_empty(), "nothing was created");
}

#[test]
fn wrong_methods_and_paths_are_404_even_without_a_key() {
    let app = app();
    let cal = create(&app);
    let coll = format!("/v1/c/{}/todos", cal.id);
    for (m, p) in [
        ("PUT", coll.clone()),
        ("DELETE", coll.clone()),
        ("PATCH", coll.clone()),
        ("POST", format!("{coll}/x")),
        ("GET", format!("{coll}/a/b")),
        ("GET", format!("{coll}/")),
    ] {
        let (status, _, _) = send(&app, Request::new(m, &p));
        assert_eq!(status, 404, "{m} {p}");
    }
}

// ---- listing ----------------------------------------------------------------

#[test]
fn lists_open_first_then_by_due_with_undated_last_and_filters() {
    let app = app();
    let cal = create(&app);
    let coll = format!("/v1/c/{}/todos", cal.id);
    for body in [
        r#"{"title":"later","due":"2026-12-01","tags":["home"]}"#,
        r#"{"title":"undated"}"#,
        r#"{"title":"soon","due":"2026-10-01","tags":["home","work"]}"#,
        r#"{"title":"finished","due":"2026-09-01","done":true,"tags":["work"]}"#,
    ] {
        assert_eq!(call(&app, &cal.key, "POST", &coll, Some(body)).0, 201);
    }
    let list = |q: &str| titles(&call(&app, &cal.key, "GET", &format!("{coll}{q}"), None).1);
    assert_eq!(list(""), ["soon", "later", "undated", "finished"]);
    assert_eq!(list("?status=open"), ["soon", "later", "undated"]);
    assert_eq!(list("?status=done"), ["finished"]);
    assert_eq!(list("?status=all"), ["soon", "later", "undated", "finished"]);
    assert_eq!(list("?tag=work"), ["soon", "finished"]);
    assert_eq!(list("?tag=WORK&status=open"), ["soon"]);
    assert_eq!(list("?tag=none"), Vec::<String>::new());
}

// ---- one key, isolation -----------------------------------------------------

#[test]
fn one_key_writes_events_and_todos_and_nothing_crosses_calendars() {
    let app = app();
    let a = create(&app);
    let b = create(&app);

    let event = format!("/v1/c/{}/events/e1", a.id);
    let todo = format!("/v1/c/{}/todos/t1", a.id);
    let ev = r#"{"summary":"Dentist","start":"2026-10-01T09:00:00-07:00"}"#;
    assert_eq!(call(&app, &a.key, "PUT", &event, Some(ev)).0, 201);
    assert_eq!(call(&app, &a.key, "PUT", &todo, Some(r#"{"title":"Floss"}"#)).0, 201);

    // A's key is nothing on B, for either resource.
    for path in [format!("/v1/c/{}/todos", b.id), format!("/v1/c/{}/events", b.id)] {
        assert_eq!(call(&app, &a.key, "GET", &path, None).0, 401, "{path}");
    }
    assert_eq!(call(&app, &a.key, "PUT", &format!("/v1/c/{}/todos/t1", b.id), Some(r#"{"title":"x"}"#)).0, 401);
    assert_eq!(call(&app, "", "GET", &format!("/v1/c/{}/todos", a.id), None).0, 401);
    assert_eq!(call(&app, "wrong", "GET", &format!("/v1/c/{}/todos", a.id), None).0, 401);

    // B sees none of A's todos even with its own key.
    let (_, list) = call(&app, &b.key, "GET", &format!("/v1/c/{}/todos", b.id), None);
    assert!(titles(&list).is_empty());
    // Same uid on two calendars is two todos.
    assert_eq!(call(&app, &b.key, "PUT", &format!("/v1/c/{}/todos/t1", b.id), Some(r#"{"title":"B's"}"#)).0, 201);
    assert_eq!(text(&call(&app, &a.key, "GET", &todo, None).1, "title"), "Floss");
}

#[test]
fn the_home_calendar_has_short_todo_routes() {
    let app = app();
    assert_eq!(call(&app, HOME_KEY, "PUT", "/v1/todos/h1", Some(r#"{"title":"Mine"}"#)).0, 201);
    let (status, list) = call(&app, HOME_KEY, "GET", "/v1/todos", None);
    assert_eq!(status, 200);
    assert_eq!(titles(&list), ["Mine"]);
    assert_eq!(text(&call(&app, HOME_KEY, "GET", "/v1/c/home/todos/h1", None).1, "title"), "Mine");
    assert_eq!(call(&app, "wrong", "GET", "/v1/todos", None).0, 401);
    // The home owner reads the feed URLs from the calendar info.
    let (_, info) = call(&app, HOME_KEY, "GET", "/v1/c/home", None);
    assert!(text(field(&info, "todos"), "subscribe").contains("/feed/"));
    assert!(info.get("key").is_none());
}

// ---- limits -----------------------------------------------------------------

#[test]
fn the_todo_cap_holds_for_strangers_but_not_for_home_or_replacements() {
    let mut app = app();
    app.limits.max_todos = 2;
    let cal = create(&app);
    let coll = format!("/v1/c/{}/todos", cal.id);
    assert_eq!(call(&app, &cal.key, "PUT", &format!("{coll}/a"), Some(r#"{"title":"a"}"#)).0, 201);
    assert_eq!(call(&app, &cal.key, "POST", &coll, Some(r#"{"title":"b"}"#)).0, 201);
    assert_eq!(call(&app, &cal.key, "POST", &coll, Some(r#"{"title":"c"}"#)).0, 409);
    assert_eq!(call(&app, &cal.key, "PUT", &format!("{coll}/c"), Some(r#"{"title":"c"}"#)).0, 409);
    // Replacing one that exists is not adding.
    assert_eq!(call(&app, &cal.key, "PUT", &format!("{coll}/a"), Some(r#"{"title":"a2"}"#)).0, 200);
    // Freeing a slot frees the cap.
    assert_eq!(call(&app, &cal.key, "DELETE", &format!("{coll}/a"), None).0, 204);
    assert_eq!(call(&app, &cal.key, "POST", &coll, Some(r#"{"title":"c"}"#)).0, 201);

    for i in 0..5 {
        let path = format!("/v1/todos/h{i}");
        assert_eq!(call(&app, HOME_KEY, "PUT", &path, Some(r#"{"title":"h"}"#)).0, 201);
    }
}

#[test]
fn events_and_todos_spend_one_write_budget() {
    let mut app = app();
    app.limits.writes_per_minute = 2;
    let cal = create(&app);
    let ev = r#"{"summary":"x","start":"2026-10-01"}"#;
    assert_eq!(call(&app, &cal.key, "PUT", &format!("/v1/c/{}/events/e", cal.id), Some(ev)).0, 201);
    assert_eq!(call(&app, &cal.key, "PUT", &format!("/v1/c/{}/todos/t", cal.id), Some(r#"{"title":"x"}"#)).0, 201);
    let (status, _, res) = send(
        &app,
        Request::new("PUT", &format!("/v1/c/{}/todos/t2", cal.id))
            .with_header("authorization", &format!("Bearer {}", cal.key))
            .with_body(br#"{"title":"y"}"#.to_vec()),
    );
    assert_eq!(status, 429);
    assert!(res.header_value("retry-after").is_some());
    // Reads stay free.
    assert_eq!(call(&app, &cal.key, "GET", &format!("/v1/c/{}/todos", cal.id), None).0, 200);
    // A stranger cannot spend the owner's budget.
    assert_eq!(call(&app, "wrong", "PUT", &format!("/v1/c/{}/todos/t3", cal.id), Some(r#"{"title":"z"}"#)).0, 401);
}

// ---- feeds ------------------------------------------------------------------

#[test]
fn the_todos_feed_holds_only_todos_and_the_calendar_feed_only_events() {
    let app = app();
    let cal = create(&app);
    let ev = r#"{"summary":"Dentist","start":"2026-10-01T09:00:00-07:00"}"#;
    call(&app, &cal.key, "PUT", &format!("/v1/c/{}/events/e1", cal.id), Some(ev));
    call(
        &app,
        &cal.key,
        "PUT",
        &format!("/v1/c/{}/todos/t1", cal.id),
        Some(r#"{"title":"Buy milk; eggs","due":"2026-10-02","priority":3,"tags":["home"]}"#),
    );
    call(&app, &cal.key, "PUT", &format!("/v1/c/{}/todos/t2", cal.id), Some(r#"{"title":"Done one","done":true}"#));

    let (status, ics, res) = send(&app, Request::new("GET", &cal.todos_feed_path));
    assert_eq!(status, 200, "{ics}");
    assert!(res.header_value("content-type").unwrap().starts_with("text/calendar"));
    assert_eq!(ics.matches("BEGIN:VTODO").count(), 2);
    assert!(!ics.contains("VEVENT"));
    assert!(ics.contains("SUMMARY:Buy milk\\; eggs\r\n"), "{ics}");
    assert!(ics.contains("DUE;VALUE=DATE:20261002\r\n"));
    assert!(ics.contains("PRIORITY:3\r\n"));
    assert!(ics.contains("CATEGORIES:home\r\n"));
    assert!(ics.contains("STATUS:COMPLETED\r\n"));
    assert!(ics.contains("STATUS:NEEDS-ACTION\r\n"));
    assert!(ics.ends_with("END:VCALENDAR\r\n"));

    // The events feed (from the create response) is unchanged and has no todos.
    let (_, info) = call(&app, &cal.key, "GET", &format!("/v1/c/{}", cal.id), None);
    let events_feed = text(&info, "subscribe").strip_prefix(BASE).unwrap().to_string();
    let (status, events_ics, _) = send(&app, Request::new("GET", &events_feed));
    assert_eq!(status, 200);
    assert!(events_ics.contains("VEVENT") && !events_ics.contains("VTODO"));
    assert_ne!(events_feed, cal.todos_feed_path, "each feed has its own token");
}

#[test]
fn feed_tokens_are_stable_unguessable_and_per_calendar() {
    let app = app();
    let a = create(&app);
    let b = create(&app);
    assert_ne!(a.todos_feed_path, b.todos_feed_path);
    let (_, info) = call(&app, &a.key, "GET", &format!("/v1/c/{}", a.id), None);
    let again = text(field(&info, "todos"), "subscribe");
    assert_eq!(again, format!("{BASE}{}", a.todos_feed_path), "asking again must not mint a new token");

    for path in ["/feed/nope.ics", "/feed/.ics", "/feed/", &format!("{}x", a.todos_feed_path)] {
        assert_eq!(send(&app, Request::new("GET", path)).0, 404, "{path}");
    }
    // Only reads are allowed on a feed.
    assert_eq!(send(&app, Request::new("POST", &a.todos_feed_path)).0, 404);
    let (status, body, res) = send(&app, Request::new("HEAD", &a.todos_feed_path));
    assert_eq!(status, 200);
    assert!(body.is_empty());
    assert!(res.header_value("content-length").unwrap().parse::<usize>().unwrap() > 0);
}

#[test]
fn a_calendar_that_predates_todos_gets_a_feed_on_first_ask() {
    let app = app();
    let (status, info) = call(&app, HOME_KEY, "GET", "/v1/c/home", None);
    assert_eq!(status, 200);
    let path = text(field(&info, "todos"), "subscribe").strip_prefix(BASE).unwrap().to_string();
    assert_eq!(send(&app, Request::new("GET", &path)).0, 200);
    // The home calendar's own event feed keeps working with its env token.
    assert_eq!(send(&app, Request::new("GET", &format!("/feed/{HOME_FEED}.ics"))).0, 200);
}

#[test]
fn a_title_cannot_inject_ics_properties_into_the_feed() {
    let app = app();
    let cal = create(&app);
    call(
        &app,
        &cal.key,
        "PUT",
        &format!("/v1/c/{}/todos/t", cal.id),
        Some(r#"{"title":"a\r\nSTATUS:COMPLETED\r\nEND:VTODO","description":"x\nBEGIN:VTODO"}"#),
    );
    let (_, ics, _) = send(&app, Request::new("GET", &cal.todos_feed_path));
    let lines: Vec<&str> = ics.split("\r\n").collect();
    assert_eq!(lines.iter().filter(|l| **l == "BEGIN:VTODO").count(), 1, "{ics}");
    assert_eq!(lines.iter().filter(|l| l.starts_with("STATUS:")).count(), 1, "{ics}");
    assert_eq!(lines.iter().filter(|l| **l == "END:VTODO").count(), 1, "{ics}");
}
