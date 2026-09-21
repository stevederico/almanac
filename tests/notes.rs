//! Notes over the HTTP surface, in process through `handle()`.

mod common;

use almanac::http::Request;
use almanac::json::Value;
use common::*;

fn titles(v: &Value) -> Vec<String> {
    common::titles(v, "notes")
}

// ---- CRUD -------------------------------------------------------------------

#[test]
fn creates_reads_updates_and_deletes_a_note() {
    let app = app();
    let cal = create(&app);
    let coll = format!("/v1/c/{}/notes", cal.id);

    let (status, v) = call(
        &app,
        &cal.key,
        "POST",
        &coll,
        Some(r##"{"title":"Ideas","body":"# Ideas\n- one","tags":["Work"],"pinned":true}"##),
    );
    assert_eq!(status, 201);
    let uid = text(&v, "uid").to_string();
    assert!(uid.starts_with("note-"), "{uid}");
    assert_eq!(text(&v, "body"), "# Ideas\n- one");
    assert_eq!(field(&v, "pinned"), &Value::Bool(true));
    assert_eq!(field(&v, "tags").as_array().unwrap().len(), 1);

    let item = format!("{coll}/{uid}");
    let (status, got) = call(&app, &cal.key, "GET", &item, None);
    assert_eq!(status, 200);
    assert_eq!(text(&got, "body"), "# Ideas\n- one", "a single note carries its body");

    let (status, patched) = call(&app, &cal.key, "PATCH", &item, Some(r#"{"body":"changed"}"#));
    assert_eq!(status, 200);
    assert_eq!(text(&patched, "body"), "changed");
    assert_eq!(text(&patched, "title"), "Ideas", "untouched fields stay");
    assert_eq!(field(&patched, "pinned"), &Value::Bool(true));

    assert_eq!(call(&app, &cal.key, "DELETE", &item, None).0, 204);
    assert_eq!(call(&app, &cal.key, "GET", &item, None).0, 404);
    assert_eq!(call(&app, &cal.key, "DELETE", &item, None).0, 404);
    assert_eq!(call(&app, &cal.key, "PATCH", &item, Some(r#"{"body":"x"}"#)).0, 404);
}

#[test]
fn put_is_idempotent_and_keeps_fields_it_does_not_mention() {
    let app = app();
    let cal = create(&app);
    let item = format!("/v1/c/{}/notes/plan", cal.id);

    let (status, _) = call(&app, &cal.key, "PUT", &item, Some(r#"{"title":"Plan","body":"draft","tags":["a"]}"#));
    assert_eq!(status, 201);
    let (status, v) = call(&app, &cal.key, "PUT", &item, Some(r#"{"title":"Plan v2"}"#));
    assert_eq!(status, 200, "same uid replaces in place");
    assert_eq!(text(&v, "body"), "draft");
    assert_eq!(field(&v, "tags").as_array().unwrap().len(), 1);

    let (_, v) = call(&app, &cal.key, "PUT", &item, Some(r#"{"title":"Plan v2","body":null,"tags":[]}"#));
    assert_eq!(text(&v, "body"), "");
    assert!(field(&v, "tags").as_array().unwrap().is_empty());

    let (_, list) = call(&app, &cal.key, "GET", &format!("/v1/c/{}/notes", cal.id), None);
    assert_eq!(titles(&list), ["Plan v2"], "still one note");
}

#[test]
fn a_list_leaves_the_bodies_out_unless_asked() {
    let app = app();
    let cal = create(&app);
    let coll = format!("/v1/c/{}/notes", cal.id);
    call(&app, &cal.key, "PUT", &format!("{coll}/n"), Some(r#"{"title":"T","body":"hello"}"#));

    let (_, lean) = call(&app, &cal.key, "GET", &coll, None);
    let first = &field(&lean, "notes").as_array().unwrap()[0];
    assert!(first.get("body").is_none());
    assert_eq!(field(first, "size"), &Value::Number(5));

    let (_, full) = call(&app, &cal.key, "GET", &format!("{coll}?body=true"), None);
    assert_eq!(text(&field(&full, "notes").as_array().unwrap()[0], "body"), "hello");
}

// ---- validation -------------------------------------------------------------

#[test]
fn rejects_bad_input_with_a_400() {
    let app = app();
    let cal = create(&app);
    let coll = format!("/v1/c/{}/notes", cal.id);
    let long_title = format!(r#"{{"title":"{}"}}"#, "x".repeat(201));
    let long_body = format!(r#"{{"title":"x","body":"{}"}}"#, "y".repeat(32 * 1024 + 1));
    for bad in [
        "{}",
        r#"{"title":""}"#,
        r#"{"title":5}"#,
        r#"{"title":"x","body":5}"#,
        r#"{"title":"x","pinned":"yes"}"#,
        r#"{"title":"x","tags":["has space"]}"#,
        "[]",
        "not json",
        long_title.as_str(),
        long_body.as_str(),
    ] {
        let shown: String = bad.chars().take(60).collect();
        assert_eq!(call(&app, &cal.key, "POST", &coll, Some(bad)).0, 400, "{shown}");
    }
    assert_eq!(call(&app, &cal.key, "PUT", &format!("{coll}/bad%20uid"), Some(r#"{"title":"x"}"#)).0, 400);
    assert_eq!(call(&app, &cal.key, "PATCH", &format!("{coll}/x"), Some("{}")).0, 400);
    let (_, list) = call(&app, &cal.key, "GET", &coll, None);
    assert!(titles(&list).is_empty(), "nothing was created");

    // The largest body that fits is accepted.
    let biggest = format!(r#"{{"title":"x","body":"{}"}}"#, "y".repeat(32 * 1024));
    assert_eq!(call(&app, &cal.key, "POST", &coll, Some(&biggest)).0, 201);
}

#[test]
fn wrong_methods_and_paths_are_404_even_without_a_key() {
    let app = app();
    let cal = create(&app);
    let coll = format!("/v1/c/{}/notes", cal.id);
    for (m, p) in [
        ("PUT", coll.clone()),
        ("DELETE", coll.clone()),
        ("POST", format!("{coll}/x")),
        ("GET", format!("{coll}/a/b")),
        ("GET", format!("{coll}/")),
    ] {
        assert_eq!(send(&app, Request::new(m, &p)).0, 404, "{m} {p}");
    }
}

// ---- listing and search -----------------------------------------------------

#[test]
fn lists_pinned_first_then_newest_and_filters_by_tag() {
    let app = app();
    let cal = create(&app);
    let coll = format!("/v1/c/{}/notes", cal.id);
    for body in [
        r#"{"title":"old","tags":["a"]}"#,
        r#"{"title":"pinned","pinned":true,"tags":["a","b"]}"#,
        r#"{"title":"new"}"#,
    ] {
        assert_eq!(call(&app, &cal.key, "POST", &coll, Some(body)).0, 201);
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
    let list = |q: &str| titles(&call(&app, &cal.key, "GET", &format!("{coll}{q}"), None).1);
    assert_eq!(list(""), ["pinned", "new", "old"]);
    assert_eq!(list("?tag=a"), ["pinned", "old"]);
    assert_eq!(list("?tag=B"), ["pinned"]);
    assert_eq!(list("?tag=none"), Vec::<String>::new());
}

#[test]
fn search_matches_title_or_body_and_treats_wildcards_and_quotes_literally() {
    let app = app();
    let cal = create(&app);
    let coll = format!("/v1/c/{}/notes", cal.id);
    for (title, body) in [
        ("Groceries", "Milk and eggs"),
        ("Trip", "book the FLIGHT"),
        ("Score", "we are 100% sure"),
        ("Snake", "a_b"),
        ("Decoy", "axb"),
        ("Quote", "it's fine"),
        ("Path", r"c:\dir"),
    ] {
        let body = format!(r#"{{"title":"{title}","body":{}}}"#, almanac::json::stringify(&Value::String(body.into())));
        assert_eq!(call(&app, &cal.key, "POST", &coll, Some(&body)).0, 201);
    }
    let find = |q: &str| {
        let mut t = titles(&call(&app, &cal.key, "GET", &format!("{coll}?q={q}"), None).1);
        t.sort();
        t
    };
    assert_eq!(find("milk"), ["Groceries"], "case-insensitive, in the body");
    assert_eq!(find("trip"), ["Trip"], "in the title");
    assert_eq!(find("flight"), ["Trip"]);
    assert_eq!(find("100%25"), ["Score"], "% is not a wildcard");
    assert_eq!(find("a_b"), ["Snake"], "_ is not a wildcard");
    assert_eq!(find("it%27s"), ["Quote"]);
    assert_eq!(find("c%3A%5Cdir"), ["Path"], "a backslash is literal");
    assert_eq!(find("%27%3B%20DROP%20TABLE%20notes%3B--"), Vec::<String>::new());
    assert_eq!(find("nothing"), Vec::<String>::new());
    // The table survived the injection attempt.
    assert_eq!(titles(&call(&app, &cal.key, "GET", &coll, None).1).len(), 7);
    // An empty q is no filter; an oversized one is refused.
    assert_eq!(find("").len(), 7);
    assert_eq!(call(&app, &cal.key, "GET", &format!("{coll}?q={}", "x".repeat(201)), None).0, 400);
}

// ---- one key, isolation, limits ---------------------------------------------

#[test]
fn one_key_writes_everything_and_nothing_crosses_calendars() {
    let app = app();
    let a = create(&app);
    let b = create(&app);
    let ev = r#"{"summary":"Dentist","start":"2026-10-01T09:00:00-07:00"}"#;
    assert_eq!(call(&app, &a.key, "PUT", &format!("/v1/c/{}/events/e", a.id), Some(ev)).0, 201);
    assert_eq!(call(&app, &a.key, "PUT", &format!("/v1/c/{}/todos/t", a.id), Some(r#"{"title":"x"}"#)).0, 201);
    assert_eq!(call(&app, &a.key, "PUT", &format!("/v1/c/{}/notes/n", a.id), Some(r#"{"title":"Mine"}"#)).0, 201);

    assert_eq!(call(&app, &a.key, "GET", &format!("/v1/c/{}/notes", b.id), None).0, 401);
    assert_eq!(call(&app, &a.key, "PUT", &format!("/v1/c/{}/notes/n", b.id), Some(r#"{"title":"x"}"#)).0, 401);
    assert_eq!(call(&app, "", "GET", &format!("/v1/c/{}/notes", a.id), None).0, 401);
    assert_eq!(call(&app, "wrong", "GET", &format!("/v1/c/{}/notes/n", a.id), None).0, 401);
    let (_, list) = call(&app, &b.key, "GET", &format!("/v1/c/{}/notes", b.id), None);
    assert!(titles(&list).is_empty());
    assert_eq!(call(&app, &b.key, "PUT", &format!("/v1/c/{}/notes/n", b.id), Some(r#"{"title":"B's"}"#)).0, 201);
    assert_eq!(text(&call(&app, &a.key, "GET", &format!("/v1/c/{}/notes/n", a.id), None).1, "title"), "Mine");
}

#[test]
fn the_home_calendar_has_short_note_routes() {
    let app = app();
    assert_eq!(call(&app, HOME_KEY, "PUT", "/v1/notes/h1", Some(r#"{"title":"Mine"}"#)).0, 201);
    let (status, list) = call(&app, HOME_KEY, "GET", "/v1/notes", None);
    assert_eq!(status, 200);
    assert_eq!(titles(&list), ["Mine"]);
    assert_eq!(call(&app, "wrong", "GET", "/v1/notes", None).0, 401);
    let (_, info) = call(&app, HOME_KEY, "GET", "/v1/c/home", None);
    assert!(text(field(&info, "notes"), "subscribe").ends_with(".atom"));
}

#[test]
fn the_note_cap_holds_for_strangers_but_not_for_home_or_replacements() {
    let mut app = app();
    app.limits.max_notes = 2;
    let cal = create(&app);
    let coll = format!("/v1/c/{}/notes", cal.id);
    assert_eq!(call(&app, &cal.key, "PUT", &format!("{coll}/a"), Some(r#"{"title":"a"}"#)).0, 201);
    assert_eq!(call(&app, &cal.key, "POST", &coll, Some(r#"{"title":"b"}"#)).0, 201);
    assert_eq!(call(&app, &cal.key, "POST", &coll, Some(r#"{"title":"c"}"#)).0, 409);
    assert_eq!(call(&app, &cal.key, "PUT", &format!("{coll}/a"), Some(r#"{"title":"a2"}"#)).0, 200);
    assert_eq!(call(&app, &cal.key, "DELETE", &format!("{coll}/a"), None).0, 204);
    assert_eq!(call(&app, &cal.key, "POST", &coll, Some(r#"{"title":"c"}"#)).0, 201);
    for i in 0..5 {
        let path = format!("/v1/notes/h{i}");
        assert_eq!(call(&app, HOME_KEY, "PUT", &path, Some(r#"{"title":"h"}"#)).0, 201);
    }
}

#[test]
fn events_todos_and_notes_spend_one_write_budget() {
    let mut app = app();
    app.limits.writes_per_minute = 3;
    let cal = create(&app);
    let ev = r#"{"summary":"x","start":"2026-10-01"}"#;
    assert_eq!(call(&app, &cal.key, "PUT", &format!("/v1/c/{}/events/e", cal.id), Some(ev)).0, 201);
    assert_eq!(call(&app, &cal.key, "PUT", &format!("/v1/c/{}/todos/t", cal.id), Some(r#"{"title":"x"}"#)).0, 201);
    assert_eq!(call(&app, &cal.key, "PUT", &format!("/v1/c/{}/notes/n", cal.id), Some(r#"{"title":"x"}"#)).0, 201);
    assert_eq!(call(&app, &cal.key, "PUT", &format!("/v1/c/{}/notes/n2", cal.id), Some(r#"{"title":"y"}"#)).0, 429);
    assert_eq!(call(&app, &cal.key, "GET", &format!("/v1/c/{}/notes", cal.id), None).0, 200);
}

// ---- feed -------------------------------------------------------------------

#[test]
fn the_notes_feed_is_atom_and_holds_only_notes() {
    let app = app();
    let cal = create(&app);
    call(&app, &cal.key, "PUT", &format!("/v1/c/{}/events/e", cal.id), Some(r#"{"summary":"Dentist","start":"2026-10-01"}"#));
    call(&app, &cal.key, "PUT", &format!("/v1/c/{}/todos/t", cal.id), Some(r#"{"title":"Floss"}"#));
    call(
        &app,
        &cal.key,
        "PUT",
        &format!("/v1/c/{}/notes/n1", cal.id),
        Some(r#"{"title":"R&D <plan>","body":"a & b\n</content><evil/>","tags":["work"]}"#),
    );

    let (status, xml, res) = send(&app, Request::new("GET", &cal.notes_feed_path));
    assert_eq!(status, 200, "{xml}");
    assert!(cal.notes_feed_path.ends_with(".atom"));
    assert!(res.header_value("content-type").unwrap().starts_with("application/atom+xml"));
    assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<feed xmlns="));
    assert_eq!(xml.matches("<entry>").count(), 1);
    assert!(xml.contains(&format!("<id>urn:almanac:{}:n1</id>", cal.id)));
    assert!(xml.contains("<title>R&amp;D &lt;plan&gt;</title>"), "{xml}");
    assert!(xml.contains("<category term=\"work\"/>"));
    assert!(!xml.contains("<evil"), "{xml}");
    assert!(!xml.contains("Dentist") && !xml.contains("Floss"));
    assert!(xml.trim_end().ends_with("</feed>"));

    // Its token is its own, and the .ics spelling of it also works.
    assert_ne!(cal.notes_feed_path, cal.todos_feed_path);
    assert_ne!(cal.notes_feed_path, cal.events_feed_path);
    let as_ics = cal.notes_feed_path.replace(".atom", ".ics");
    assert_eq!(send(&app, Request::new("GET", &as_ics)).0, 200);
    assert_eq!(send(&app, Request::new("GET", &format!("{}x", cal.notes_feed_path))).0, 404);
    assert_eq!(send(&app, Request::new("POST", &cal.notes_feed_path)).0, 404);

    // The other two feeds never carry notes.
    let (_, ics, _) = send(&app, Request::new("GET", &cal.todos_feed_path));
    assert!(!ics.contains("R&D") && !ics.contains("VJOURNAL"));
    let (_, ics, _) = send(&app, Request::new("GET", &cal.events_feed_path));
    assert!(!ics.contains("R&D"));
}

#[test]
fn an_empty_notes_feed_is_still_a_valid_feed() {
    let app = app();
    let cal = create(&app);
    let (status, xml, _) = send(&app, Request::new("GET", &cal.notes_feed_path));
    assert_eq!(status, 200);
    assert!(xml.contains("<updated>20"), "a feed needs an updated time: {xml}");
    assert!(xml.contains("<title>Trip Notes</title>"));
    assert!(!xml.contains("<entry>"));
}

#[test]
fn the_feed_carries_only_the_newest_entries() {
    let mut app = app();
    app.limits.max_notes = 1000;
    app.limits.writes_per_minute = 100_000;
    let cal = create(&app);
    for i in 0..210 {
        let path = format!("/v1/c/{}/notes/n{i:03}", cal.id);
        assert_eq!(call(&app, &cal.key, "PUT", &path, Some(r#"{"title":"n"}"#)).0, 201);
    }
    let (_, xml, _) = send(&app, Request::new("GET", &cal.notes_feed_path));
    assert_eq!(xml.matches("<entry>").count(), 200);
    let (_, list) = call(&app, &cal.key, "GET", &format!("/v1/c/{}/notes", cal.id), None);
    assert_eq!(titles(&list).len(), 210, "the API still lists them all");
}
