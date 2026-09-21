use crate::db::{json_calendar, CalendarRow};
use crate::json::Value;

fn esc(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn json_index(base: &str) -> Value {
    let origin = base.trim_end_matches('/');
    Value::object(&[
        ("name", Value::String("Almanac".into())),
        (
            "description",
            Value::String(
                "Agent-first calendar. POST /calendars. Humans subscribe to the https feed URL."
                    .into(),
            ),
        ),
        ("create", Value::String(format!("POST {origin}/calendars"))),
        ("docs", Value::String(format!("{origin}/llms.txt"))),
    ])
}

pub fn llms_txt(base: &str) -> String {
    let origin = base.trim_end_matches('/');
    [
        "# Almanac",
        "",
        "Agent-first ICS calendar. Agents write events. Humans subscribe to the feed URL.",
        "",
        "Send a descriptive User-Agent header, for example Mozilla/5.0 my-agent. The host answers 403 to an empty or default curl User-Agent.",
        "",
        &format!("POST {origin}/calendars"),
        "Optional JSON body: { \"name\": \"Optional Title\" }",
        "Returns id, subscribe URL, write URL, todos and notes URLs, and key. Store the key. It is the write secret, shown once. A lost key cannot be recovered. The same key writes events, todos and notes.",
        "",
        "Write (idempotent):",
        &format!("PUT {origin}/v1/c/{{id}}/events/{{uid}}"),
        "Authorization: Bearer {key}",
        "Content-Type: application/json",
        "{ \"summary\": \"Dentist\", \"start\": \"2026-08-18T09:00:00-07:00\", \"end\": \"2026-08-18T09:45:00-07:00\" }",
        "",
        "List: GET /v1/c/{id}/events",
        "Patch: PATCH /v1/c/{id}/events/{uid}",
        "Delete: DELETE /v1/c/{id}/events/{uid}",
        "",
        "Recurrence (optional on PUT and PATCH):",
        "rrule: FREQ=WEEKLY;BYDAY=TU   (no RRULE: prefix; omit or \"\" for one event)",
        "timeZone: America/Los_Angeles  (required on a timed series; forbidden on all-day and on one-off events)",
        "Allowed timeZone: UTC, America/Los_Angeles, America/Denver, America/Chicago, America/New_York, America/Phoenix, Pacific/Honolulu, Europe/London, Europe/Paris, Asia/Tokyo, Australia/Sydney",
        "Parts: FREQ (DAILY WEEKLY MONTHLY YEARLY), INTERVAL, COUNT or UNTIL, BYDAY, BYMONTHDAY, BYMONTH.",
        "A series is one uid. The feed emits RRULE. Calendar apps expand it.",
        "Skip one date: PUT /v1/c/{id}/events/{uid}/exdates  { \"recurrenceId\": \"2026-10-06T09:00:00\" }",
        "Restore it: DELETE that URL with the same body.",
        "Change one date: PUT /v1/c/{id}/events/{uid}/overrides  { \"recurrenceId\": \"...\", \"start\": \"...\", \"end\": \"...\" }",
        "Drop that change: DELETE that URL with { \"recurrenceId\": \"...\" }.",
        "recurrenceId is the original start: YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS. Not a new uid.",
        "Same recurrenceId cannot be both an exdate and an override. The later PUT wins.",
        "Clearing rrule, or flipping allDay, fails while an exception exists.",
        "",
        "Subscribe: GET {subscribe} as text/calendar. No bearer. Use https, not webcal.",
        "Calendars poll. Refresh if a new event is missing.",
        "",
        "Todos (same key, own feed):",
        &format!("PUT {origin}/v1/c/{{id}}/todos/{{uid}}"),
        "{ \"title\": \"Buy milk\", \"due\": \"2026-10-01\", \"priority\": 1, \"tags\": [\"home\"] }",
        "POST /v1/c/{id}/todos mints a uid. List: GET /v1/c/{id}/todos?status=open|done&tag=home",
        "Get, PATCH, DELETE: /v1/c/{id}/todos/{uid}",
        "title (required, <=512). description (<=4000). due: YYYY-MM-DD or ISO-8601 datetime, null clears. priority: 0-9 (1 highest, 0 none). done: true stamps completedAt, false clears it. tags: up to 10 of [a-z0-9_-], <=32 chars.",
        "PUT keeps any field you leave out. Send null or \"\" to clear one. Wrong types are a 400.",
        "Todos feed: GET the todos.subscribe URL from POST /calendars or GET /v1/c/{id}. text/calendar VTODOs. No bearer. Its token differs from the event feed.",
        "",
        "Notes (same key, own feed):",
        &format!("PUT {origin}/v1/c/{{id}}/notes/{{uid}}"),
        "{ \"title\": \"Ideas\", \"body\": \"# Ideas\\n- one\", \"tags\": [\"work\"], \"pinned\": true }",
        "POST /v1/c/{id}/notes mints a uid. Get, PATCH, DELETE: /v1/c/{id}/notes/{uid}",
        "List: GET /v1/c/{id}/notes?q=text&tag=work&body=true. The list leaves bodies out unless body=true. q matches title or body, ASCII case-insensitive.",
        "title (required, <=200). body: markdown text, <=32KB. tags: as todos. pinned: true lists it first. Newest first otherwise.",
        "PUT keeps any field you leave out. Send null to clear body or tags. Wrong types are a 400. Caps: 500 notes per calendar (409).",
        "Notes feed: GET the notes.subscribe URL. Atom, newest 200. No bearer. Its token differs from the other feeds.",
        "",
        "Export (your data is never locked in):",
        "GET /v1/c/{id}/export?format=json|md   Authorization: Bearer {key}",
        "Everything in the calendar: events, todos, and notes with bodies. 10 per hour.",
        "",
    ]
    .join("\n")
}

const DESCRIPTION: &str =
    "An agent calendar. Agents write events. You subscribe in the app you already have.";

fn page(title: &str, body: &str, origin: &str) -> String {
    let image = if origin.is_empty() {
        "/og.png".to_string()
    } else {
        format!("{origin}/og.png")
    };
    let og_url = if origin.is_empty() {
        String::new()
    } else {
        format!("  <meta property=\"og:url\" content=\"{}\">\n", esc(origin))
    };
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title}</title>
  <meta name="description" content="{desc}">
  <meta property="og:type" content="website">
  <meta property="og:title" content="{title}">
  <meta property="og:description" content="{desc}">
  <meta property="og:image" content="{image}">
  <meta property="og:image:width" content="1200">
  <meta property="og:image:height" content="630">
  <meta property="og:image:alt" content="Almanac. An Agent Calendar.">
{og_url}  <meta name="twitter:card" content="summary_large_image">
  <meta name="twitter:title" content="{title}">
  <meta name="twitter:description" content="{desc}">
  <meta name="twitter:image" content="{image}">
  <style>
    :root {{ color-scheme: dark; }}
    body {{ font: 16px/1.45 ui-sans-serif, system-ui, sans-serif; max-width: 40rem; margin: 3rem auto; padding: 0 1.25rem; color: #e8e8e8; background: #111; }}
    .og {{ display: block; width: 100%; height: auto; margin: 0 0 1.25rem; }}
    h1 {{ font-size: 1.5rem; font-weight: 600; letter-spacing: -0.02em; margin-bottom: 0.25rem; }}
    .sub {{ margin: 0 0 1rem; color: #888; }}
    code, pre {{ font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 0.85rem; }}
    pre {{ background: #1c1c1c; padding: 0.9rem 1rem; overflow-x: auto; }}
    a {{ color: #c8c8c8; }}
    label {{ display: block; margin: 1rem 0 0.35rem; font-size: 0.85rem; }}
    input {{ width: 100%; box-sizing: border-box; padding: 0.5rem 0.6rem; background: #1c1c1c; color: inherit; border: 1px solid #333; }}
    button {{ margin-top: 0.9rem; margin-right: 0.5rem; padding: 0.5rem 0.9rem; background: #e8e8e8; color: #111; border: 0; font-weight: 600; }}
    button.secondary {{ background: #1c1c1c; color: #e8e8e8; border: 1px solid #333; }}
    .muted {{ color: #888; }}
    #prompt {{ margin-top: 1.5rem; white-space: pre-wrap; }}
  </style>
</head>
<body>
{body}
</body>
</html>
"#,
        title = esc(title),
        desc = esc(DESCRIPTION),
        image = esc(&image),
        og_url = og_url,
        body = body,
    )
}

pub fn html_home(base: &str) -> String {
    let origin = base.trim_end_matches('/');
    let prompt = [
        "Almanac is a hosted ICS calendar. You create one over HTTP. I subscribe to the feed in my own calendar app. You write events later when I ask.",
        "",
        &format!("1. GET {origin}/llms.txt and follow it."),
        "2. Create a calendar:",
        &format!("   POST {origin}/calendars"),
        "   Content-Type: application/json",
        "   Body may be {} or { \"name\": \"optional title\" }",
        "3. The JSON response has:",
        "   - id",
        "   - subscribe  (https://…/feed/<token>.ics)",
        "   - write",
        "   - key",
        "4. Store id and key. The key is shown once and cannot be recovered. You need both to add or change events.",
        "5. Give me those four values in plain text.",
        "6. Help me subscribe. The feed is HTTPS ICS. Do not use webcal://.",
        "   Walk me through my calendar of choice:",
        "   - Apple Calendar: File → New Calendar Subscription → paste subscribe",
        "   - Google Calendar: Settings → Add calendar → From URL → paste subscribe",
        "   - Outlook: Add calendar → Subscribe from web → paste subscribe",
        "   - Anything else that accepts an https .ics URL: paste subscribe there",
        "7. Do not add events unless I ask. Setup is create + subscribe only.",
        "",
        &format!("Base: {origin}"),
    ]
    .join("\n");
    page(
        "Almanac",
        &format!(
            r#"
  <img class="og" src="/og.png" width="1200" height="630" alt="Almanac. An Agent Calendar.">
  <h1>Almanac</h1>
  <p class="sub">An Agent Calendar</p>
  <p>Send your agent here. <a href="/llms.txt">/llms.txt</a></p>
  <pre id="prompt">{}</pre>
  <button type="button" class="secondary" id="copy">Copy Prompt</button>
  <script>
    document.getElementById('copy').onclick = function () {{
      navigator.clipboard.writeText(document.getElementById('prompt').textContent);
    }};
  </script>
"#,
            esc(&prompt)
        ),
        origin,
    )
}

pub fn html_created(
    row: &CalendarRow,
    key: &str,
    todos_subscribe: &str,
    notes_subscribe: &str,
    base: &str,
) -> String {
    let creds = json_calendar(row, base);
    let subscribe = creds
        .get("subscribe")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let write = creds
        .get("write")
        .and_then(Value::as_str)
        .unwrap_or_default();
    page(
        "Almanac Calendar",
        &format!(
            r#"
  <h1>Calendar Ready</h1>
  <p>Save the key now. It is shown once and cannot be recovered.</p>
  <p><strong>Id</strong><br><code>{}</code></p>
  <p><strong>Subscribe</strong><br><code>{}</code></p>
  <p><strong>Todos feed</strong><br><code>{}</code></p>
  <p><strong>Notes feed</strong><br><code>{}</code></p>
  <p><strong>Key</strong><br><code>{}</code></p>
  <p><strong>Write</strong><br><code>PUT {}/{{uid}}</code></p>
  <pre>Authorization: Bearer {}</pre>
  <p><a href="/">Back</a></p>
"#,
            esc(&row.id),
            esc(subscribe),
            esc(todos_subscribe),
            esc(notes_subscribe),
            esc(key),
            esc(write),
            esc(key),
        ),
        base.trim_end_matches('/'),
    )
}
