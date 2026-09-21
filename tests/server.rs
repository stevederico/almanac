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
