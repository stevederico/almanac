//! `GET /v1/c/{id}/export`: everything in a calendar, out through the one key.

mod common;

use almanac::http::Request;
use almanac::json::Value;
use common::*;

fn fill(app: &almanac::server::AppState, cal: &Cal) {
    let ev = r#"{"summary":"Dentist","start":"2026-10-01T09:00:00-07:00","location":"Fillmore"}"#;
    assert_eq!(call(app, &cal.key, "PUT", &format!("/v1/c/{}/events/e1", cal.id), Some(ev)).0, 201);
    let todo = r#"{"title":"Buy milk","due":"2026-10-02","priority":2,"tags":["home"]}"#;
    assert_eq!(call(app, &cal.key, "PUT", &format!("/v1/c/{}/todos/t1", cal.id), Some(todo)).0, 201);
    let note = r##"{"title":"Ideas","body":"# Ideas\n- one","tags":["work"],"pinned":true}"##;
    assert_eq!(call(app, &cal.key, "PUT", &format!("/v1/c/{}/notes/n1", cal.id), Some(note)).0, 201);
}

#[test]
fn json_export_holds_all_three_and_no_secrets() {
    let app = app();
    let cal = create(&app);
    fill(&app, &cal);

    let (status, body, res) = send(
        &app,
        Request::new("GET", &format!("/v1/c/{}/export", cal.id))
            .with_header("authorization", &format!("Bearer {}", cal.key)),
    );
    assert_eq!(status, 200, "{body}");
    assert!(res.header_value("content-type").unwrap().starts_with("application/json"));
    assert!(res.header_value("content-disposition").unwrap().contains(&format!("almanac-{}.json", cal.id)));
    assert_eq!(res.header_value("cache-control"), Some("no-store"));

    let v = json(&body);
    assert_eq!(text(field(&v, "calendar"), "id"), cal.id);
    assert_eq!(text(field(&v, "calendar"), "name"), "Trip");
    assert_eq!(text(&field(&v, "events").as_array().unwrap()[0], "summary"), "Dentist");
    assert_eq!(text(&field(&v, "todos").as_array().unwrap()[0], "title"), "Buy milk");
    let note = &field(&v, "notes").as_array().unwrap()[0];
    assert_eq!(text(note, "body"), "# Ideas\n- one", "an export carries note bodies");

    // The tokens in the feed URLs and the key are not part of a backup.
    for path in [&cal.events_feed_path, &cal.todos_feed_path, &cal.notes_feed_path] {
        let token = path.rsplit('/').next().unwrap().split('.').next().unwrap();
        assert!(!body.contains(token), "feed token leaked into the export");
    }
    assert!(!body.contains(&cal.key));
    assert!(!body.contains("subscribe"));
}

#[test]
fn markdown_export_reads_as_a_document() {
    let app = app();
    let cal = create(&app);
    fill(&app, &cal);
    let (status, body, res) = send(
        &app,
        Request::new("GET", &format!("/v1/c/{}/export?format=md", cal.id))
            .with_header("authorization", &format!("Bearer {}", cal.key)),
    );
    assert_eq!(status, 200);
    assert!(res.header_value("content-type").unwrap().starts_with("text/markdown"));
    assert!(res.header_value("content-disposition").unwrap().contains(".md\""));
    assert!(body.starts_with("# Trip\n"), "{body}");
    assert!(body.contains("Dentist (Fillmore)"), "{body}");
    assert!(body.contains("- [ ] Buy milk — due 2026-10-02 · priority 2 · #home"), "{body}");
    assert!(body.contains("### Ideas") && body.contains("# Ideas\n- one"), "{body}");
}

#[test]
fn an_export_needs_the_key_and_stays_inside_its_calendar() {
    let app = app();
    let a = create(&app);
    let b = create(&app);
    fill(&app, &a);

    let path = format!("/v1/c/{}/export", a.id);
    assert_eq!(send(&app, Request::new("GET", &path)).0, 401);
    assert_eq!(call(&app, "wrong", "GET", &path, None).0, 401);
    assert_eq!(call(&app, &b.key, "GET", &path, None).0, 401, "B's key is nothing on A");
    let (_, v) = call(&app, &b.key, "GET", &format!("/v1/c/{}/export", b.id), None);
    assert!(field(&v, "events").as_array().unwrap().is_empty());
    assert!(field(&v, "notes").as_array().unwrap().is_empty());

    // Only GET; a bad format is refused before anything is read.
    assert_eq!(send(&app, Request::new("POST", &path)).0, 404);
    assert_eq!(call(&app, &a.key, "GET", &format!("{path}?format=xml"), None).0, 400);
}

#[test]
fn exports_are_metered_but_the_home_calendar_is_exempt() {
    let mut app = app();
    app.limits.exports_per_hour = 2;
    let cal = create(&app);
    let path = format!("/v1/c/{}/export", cal.id);
    assert_eq!(call(&app, &cal.key, "GET", &path, None).0, 200);
    assert_eq!(call(&app, &cal.key, "GET", &path, None).0, 200);
    let (status, _, res) = send(
        &app,
        Request::new("GET", &path).with_header("authorization", &format!("Bearer {}", cal.key)),
    );
    assert_eq!(status, 429);
    assert!(res.header_value("retry-after").is_some());
    // A stranger cannot spend the owner's allowance.
    assert_eq!(call(&app, "wrong", "GET", &path, None).0, 401);

    for _ in 0..5 {
        assert_eq!(call(&app, HOME_KEY, "GET", "/v1/export", None).0, 200);
    }
}

#[test]
fn the_home_calendar_exports_at_the_short_route() {
    let app = app();
    assert_eq!(call(&app, HOME_KEY, "PUT", "/v1/notes/h", Some(r#"{"title":"Mine","body":"hi"}"#)).0, 201);
    let (status, v) = call(&app, HOME_KEY, "GET", "/v1/export", None);
    assert_eq!(status, 200);
    assert_eq!(text(field(&v, "calendar"), "id"), "home");
    assert_eq!(field(&v, "notes").as_array().unwrap().len(), 1);
    assert_eq!(call(&app, "wrong", "GET", "/v1/export", None).0, 401);
    assert_eq!(field(&v, "exportedAt"), &Value::String(text(&v, "exportedAt").to_string()));
}
