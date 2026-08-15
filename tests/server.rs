use std::sync::{Arc, Mutex};

use almanac::db::Db;
use almanac::server::{app, AppState};
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const FEED: &str = "feed-secret";
const KEY: &str = "agent-secret";
const BASE: &str = "http://example.test";

fn test_app() -> axum::Router {
    let dir = std::env::temp_dir().join(format!("almanac-srv-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = Db::open(dir.join("calendar.db")).unwrap();
    db.ensure_home_calendar(FEED, KEY, "My Calendar").unwrap();
    app(AppState {
        db: Arc::new(Mutex::new(db)),
        public_base: BASE.into(),
    })
}

async fn send(
    app: axum::Router,
    req: Request<Body>,
) -> (StatusCode, Vec<u8>, axum::http::HeaderMap) {
    let res = app.oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let body = to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, body, headers)
}

fn auth() -> Request<Body> {
    Request::builder()
        .uri("/v1/events")
        .header("authorization", format!("Bearer {KEY}"))
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn returns_json_for_agents() {
    let (status, body, _) = send(
        test_app(),
        Request::builder()
            .uri("/")
            .header("accept", "application/json")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert!(v.get("create").is_some());
}

#[tokio::test]
async fn returns_html_for_browsers() {
    let (status, body, _) = send(
        test_app(),
        Request::builder()
            .uri("/")
            .header("accept", "text/html")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
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

#[tokio::test]
async fn requires_a_user_agent_in_machine_docs() {
    let (status, body, _) = send(
        test_app(),
        Request::builder()
            .uri("/llms.txt")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(body).unwrap();
    assert!(text.contains("User-Agent"));
}

#[tokio::test]
async fn creates_a_calendar_and_returns_subscribe_and_key() {
    let (status, body, _) = send(
        test_app(),
        Request::builder()
            .method("POST")
            .uri("/calendars")
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .body(Body::from(json!({ "name": "Giants" }).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert!(v["id"].as_str().unwrap().starts_with("cal_"));
    assert!(v["key"].as_str().is_some());
    let sub = v["subscribe"].as_str().unwrap();
    assert!(sub.starts_with("http://example.test/feed/"));
    assert!(sub.ends_with(".ics"));
}

#[tokio::test]
async fn isolates_two_calendars() {
    let app = test_app();
    let mk = || {
        Request::builder()
            .method("POST")
            .uri("/calendars")
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .body(Body::from("{}"))
            .unwrap()
    };
    let (_, a_body, _) = send(app.clone(), mk()).await;
    let (_, b_body, _) = send(app.clone(), mk()).await;
    let a: Value = serde_json::from_slice(&a_body).unwrap();
    let b: Value = serde_json::from_slice(&b_body).unwrap();
    let a_id = a["id"].as_str().unwrap();
    let a_key = a["key"].as_str().unwrap();
    let b_id = b["id"].as_str().unwrap();
    let b_key = b["key"].as_str().unwrap();

    let (put_status, _, _) = send(
        app.clone(),
        Request::builder()
            .method("PUT")
            .uri(format!("/v1/c/{a_id}/events/only-a"))
            .header("authorization", format!("Bearer {a_key}"))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "summary": "Only A", "start": "2026-08-16T12:00:00Z" }).to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(put_status, StatusCode::CREATED);

    let (_, listed, _) = send(
        app.clone(),
        Request::builder()
            .uri(format!("/v1/c/{b_id}/events"))
            .header("authorization", format!("Bearer {b_key}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let listed: Value = serde_json::from_slice(&listed).unwrap();
    assert_eq!(listed["events"].as_array().unwrap().len(), 0);

    let (steal, _, _) = send(
        app,
        Request::builder()
            .uri(format!("/v1/c/{a_id}/events"))
            .header("authorization", format!("Bearer {b_key}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(steal, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn feed_404s_a_wrong_token() {
    let (status, _, _) = send(
        test_app(),
        Request::builder()
            .uri("/feed/nope.ics")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn returns_text_calendar_for_the_home_token() {
    let app = test_app();
    let _ = send(
        app.clone(),
        Request::builder()
            .method("POST")
            .uri("/v1/events")
            .header("authorization", format!("Bearer {KEY}"))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "uid": "giants-20260816",
                    "summary": "Rockies @ Giants",
                    "start": "2026-08-16T13:05:00-07:00",
                    "location": "Oracle Park"
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    let (status, body, headers) = send(
        app,
        Request::builder()
            .uri(format!("/feed/{FEED}.ics"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ctype = headers.get("content-type").unwrap().to_str().unwrap();
    assert!(ctype.contains("text/calendar"));
    let text = String::from_utf8(body).unwrap();
    assert!(text.contains("BEGIN:VCALENDAR"));
    assert!(text.contains("UID:giants-20260816"));
    assert!(text.contains("SUMMARY:Rockies @ Giants"));
}

#[tokio::test]
async fn rejects_missing_bearer() {
    let (status, _, _) = send(
        test_app(),
        Request::builder()
            .uri("/v1/events")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn creates_lists_patches_deletes() {
    let app = test_app();
    let (created_status, created_body, _) = send(
        app.clone(),
        Request::builder()
            .method("POST")
            .uri("/v1/events")
            .header("authorization", format!("Bearer {KEY}"))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "summary": "Dentist",
                    "start": "2026-08-18T09:00:00-07:00",
                    "end": "2026-08-18T09:45:00-07:00"
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(created_status, StatusCode::CREATED);
    let created: Value = serde_json::from_slice(&created_body).unwrap();
    let uid = created["uid"].as_str().unwrap();

    let (list_status, list_body, _) = send(app.clone(), auth()).await;
    assert_eq!(list_status, StatusCode::OK);
    let listed: Value = serde_json::from_slice(&list_body).unwrap();
    assert_eq!(listed["events"].as_array().unwrap().len(), 1);

    let (patched, _, _) = send(
        app.clone(),
        Request::builder()
            .method("PATCH")
            .uri(format!("/v1/events/{uid}"))
            .header("authorization", format!("Bearer {KEY}"))
            .header("content-type", "application/json")
            .body(Body::from(json!({ "location": "Fillmore" }).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(patched, StatusCode::OK);

    let (gone, _, _) = send(
        app,
        Request::builder()
            .method("DELETE")
            .uri(format!("/v1/events/{uid}"))
            .header("authorization", format!("Bearer {KEY}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(gone, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn upserts_by_uid_via_put() {
    let app = test_app();
    let (first, _, _) = send(
        app.clone(),
        Request::builder()
            .method("PUT")
            .uri("/v1/events/standup")
            .header("authorization", format!("Bearer {KEY}"))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "summary": "Standup",
                    "start": "2026-08-14T09:30:00-07:00"
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(first, StatusCode::CREATED);
    let (again, body, _) = send(
        app,
        Request::builder()
            .method("PUT")
            .uri("/v1/events/standup")
            .header("authorization", format!("Bearer {KEY}"))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "summary": "Standup moved",
                    "start": "2026-08-14T10:00:00-07:00"
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(again, StatusCode::OK);
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["sequence"], 1);
}
