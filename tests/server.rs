

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
    AppState::new(db, BASE.into())
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
    let resources = v.get("resources").expect("the index lists every resource");
    for kind in ["events", "todos", "notes", "export"] {
        let url = resources.get(kind).and_then(Value::as_str).unwrap();
        assert!(url.contains("/v1/c/{id}/") && url.ends_with(kind), "{kind}: {url}");
    }
    assert!(v.get("description").and_then(Value::as_str).unwrap().contains("todos"));
}

#[test]
fn returns_html_for_browsers() {
    let (status, body, _) = send(
        &test_app(),
        Request::new("GET", "/").with_header("accept", "text/html"),
    );
    assert_eq!(status, 200);
    let html = String::from_utf8(body).unwrap();
    assert!(html.contains("Your agent keeps the calendar."));
    assert!(html.contains("send your agent this"));
    assert!(html.contains("Copy prompt"));
    assert!(html.contains("/llms.txt"));
    assert!(html.contains("/calendars"));
    assert!(html.contains("Subscribe from web"));
    assert!(html.contains("Do not add events unless I ask"));
    // The page has to say it is more than a calendar.
    assert!(html.contains("todos") && html.contains("notes"));
    assert!(html.contains("todos.subscribe") && html.contains("notes.subscribe"));
    assert!(html.contains("/export"));
    // The three products are shown, not just described, and the bunny is gone.
    for app in ["A calendar app", "A todo app", "A notes app"] {
        assert!(html.contains(app), "no mockup for {app}");
    }
    assert!(html.contains("Three products. One key."));
    assert!(!html.contains("mascot") && !html.contains("<img"));
    // No external requests: fonts and images are all local.
    assert!(!html.contains("fonts.googleapis"));
    assert!(!html.contains("<link rel=\"stylesheet\""));
    assert!(!html.contains("{origin}"), "an unformatted placeholder leaked into the page");
    assert!(html.contains("example.test/v1/c/{id}/export"));
    assert!(html.contains("og.png?v="), "the share image URL must be versioned");
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
    assert!(text.contains("403"), "say what happens without one");
    // Only true because GET /v1/c/{id} no longer returns the key; see
    // `the_key_is_shown_at_creation_and_never_again`.
    assert!(text.contains("shown once"));
    assert!(text.contains("cannot be recovered"));
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

#[test]
fn writes_a_series_with_exceptions_on_both_url_trees() {
    let app = test_app();
    let (_, created_body, _) = send(
        &app,
        Request::new("POST", "/calendars")
            .with_header("content-type", "application/json")
            .with_header("accept", "application/json")
            .with_body(b"{}".to_vec()),
    );
    let created = parse(std::str::from_utf8(&created_body).unwrap()).unwrap();
    let id = created.get("id").and_then(Value::as_str).unwrap();
    let key = created.get("key").and_then(Value::as_str).unwrap();
    let sub = created
        .get("subscribe")
        .and_then(Value::as_str)
        .unwrap()
        .strip_prefix(BASE)
        .unwrap();

    let series = br#"{"summary":"Standup","start":"2026-09-22T09:00:00-07:00","end":"2026-09-22T09:15:00-07:00","timeZone":"America/Los_Angeles","rrule":"FREQ=WEEKLY;BYDAY=TU"}"#;
    let (status, body, _) = send(
        &app,
        Request::new("PUT", &format!("/v1/c/{id}/events/standup"))
            .with_header("authorization", &format!("Bearer {key}"))
            .with_header("content-type", "application/json")
            .with_body(series.to_vec()),
    );
    assert_eq!(status, 201);
    let saved = parse(std::str::from_utf8(&body).unwrap()).unwrap();
    assert_eq!(
        saved.get("rrule").and_then(Value::as_str),
        Some("FREQ=WEEKLY;BYDAY=TU")
    );
    assert_eq!(
        saved.get("start").and_then(Value::as_str),
        Some("2026-09-22T09:00:00")
    );
    assert_eq!(
        saved.get("timeZone").and_then(Value::as_str),
        Some("America/Los_Angeles")
    );

    let (missing_tz, _, _) = send(
        &app,
        Request::new("PUT", &format!("/v1/c/{id}/events/no-zone"))
            .with_header("authorization", &format!("Bearer {key}"))
            .with_header("content-type", "application/json")
            .with_body(
                br#"{"summary":"Nope","start":"2026-09-22T09:00:00-07:00","rrule":"FREQ=DAILY"}"#
                    .to_vec(),
            ),
    );
    assert_eq!(missing_tz, 400);

    let (bad_zone, _, _) = send(
        &app,
        Request::new("PUT", &format!("/v1/c/{id}/events/bad-zone"))
            .with_header("authorization", &format!("Bearer {key}"))
            .with_header("content-type", "application/json")
            .with_body(
                br#"{"summary":"Nope","start":"2026-09-22T09:00:00-07:00","timeZone":"Mars/Base","rrule":"FREQ=DAILY"}"#
                    .to_vec(),
            ),
    );
    assert_eq!(bad_zone, 400);

    let (ex_status, ex_body, _) = send(
        &app,
        Request::new("PUT", &format!("/v1/c/{id}/events/standup/exdates"))
            .with_header("authorization", &format!("Bearer {key}"))
            .with_header("content-type", "application/json")
            .with_body(br#"{"recurrenceId":"2026-10-06T09:00:00"}"#.to_vec()),
    );
    assert_eq!(ex_status, 201);
    let skipped = parse(std::str::from_utf8(&ex_body).unwrap()).unwrap();
    assert_eq!(
        skipped
            .get("exdates")
            .and_then(Value::as_array)
            .unwrap()
            .len(),
        1
    );

    let (over_status, over_body, _) = send(
        &app,
        Request::new("PUT", &format!("/v1/c/{id}/events/standup/overrides"))
            .with_header("authorization", &format!("Bearer {key}"))
            .with_header("content-type", "application/json")
            .with_body(
                br#"{"recurrenceId":"2026-10-13T09:00:00","summary":"Standup late","start":"2026-10-13T10:00:00","end":"2026-10-13T10:15:00"}"#
                    .to_vec(),
            ),
    );
    assert_eq!(over_status, 201);
    let moved = parse(std::str::from_utf8(&over_body).unwrap()).unwrap();
    assert_eq!(
        moved
            .get("overrides")
            .and_then(Value::as_array)
            .unwrap()
            .len(),
        1
    );

    let (got_status, got_body, _) = send(
        &app,
        Request::new("GET", &format!("/v1/c/{id}/events/standup"))
            .with_header("authorization", &format!("Bearer {key}")),
    );
    assert_eq!(got_status, 200);
    let got = parse(std::str::from_utf8(&got_body).unwrap()).unwrap();
    assert!(got.get("exdates").and_then(Value::as_array).is_some());
    assert!(got.get("overrides").and_then(Value::as_array).is_some());

    let (blocked, _, _) = send(
        &app,
        Request::new("PATCH", &format!("/v1/c/{id}/events/standup"))
            .with_header("authorization", &format!("Bearer {key}"))
            .with_header("content-type", "application/json")
            .with_body(br#"{"rrule":""}"#.to_vec()),
    );
    assert_eq!(blocked, 400);

    let (feed_status, feed_body, _) = send(&app, Request::new("GET", sub));
    assert_eq!(feed_status, 200);
    let feed = String::from_utf8(feed_body).unwrap();
    assert!(feed.contains("BEGIN:VTIMEZONE"));
    assert!(feed.contains("TZID:America/Los_Angeles"));
    assert!(feed.contains("RRULE:FREQ=WEEKLY;BYDAY=TU"));
    assert!(feed.contains("EXDATE;TZID=America/Los_Angeles:20261006T090000"));
    assert!(feed.contains("RECURRENCE-ID;TZID=America/Los_Angeles:20261013T090000"));
    assert!(feed.contains("DTSTART;TZID=America/Los_Angeles:20260922T090000"));

    let (gone_ex, _, _) = send(
        &app,
        Request::new("DELETE", &format!("/v1/c/{id}/events/standup/exdates"))
            .with_header("authorization", &format!("Bearer {key}"))
            .with_header("content-type", "application/json")
            .with_body(br#"{"recurrenceId":"2026-10-06T09:00:00"}"#.to_vec()),
    );
    assert_eq!(gone_ex, 204);
    let (gone_over, _, _) = send(
        &app,
        Request::new("DELETE", &format!("/v1/c/{id}/events/standup/overrides"))
            .with_header("authorization", &format!("Bearer {key}"))
            .with_header("content-type", "application/json")
            .with_body(br#"{"recurrenceId":"2026-10-13T09:00:00"}"#.to_vec()),
    );
    assert_eq!(gone_over, 204);

    let (home_status, _, _) = send(
        &app,
        Request::new("PUT", "/v1/events/home-standup")
            .with_header("authorization", &format!("Bearer {KEY}"))
            .with_header("content-type", "application/json")
            .with_body(series.to_vec()),
    );
    assert_eq!(home_status, 201);
    let (home_ex, _, _) = send(
        &app,
        Request::new("PUT", "/v1/events/home-standup/exdates")
            .with_header("authorization", &format!("Bearer {KEY}"))
            .with_header("content-type", "application/json")
            .with_body(br#"{"recurrenceId":"2026-10-06T09:00:00"}"#.to_vec()),
    );
    assert_eq!(home_ex, 201);

    let (one_off, one_body, _) = send(
        &app,
        Request::new("PUT", "/v1/events/once")
            .with_header("authorization", &format!("Bearer {KEY}"))
            .with_header("content-type", "application/json")
            .with_body(br#"{"summary":"Once","start":"2026-08-16T13:05:00-07:00"}"#.to_vec()),
    );
    assert_eq!(one_off, 201);
    let once = parse(std::str::from_utf8(&one_body).unwrap()).unwrap();
    assert!(once.get("rrule").is_none());

    let (_, home_feed, _) = send(&app, Request::new("GET", &format!("/feed/{FEED}.ics")));
    let home_feed = String::from_utf8(home_feed).unwrap();
    assert!(home_feed.contains("DTSTART:20260816T200500Z"));
    assert!(home_feed.contains("UID:once"));
    let once_at = home_feed.find("UID:once").unwrap();
    let once_end = home_feed[once_at..].find("END:VEVENT").unwrap();
    assert!(!home_feed[once_at..once_at + once_end].contains("RRULE:"));
}

// ---- crash / poison regressions -------------------------------------------

fn home_req(method: &str, path: &str, body: &str) -> Request {
    Request::new(method, path)
        .with_header("authorization", &format!("Bearer {KEY}"))
        .with_header("content-type", "application/json")
        .with_body(body.as_bytes().to_vec())
}

/// Every request that follows a hostile one must still be served.
fn assert_db_alive(app: &AppState) {
    let (status, _, _) = send(app, auth_get("/v1/events"));
    assert_eq!(status, 200, "db lock is unusable after a hostile request");
    let (status, _, _) = send(app, Request::new("GET", "/health"));
    assert_eq!(status, 200);
}

/// Swap the ASCII char at every offset for a 2-byte one, so a fixed-offset
/// slice lands inside a char. Also insert it, shifting every later byte.
fn mutations(valid: &str) -> Vec<String> {
    let chars: Vec<char> = valid.chars().collect();
    let mut out = Vec::new();
    for i in 0..chars.len() {
        let mut swapped = chars.clone();
        swapped[i] = 'é';
        out.push(swapped.into_iter().collect());
        let mut inserted = chars.clone();
        inserted.insert(i, 'é');
        out.push(inserted.into_iter().collect());
    }
    out
}

#[test]
fn deep_json_does_not_kill_the_process() {
    let app = test_app();
    for body in ["[".repeat(200_000), "{\"a\":".repeat(200_000)] {
        let (status, _, _) = send(
            &app,
            Request::new("POST", "/calendars")
                .with_header("content-type", "application/json")
                .with_body(body.clone().into_bytes()),
        );
        assert!(status == 201 || status == 400, "status {status}");
        let (status, _, _) = send(&app, home_req("PUT", "/v1/events/deep", &body));
        assert_eq!(status, 400);
    }
    assert_db_alive(&app);
}

#[test]
fn non_ascii_datetimes_are_400_not_a_panic() {
    let app = test_app();
    for bad in mutations("2026-09-22T09:00:00.000Z") {
        for field in ["start", "end"] {
            let other = if field == "start" { "end" } else { "start" };
            let other_val = if field == "start" {
                "2026-09-23T09:00:00.000Z"
            } else {
                "2026-09-21T09:00:00.000Z"
            };
            let body =
                format!(r#"{{"summary":"x","{field}":"{bad}","{other}":"{other_val}"}}"#);
            let (status, _, _) = send(&app, home_req("PUT", "/v1/events/mb", &body));
            assert_eq!(status, 400, "{field}={bad:?}");
        }
    }
    assert_db_alive(&app);
}

#[test]
fn non_ascii_wall_times_and_recurrence_ids_are_400() {
    let app = test_app();
    let series = r#"{"summary":"S","start":"2026-09-22T09:00:00-07:00","end":"2026-09-22T09:15:00-07:00","timeZone":"America/Los_Angeles","rrule":"FREQ=WEEKLY;BYDAY=TU"}"#;
    let (status, _, _) = send(&app, home_req("PUT", "/v1/events/s", series));
    assert_eq!(status, 201);
    for bad in mutations("2026-10-06T09:00:00") {
        // Recurring start: goes through parse_wall.
        let body = series.replace("2026-09-22T09:00:00-07:00", &bad);
        let (status, _, _) = send(&app, home_req("PUT", "/v1/events/s2", &body));
        assert_eq!(status, 400, "series start={bad:?}");
        for path in ["/v1/events/s/exdates", "/v1/events/s/overrides"] {
            let body = format!(
                r#"{{"recurrenceId":"{bad}","start":"2026-10-06T10:00:00","end":"2026-10-06T10:15:00"}}"#
            );
            let (status, _, _) = send(&app, home_req("PUT", path, &body));
            assert_eq!(status, 400, "{path} recurrenceId={bad:?}");
            let (status, _, _) = send(&app, home_req("DELETE", path, &body));
            assert!(status == 400 || status == 404, "{path} delete {status}");
        }
    }
    assert_db_alive(&app);
}

#[test]
fn a_poisoned_db_lock_does_not_brick_the_service() {
    let app = test_app();
    let poisoner = app.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poisoner.db.lock().unwrap();
        panic!("simulated handler panic while holding the db lock");
    })
    .join();
    assert!(app.db.is_poisoned());
    assert_db_alive(&app);
    let (status, _, _) = send(
        &app,
        home_req(
            "PUT",
            "/v1/events/after",
            r#"{"summary":"ok","start":"2026-09-22"}"#,
        ),
    );
    assert_eq!(status, 201);
}

#[test]
fn form_names_with_stray_percent_signs_do_not_panic() {
    let app = test_app();
    for name in ["%", "%é", "a%4", "%zz", "%%41", "é%é%", "%41%42+x"] {
        let (status, _, _) = send(
            &app,
            Request::new("POST", "/calendars")
                .with_header("content-type", "application/x-www-form-urlencoded")
                .with_body(format!("name={name}").into_bytes()),
        );
        assert_eq!(status, 201, "{name}");
    }
}

// ---- limits ----------------------------------------------------------------

fn create(app: &AppState, xff: Option<&str>) -> (u16, almanac::http::Response) {
    let mut req = Request::new("POST", "/calendars")
        .with_header("content-type", "application/json")
        .with_header("accept", "application/json")
        .with_body(b"{}".to_vec());
    if let Some(xff) = xff {
        req = req.with_header("x-forwarded-for", xff);
    }
    let (status, _, res) = send(app, req);
    (status, res)
}

fn new_calendar(app: &AppState) -> (String, String) {
    let (status, res) = create(app, None);
    assert_eq!(status, 201);
    let v = parse(std::str::from_utf8(&res.body).unwrap()).unwrap();
    (
        v.get("id").and_then(Value::as_str).unwrap().to_string(),
        v.get("key").and_then(Value::as_str).unwrap().to_string(),
    )
}

fn call(app: &AppState, method: &str, path: &str, key: &str, body: &str) -> u16 {
    send(
        app,
        Request::new(method, path)
            .with_header("authorization", &format!("Bearer {key}"))
            .with_header("content-type", "application/json")
            .with_body(body.as_bytes().to_vec()),
    )
    .0
}

fn day(n: u32) -> String {
    format!(r#"{{"summary":"e{n}","start":"2026-10-{n:02}"}}"#)
}

#[test]
fn rate_limits_calendar_creation_per_client() {
    let mut app = test_app();
    app.limits.creates_per_client_hour = 3;
    for _ in 0..3 {
        assert_eq!(create(&app, Some("1.1.1.1")).0, 201);
    }
    let (status, res) = create(&app, Some("1.1.1.1"));
    assert_eq!(status, 429);
    let retry: u64 = res.header_value("retry-after").unwrap().parse().unwrap();
    assert!((1..=3600).contains(&retry));
    // Another client is unaffected.
    assert_eq!(create(&app, Some("2.2.2.2")).0, 201);
    // A caller cannot pick their own bucket by prepending addresses: one proxy
    // hop is trusted, so only the rightmost entry counts.
    assert_eq!(create(&app, Some("9.9.9.9, 1.1.1.1")).0, 429);
    assert_eq!(create(&app, Some("1.1.1.1, 3.3.3.3")).0, 201);
}

#[test]
fn caps_calendar_creation_globally_and_in_total() {
    let mut app = test_app();
    app.limits.creates_global_hour = 2;
    assert_eq!(create(&app, Some("1.1.1.1")).0, 201);
    assert_eq!(create(&app, Some("2.2.2.2")).0, 201);
    assert_eq!(create(&app, Some("3.3.3.3")).0, 429);

    let mut app = test_app();
    // The seeded home calendar counts.
    app.limits.max_calendars = 3;
    assert_eq!(create(&app, Some("1.1.1.1")).0, 201);
    assert_eq!(create(&app, Some("2.2.2.2")).0, 201);
    assert_eq!(create(&app, Some("3.3.3.3")).0, 503);
}

#[test]
fn caps_events_per_calendar_but_not_home() {
    let mut app = test_app();
    app.limits.max_events = 3;
    let (id, key) = new_calendar(&app);
    let url = |uid: &str| format!("/v1/c/{id}/events/{uid}");
    for n in 1..=3 {
        assert_eq!(call(&app, "PUT", &url(&format!("e{n}")), &key, &day(n)), 201);
    }
    assert_eq!(call(&app, "PUT", &url("e4"), &key, &day(4)), 409);
    assert_eq!(
        call(&app, "POST", &format!("/v1/c/{id}/events"), &key, &day(5)),
        409
    );
    // Replacing an existing event is not adding one.
    assert_eq!(call(&app, "PUT", &url("e1"), &key, &day(6)), 200);
    // Deleting frees a slot.
    assert_eq!(call(&app, "DELETE", &url("e1"), &key, ""), 204);
    assert_eq!(call(&app, "PUT", &url("e4"), &key, &day(4)), 201);
    // The owner's calendar is exempt.
    for n in 1..=5 {
        assert_eq!(
            call(&app, "PUT", &format!("/v1/events/h{n}"), KEY, &day(n)),
            201
        );
    }
}

#[test]
fn caps_exceptions_per_series() {
    let mut app = test_app();
    app.limits.max_exceptions = 2;
    let (id, key) = new_calendar(&app);
    let base = format!("/v1/c/{id}/events/standup");
    let series = r#"{"summary":"S","start":"2026-09-22T09:00:00-07:00","end":"2026-09-22T09:15:00-07:00","timeZone":"America/Los_Angeles","rrule":"FREQ=WEEKLY;BYDAY=TU"}"#;
    assert_eq!(call(&app, "PUT", &base, &key, series), 201);
    let ex = |d: &str| format!(r#"{{"recurrenceId":"2026-{d}T09:00:00"}}"#);
    let ov = |d: &str| {
        format!(r#"{{"recurrenceId":"2026-{d}T09:00:00","start":"2026-{d}T10:00:00"}}"#)
    };
    let exd = format!("{base}/exdates");
    let ovr = format!("{base}/overrides");
    assert_eq!(call(&app, "PUT", &exd, &key, &ex("10-06")), 201);
    assert_eq!(call(&app, "PUT", &ovr, &key, &ov("10-13")), 201);
    assert_eq!(call(&app, "PUT", &exd, &key, &ex("10-20")), 409);
    assert_eq!(call(&app, "PUT", &ovr, &key, &ov("10-20")), 409);
    // Touching one that exists is fine at the cap.
    assert_eq!(call(&app, "PUT", &exd, &key, &ex("10-06")), 200);
    assert_eq!(call(&app, "PUT", &ovr, &key, &ov("10-13")), 200);
    // Removing one makes room. Deleting is never blocked.
    assert_eq!(call(&app, "DELETE", &exd, &key, &ex("10-06")), 204);
    assert_eq!(call(&app, "PUT", &exd, &key, &ex("10-20")), 201);
}

#[test]
fn rate_limits_writes_per_calendar() {
    let mut app = test_app();
    app.limits.writes_per_minute = 3;
    let (id, key) = new_calendar(&app);
    let (other_id, other_key) = new_calendar(&app);
    let url = |uid: &str| format!("/v1/c/{id}/events/{uid}");
    for n in 1..=3 {
        assert_eq!(call(&app, "PUT", &url(&format!("e{n}")), &key, &day(n)), 201);
    }
    assert_eq!(call(&app, "PUT", &url("e4"), &key, &day(4)), 429);
    assert_eq!(call(&app, "DELETE", &url("e1"), &key, ""), 429);
    // Reads are free, other calendars have their own budget, and someone
    // without the key cannot spend this calendar's.
    assert_eq!(call(&app, "GET", &url("e1"), &key, ""), 200);
    assert_eq!(
        call(&app, "PUT", &format!("/v1/c/{other_id}/events/x"), &other_key, &day(1)),
        201
    );
    assert_eq!(call(&app, "PUT", &url("e9"), "wrong-key", &day(9)), 401);
    // The owner is exempt.
    for n in 1..=6 {
        assert_eq!(
            call(&app, "PUT", &format!("/v1/events/h{n}"), KEY, &day(n)),
            201
        );
    }
}

#[test]
fn a_wrong_key_does_not_spend_the_write_budget() {
    let mut app = test_app();
    app.limits.writes_per_minute = 1;
    let (id, key) = new_calendar(&app);
    for _ in 0..5 {
        assert_eq!(
            call(&app, "PUT", &format!("/v1/c/{id}/events/a"), "wrong", &day(1)),
            401
        );
    }
    assert_eq!(
        call(&app, "PUT", &format!("/v1/c/{id}/events/a"), &key, &day(1)),
        201
    );
}

#[test]
fn feed_keeps_each_series_own_exceptions() {
    let app = test_app();
    let series = |uid: &str| {
        (
            format!("/v1/events/{uid}"),
            r#"{"summary":"S","start":"2026-09-22T09:00:00-07:00","end":"2026-09-22T09:15:00-07:00","timeZone":"America/Los_Angeles","rrule":"FREQ=WEEKLY;BYDAY=TU"}"#,
        )
    };
    for (uid, skip) in [("a", "2026-10-06"), ("b", "2026-10-13")] {
        let (path, body) = series(uid);
        assert_eq!(call(&app, "PUT", &path, KEY, body), 201);
        let ex = format!(r#"{{"recurrenceId":"{skip}T09:00:00"}}"#);
        assert_eq!(call(&app, "PUT", &format!("{path}/exdates"), KEY, &ex), 201);
    }
    let (status, body, _) = send(&app, Request::new("GET", &format!("/feed/{FEED}.ics")));
    assert_eq!(status, 200);
    let ics = String::from_utf8(body).unwrap().replace("\r\n ", "");
    for (uid, own, other) in [("a", "20261006", "20261013"), ("b", "20261013", "20261006")] {
        let vevent = ics
            .split("BEGIN:VEVENT")
            .find(|v| v.contains(&format!("UID:{uid}\r\n")))
            .unwrap();
        assert!(vevent.contains(&format!("EXDATE;TZID=America/Los_Angeles:{own}T090000")));
        assert!(!vevent.contains(other), "{uid} picked up another series' exception");
    }
}

#[test]
fn rotating_the_home_key_takes_effect() {
    let app = test_app();
    assert_eq!(call(&app, "GET", "/v1/events", KEY, ""), 200);
    app.db
        .lock()
        .unwrap()
        .ensure_home_calendar(FEED, "rotated", "My Calendar")
        .unwrap();
    assert_eq!(call(&app, "GET", "/v1/events", KEY, ""), 401);
    assert_eq!(call(&app, "GET", "/v1/events", "rotated", ""), 200);
    // The feed token did not change, so subscribers are undisturbed.
    let (status, _, _) = send(&app, Request::new("GET", &format!("/feed/{FEED}.ics")));
    assert_eq!(status, 200);
}

#[test]
fn feed_carries_no_raw_control_characters() {
    let app = test_app();
    let body = r#"{"summary":"Bell\u0007 NUL\u0000 Esc\u001b[31m","location":"Rm\u007f1","description":"line1\r\nline2\ttab\u0085x","start":"2026-09-22"}"#;
    assert_eq!(call(&app, "PUT", "/v1/events/ctl", KEY, body), 201);
    let (status, feed, _) = send(&app, Request::new("GET", &format!("/feed/{FEED}.ics")));
    assert_eq!(status, 200);
    let text = String::from_utf8(feed).unwrap();
    let bad: Vec<char> = text
        .chars()
        .filter(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t'))
        .collect();
    assert!(bad.is_empty(), "control characters in the feed: {bad:?}");
    assert!(text.contains("SUMMARY:Bell NUL Esc[31m"));
    assert!(text.contains("line1\\nline2\ttab"));
}

#[test]
fn head_on_a_feed_matches_get_without_the_body() {
    let app = test_app();
    let path = format!("/feed/{FEED}.ics");
    let (get_status, get_body, get_res) = send(&app, Request::new("GET", &path));
    let (head_status, head_body, head_res) = send(&app, Request::new("HEAD", &path));
    assert_eq!((get_status, head_status), (200, 200));
    assert!(head_body.is_empty());
    assert_eq!(
        head_res.header_value("content-length"),
        Some(get_body.len().to_string().as_str())
    );
    assert_eq!(
        head_res.header_value("content-type"),
        get_res.header_value("content-type")
    );
    let (status, _, _) = send(&app, Request::new("HEAD", "/feed/wrong.ics"));
    assert_eq!(status, 404);
    for method in ["POST", "PUT", "DELETE"] {
        let (status, _, _) = send(&app, Request::new(method, &path));
        assert_eq!(status, 404, "{method}");
    }
}

// ---- create input ------------------------------------------------------------

#[test]
fn bad_create_bodies_are_rejected_and_create_nothing() {
    let mut app = test_app();
    app.limits.creates_per_client_hour = 1000;
    let post = |ctype: &str, body: &[u8]| {
        send(
            &app,
            Request::new("POST", "/calendars")
                .with_header("content-type", ctype)
                .with_header("accept", "application/json")
                .with_body(body.to_vec()),
        )
        .0
    };
    let json = "application/json";
    for bad in [
        &b"{not json"[..],
        b"[",
        b"[]",
        b"\"just a string\"",
        b"42",
        b"{\"name\": 5}",
        b"{\"name\": [\"a\"]}",
        b"\xff\xfe",
    ] {
        assert_eq!(post(json, bad), 400, "{:?}", String::from_utf8_lossy(bad));
    }
    assert_eq!(app.db.lock().unwrap().count_calendars().unwrap(), 1, "only home");

    // Still fine: nothing at all, blanks, an object, a null name, a form.
    assert_eq!(post(json, b""), 201);
    assert_eq!(post(json, b"  \n"), 201);
    assert_eq!(post(json, b"{}"), 201);
    assert_eq!(post(json, b"{\"name\": null}"), 201);
    assert_eq!(post(json, b"{\"name\": \"Roadmap\"}"), 201);
    assert_eq!(post("application/x-www-form-urlencoded", b"name=Trip"), 201);
    assert_eq!(post("text/plain", b"whatever"), 201);
}

// ---- key hashing -------------------------------------------------------------

fn create_json(app: &AppState) -> (String, String) {
    let (status, res) = create(app, None);
    assert_eq!(status, 201);
    let v = parse(std::str::from_utf8(&res.body).unwrap()).unwrap();
    (
        v.get("id").and_then(Value::as_str).unwrap().to_string(),
        v.get("key").and_then(Value::as_str).unwrap().to_string(),
    )
}

#[test]
fn the_key_is_shown_at_creation_and_never_again() {
    let app = test_app();
    let (id, key) = create_json(&app);

    // Authenticates, so the hash of the shown key is what was stored.
    let info = |bearer: &str| {
        send(
            &app,
            Request::new("GET", &format!("/v1/c/{id}"))
                .with_header("authorization", &format!("Bearer {bearer}")),
        )
    };
    let (status, body, _) = info(&key);
    assert_eq!(status, 200);
    let text = String::from_utf8(body).unwrap();
    let v = parse(&text).unwrap();
    assert!(v.get("key").is_none(), "GET must not return the key: {text}");
    assert!(!text.contains(&key), "the key leaked into {text}");
    assert_eq!(v.get("id").and_then(Value::as_str), Some(id.as_str()));

    assert_eq!(info("wrong").0, 401);
}

#[test]
fn a_row_with_no_hash_never_authenticates() {
    let dir = std::env::temp_dir().join(format!("almanac-nohash-{}", hex_encode(&random_bytes(8))));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("calendar.db");
    let db = Db::open(&path).unwrap();
    db.ensure_home_calendar(FEED, KEY, "My Calendar").unwrap();
    let app = AppState::new(db, BASE.into());
    let (id, _) = create_json(&app);

    let conn = almanac::sqlite::Connection::open(&path).unwrap();
    conn.execute_batch("UPDATE calendars SET key_hash = ''").unwrap();

    for header in [None, Some(""), Some("Bearer "), Some("Bearer x")] {
        let mut req = Request::new("GET", &format!("/v1/c/{id}/events"));
        if let Some(h) = header {
            req = req.with_header("authorization", h);
        }
        assert_eq!(send(&app, req).0, 401, "{header:?}");
    }
}

#[test]
fn llms_txt_documents_todos() {
    let (_, body, _) = send(&test_app(), Request::new("GET", "/llms.txt"));
    let text = String::from_utf8(body).unwrap();
    assert!(text.contains("/v1/c/{id}/todos"));
    assert!(text.contains("todos.subscribe"));
    assert!(text.contains("/v1/c/{id}/notes"));
    assert!(text.contains("notes.subscribe"));
}
