use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rust_embed::Embed;
use serde_json::{json, Value};

use crate::db::{json_calendar, json_event, parse_event_input, parse_event_patch, CalendarRow, Db};
use crate::ics::{render_calendar, IcsEvent};
use crate::landing::{html_created, html_home, json_index, llms_txt};

#[derive(Embed)]
#[folder = "public"]
struct Public;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<Db>>,
    pub public_base: String,
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/health", get(health))
        .route("/robots.txt", get(robots))
        .route("/llms.txt", get(llms))
        .route(
            "/og.png",
            get(|| async { static_file("og.png", "image/png") }),
        )
        .route(
            "/mascot.webp",
            get(|| async { static_file("mascot.webp", "image/webp") }),
        )
        .route("/calendars", post(create_calendar))
        .route("/feed/{token}", get(feed))
        .route("/v1/c/{id}", get(calendar_info))
        .route("/v1/c/{id}/events", get(list_events).post(post_event))
        .route(
            "/v1/c/{id}/events/{uid}",
            get(get_event)
                .put(put_event)
                .patch(patch_event)
                .delete(delete_event),
        )
        .route("/v1/events", get(home_list).post(home_post))
        .route(
            "/v1/events/{uid}",
            get(home_get)
                .put(home_put)
                .patch(home_patch)
                .delete(home_delete),
        )
        .layer(middleware::from_fn(log_req))
        .with_state(state)
}

async fn log_req(req: Request<Body>, next: Next) -> Response {
    let t0 = Instant::now();
    let method = req.method().to_string();
    let ua = req
        .headers()
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let path = redact_feed(req.uri().path());
    let res = next.run(req).await;
    let status = res.status().as_u16();
    println!(
        "{}",
        json!({
            "t": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "msg": "req",
            "method": method,
            "path": path,
            "status": status,
            "ms": t0.elapsed().as_millis(),
            "ua": ua,
        })
    );
    res
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

fn static_file(name: &str, ctype: &'static str) -> Response {
    match Public::get(name) {
        Some(file) => ([(header::CONTENT_TYPE, ctype)], file.data.into_owned()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

async fn robots() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "User-agent: *\nDisallow: /\n",
    )
}

async fn llms(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let base = request_base(&headers, &state.public_base);
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        llms_txt(&base),
    )
}

async fn home(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let base = request_base(&headers, &state.public_base);
    if wants_html(&headers) {
        axum::response::Html(html_home(&base)).into_response()
    } else {
        Json(json_index(&base)).into_response()
    }
}

async fn create_calendar(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: BytesOrEmpty,
) -> Response {
    let ctype = header_str(&headers, header::CONTENT_TYPE);
    let name = parse_calendar_name(&ctype, &body.0);
    let db = state.db.lock().expect("db");
    let saved = match db.create_calendar(name.as_deref(), None, None, None) {
        Ok(row) => row,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &e),
    };
    let base = request_base(&headers, &state.public_base);
    if wants_html(&headers) || ctype.contains("application/x-www-form-urlencoded") {
        return (
            StatusCode::CREATED,
            axum::response::Html(html_created(&saved, &base)),
        )
            .into_response();
    }
    (StatusCode::CREATED, Json(json_calendar(&saved, &base))).into_response()
}

struct BytesOrEmpty(Vec<u8>);

impl<S: Send + Sync> axum::extract::FromRequest<S> for BytesOrEmpty {
    type Rejection = std::convert::Infallible;

    async fn from_request(req: Request<Body>, _state: &S) -> Result<Self, Self::Rejection> {
        let bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024)
            .await
            .unwrap_or_default();
        Ok(Self(bytes.to_vec()))
    }
}

fn parse_calendar_name(ctype: &str, body: &[u8]) -> Option<String> {
    if ctype.contains("application/json") {
        let value: Value = serde_json::from_slice(body).ok()?;
        return value
            .as_object()
            .and_then(|o| o.get("name"))
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

async fn feed(State(state): State<AppState>, Path(token): Path<String>) -> Response {
    let token = feed_token_of(&token);
    let db = state.db.lock().expect("db");
    let cal = match db.get_calendar_by_feed_token(&token) {
        Ok(Some(cal)) if safe_equal(&token, &cal.feed_token) => cal,
        _ => return json_error(StatusCode::NOT_FOUND, "not found"),
    };
    let events = match db.list_events(&cal.id) {
        Ok(rows) => rows,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
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
        })
        .collect();
    let body = render_calendar(&cal.name, &ics_events);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/calendar; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
            (
                header::CONTENT_DISPOSITION,
                "inline; filename=\"calendar.ics\"",
            ),
        ],
        body,
    )
        .into_response()
}

async fn calendar_info(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let db = state.db.lock().expect("db");
    let Some(cal) = calendar_auth(&db, &id, headers.get(header::AUTHORIZATION)) else {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    let base = request_base(&headers, &state.public_base);
    Json(json_calendar(&cal, &base)).into_response()
}

async fn list_events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let db = state.db.lock().expect("db");
    let Some(cal) = calendar_auth(&db, &id, headers.get(header::AUTHORIZATION)) else {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    match db.list_events(&cal.id) {
        Ok(rows) => Json(json!({ "events": rows.iter().map(json_event).collect::<Vec<_>>() }))
            .into_response(),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn get_event(
    State(state): State<AppState>,
    Path((id, uid)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let db = state.db.lock().expect("db");
    let Some(cal) = calendar_auth(&db, &id, headers.get(header::AUTHORIZATION)) else {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    match db.get_event(&cal.id, &uid) {
        Ok(Some(row)) => Json(json_event(&row)).into_response(),
        Ok(None) => json_error(StatusCode::NOT_FOUND, "not found"),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn post_event(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let db = state.db.lock().expect("db");
    let Some(cal) = calendar_auth(&db, &id, headers.get(header::AUTHORIZATION)) else {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    upsert_response(&db, &cal.id, &body, None)
}

async fn put_event(
    State(state): State<AppState>,
    Path((id, uid)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let db = state.db.lock().expect("db");
    let Some(cal) = calendar_auth(&db, &id, headers.get(header::AUTHORIZATION)) else {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    upsert_response(&db, &cal.id, &body, Some(uid))
}

async fn patch_event(
    State(state): State<AppState>,
    Path((id, uid)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let db = state.db.lock().expect("db");
    let Some(cal) = calendar_auth(&db, &id, headers.get(header::AUTHORIZATION)) else {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    let patch = match parse_event_patch(&body) {
        Ok(p) => p,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &e),
    };
    match db.patch_event(&cal.id, &uid, &patch) {
        Ok(row) => Json(json_event(&row)).into_response(),
        Err(e) if e == "not found" => json_error(StatusCode::NOT_FOUND, &e),
        Err(e) => json_error(StatusCode::BAD_REQUEST, &e),
    }
}

async fn delete_event(
    State(state): State<AppState>,
    Path((id, uid)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let db = state.db.lock().expect("db");
    let Some(cal) = calendar_auth(&db, &id, headers.get(header::AUTHORIZATION)) else {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    match db.delete_event(&cal.id, &uid) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => json_error(StatusCode::NOT_FOUND, "not found"),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

fn home_auth(db: &Db, headers: &HeaderMap) -> Option<CalendarRow> {
    let home = db.get_calendar("home").ok().flatten()?;
    let given = bearer(headers.get(header::AUTHORIZATION));
    if safe_equal(&given, &home.agent_key) {
        Some(home)
    } else {
        None
    }
}

async fn home_list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let db = state.db.lock().expect("db");
    if home_auth(&db, &headers).is_none() {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    match db.list_events("home") {
        Ok(rows) => Json(json!({ "events": rows.iter().map(json_event).collect::<Vec<_>>() }))
            .into_response(),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn home_get(
    State(state): State<AppState>,
    Path(uid): Path<String>,
    headers: HeaderMap,
) -> Response {
    let db = state.db.lock().expect("db");
    if home_auth(&db, &headers).is_none() {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    match db.get_event("home", &uid) {
        Ok(Some(row)) => Json(json_event(&row)).into_response(),
        Ok(None) => json_error(StatusCode::NOT_FOUND, "not found"),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn home_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let db = state.db.lock().expect("db");
    if home_auth(&db, &headers).is_none() {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    upsert_response(&db, "home", &body, None)
}

async fn home_put(
    State(state): State<AppState>,
    Path(uid): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let db = state.db.lock().expect("db");
    if home_auth(&db, &headers).is_none() {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    upsert_response(&db, "home", &body, Some(uid))
}

async fn home_patch(
    State(state): State<AppState>,
    Path(uid): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let db = state.db.lock().expect("db");
    if home_auth(&db, &headers).is_none() {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let patch = match parse_event_patch(&body) {
        Ok(p) => p,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &e),
    };
    match db.patch_event("home", &uid, &patch) {
        Ok(row) => Json(json_event(&row)).into_response(),
        Err(e) if e == "not found" => json_error(StatusCode::NOT_FOUND, &e),
        Err(e) => json_error(StatusCode::BAD_REQUEST, &e),
    }
}

async fn home_delete(
    State(state): State<AppState>,
    Path(uid): Path<String>,
    headers: HeaderMap,
) -> Response {
    let db = state.db.lock().expect("db");
    if home_auth(&db, &headers).is_none() {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    match db.delete_event("home", &uid) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => json_error(StatusCode::NOT_FOUND, "not found"),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

fn upsert_response(db: &Db, calendar_id: &str, body: &Value, uid: Option<String>) -> Response {
    let mut input = match parse_event_input(body) {
        Ok(i) => i,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &e),
    };
    if let Some(uid) = uid {
        input.uid = Some(uid);
    }
    match db.upsert_event(calendar_id, &input) {
        Ok(saved) => {
            let status = if saved.sequence == 0 {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            (status, Json(json_event(&saved))).into_response()
        }
        Err(e) => json_error(StatusCode::BAD_REQUEST, &e),
    }
}

fn calendar_auth(
    db: &Db,
    id: &str,
    header: Option<&axum::http::HeaderValue>,
) -> Option<CalendarRow> {
    let cal = db.get_calendar(id).ok().flatten()?;
    if safe_equal(&bearer(header), &cal.agent_key) {
        Some(cal)
    } else {
        None
    }
}

fn bearer(header: Option<&axum::http::HeaderValue>) -> String {
    let Some(raw) = header.and_then(|v| v.to_str().ok()) else {
        return String::new();
    };
    raw.strip_prefix("Bearer ").unwrap_or("").to_string()
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

fn request_base(headers: &HeaderMap, fallback: &str) -> String {
    if !fallback.is_empty() {
        return fallback.trim_end_matches('/').to_string();
    }
    let proto = header_str(headers, "x-forwarded-proto");
    let proto = if proto.is_empty() {
        "http"
    } else {
        proto.as_str()
    };
    let host = header_str(headers, header::HOST);
    let host = if host.is_empty() {
        "localhost"
    } else {
        host.as_str()
    };
    format!("{proto}://{host}")
}

fn wants_html(headers: &HeaderMap) -> bool {
    let accept = header_str(headers, header::ACCEPT);
    accept.contains("text/html") && !accept.contains("application/json")
}

fn header_str(headers: &HeaderMap, name: impl axum::http::header::AsHeaderName) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

fn json_error(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}
