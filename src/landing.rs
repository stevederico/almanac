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
                "Agent-first calendar, todos and notes on one key. POST /calendars. Humans subscribe to the https feed URLs."
                    .into(),
            ),
        ),
        ("create", Value::String(format!("POST {origin}/calendars"))),
        (
            "resources",
            Value::object(&[
                ("events", Value::String(format!("{origin}/v1/c/{{id}}/events"))),
                ("todos", Value::String(format!("{origin}/v1/c/{{id}}/todos"))),
                ("notes", Value::String(format!("{origin}/v1/c/{{id}}/notes"))),
                ("export", Value::String(format!("{origin}/v1/c/{{id}}/export"))),
            ]),
        ),
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
        "Optional JSON body: { \"name\": \"Optional Title\", \"feed\": \"seal\" | \"plain\" }. New calendars default to seal.",
        "Returns id, subscribe URL, write URL, todos and notes URLs, and key. Store the key. It is the write secret, shown once. A lost key cannot be recovered. The same key writes events, todos and notes.",
        "Delete a calendar: DELETE /v1/c/{id} with the same Authorization. 204 when it is gone, including its events, todos, notes and feeds. A missing id or a wrong key is 401. The home calendar cannot be deleted. Do not create or delete a calendar unless asked.",
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
        "Seals: a seal calendar stores each event, todo, and note as one alm1. ciphertext. PUT { \"seal\": \"alm1....\" } with the uid in the path. The server does not have the write key, so it cannot read or merge fields inside a seal. GET returns uid, seal, createdAt, updatedAt. A sealed feed is a shell: UID and X-ALMANAC-SEAL, with no SUMMARY, DTSTART, DUE, or note text. Open it with the write key. A plain calendar is the field API and the readable ICS, VTODO, and Atom feeds. PATCH /v1/c/{id} { \"feed\": \"seal\" | \"plain\" } sets the mode. The server cannot convert existing rows either way.",
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
    "An agent-first calendar, notes and todos. Your agent writes them. You subscribe in the app you already use.";

const PAGE: &str = r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>@TITLE@</title>
  <meta name="description" content="@DESC@">
  <meta property="og:type" content="website">
  <meta property="og:title" content="@TITLE@">
  <meta property="og:description" content="@DESC@">
  <meta property="og:image" content="@IMAGE@">
  <meta property="og:image:width" content="1200">
  <meta property="og:image:height" content="630">
  <meta property="og:image:alt" content="Almanac. Agent-first calendar, notes and todos.">
@OG_URL@  <meta name="twitter:card" content="summary_large_image">
  <meta name="twitter:title" content="@TITLE@">
  <meta name="twitter:description" content="@DESC@">
  <meta name="twitter:image" content="@IMAGE@">
  <style>
    :root { color-scheme: dark; }
    * { box-sizing: border-box; }
    body { margin: 0; background: #111210; color: #e9e6da; font: 16px/1.5 ui-monospace, "IBM Plex Mono", SFMono-Regular, Menlo, Consolas, monospace; }
    a { color: #e9e6da; }
    a:hover { color: #f2b84b; }
    code, pre { font-family: inherit; }
    pre { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
    button { font: inherit; font-size: 13px; font-weight: 600; min-height: 44px; padding: 0 16px; background: #f2b84b; color: #111210; border: 0; cursor: pointer; }
    .wrap { max-width: 1216px; margin: 0 auto; padding: 0 clamp(20px, 5vw, 64px); }
    .narrow { max-width: 46rem; padding-top: 3rem; padding-bottom: 3rem; }
    .narrow h1 { font-size: 1.6rem; font-weight: 500; letter-spacing: -0.02em; margin: 0 0 1rem; }
    .narrow pre { background: #171814; border: 1px solid #34362d; padding: 0.9rem 1rem; }
    .narrow code { font-size: 0.85rem; }
    .muted { color: #8f8b7e; }
  </style>
</head>
<body>
@BODY@
</body>
</html>
"##;

/// Cache key for the share image. Cloudflare holds `/og.png` for hours and
/// link-preview crawlers cache by URL, so a new image at the same URL keeps
/// showing the old one. Bump this whenever `public/og.png` changes.
const OG_VERSION: &str = "3";

fn page(title: &str, body: &str, origin: &str) -> String {
    let image = if origin.is_empty() {
        format!("/og.png?v={OG_VERSION}")
    } else {
        format!("{origin}/og.png?v={OG_VERSION}")
    };
    let og_url = if origin.is_empty() {
        String::new()
    } else {
        format!("  <meta property=\"og:url\" content=\"{}\">\n", esc(origin))
    };
    PAGE.replace("@TITLE@", &esc(title))
        .replace("@DESC@", &esc(DESCRIPTION))
        .replace("@IMAGE@", &esc(&image))
        .replace("@OG_URL@", &og_url)
        .replace("@BODY@", body)
}

/// Landing-page styles. Colours and sizes follow the terminal design; the
/// three product mockups below are plain HTML and CSS, no images.
const HOME_CSS: &str = r##"
  .bar { height: 72px; display: flex; align-items: center; justify-content: space-between; border-bottom: 1px solid #2a2c25; }
  .word { font-size: 18px; font-weight: 600; letter-spacing: 0.02em; }
  .word i { font-style: normal; color: #f2b84b; }
  .bar a { font-size: 14px; text-decoration: none; border-bottom: 1px solid #55574c; }
  .hero { padding-top: clamp(48px, 9vw, 112px); display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: clamp(32px, 5vw, 64px); align-items: center; }
  h1 { margin: 0; font-size: clamp(34px, 4.2vw, 54px); line-height: 1.04; letter-spacing: -0.035em; font-weight: 500; }
  .box { border: 1px solid #34362d; background: #171814; }
  .box-h { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 12px 16px; border-bottom: 1px solid #34362d; font-size: 13px; color: #8f8b7e; }
  .prompt { padding: 24px 24px 28px; font-size: 15px; line-height: 1.7; color: #d4d0c2; }
  .products { margin-top: clamp(64px, 9vw, 120px); padding-bottom: clamp(48px, 7vw, 96px); }
  .cols { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 300px), 1fr)); gap: 48px; align-items: start; }
  .cols figure { margin: 0; max-width: 420px; }
  .cols h3 { margin: 20px 0 0; font-size: 20px; font-weight: 500; }
  .app { height: 340px; overflow: hidden; background: #171814; color: #e9e6da; border: 1px solid #34362d; font-size: 12px; }
  .app-h { height: 44px; padding: 0 14px; display: flex; align-items: center; justify-content: space-between; border-bottom: 1px solid #34362d; }
  .app-h b { font-size: 15px; font-weight: 600; }
  .app-h.lg { height: 60px; padding: 0 16px; }
  .app-h.lg b { font-size: 20px; }
  .cal-days, .cal-allday, .cal-body { display: grid; grid-template-columns: 36px repeat(5, minmax(0, 1fr)); }
  .cal-days { height: 30px; align-items: center; border-bottom: 1px solid #34362d; color: #8f8b7e; font-size: 11px; }
  .cal-days span { text-align: center; }
  .cal-days b { color: #e9e6da; font-weight: 600; }
  .cal-allday { height: 24px; align-items: center; border-bottom: 1px solid #34362d; }
  .trip { grid-column: 6; height: 18px; margin: 0 2px; padding: 0 6px; background: #f2b84b; color: #111210; font-size: 11px; line-height: 18px; font-weight: 600; }
  .cal-body { height: 220px; }
  .hrs { position: relative; }
  .hrs span { position: absolute; left: 4px; font-size: 9px; color: #8f8b7e; }
  .col { position: relative; border-left: 1px solid #34362d; background: repeating-linear-gradient(to bottom, transparent 0, transparent 43px, #34362d 43px, #34362d 44px); }
  .ev { position: absolute; left: 2px; right: 2px; padding: 3px 5px; background: #2f2a17; overflow: hidden; font-size: 11px; line-height: 1.25; }
  .ev b { display: block; font-weight: 600; }
  .ev small { font-size: 10px; color: #8f8b7e; }
  .todos { list-style: none; margin: 0; padding: 0; }
  .todos li { height: 56px; padding: 0 16px; display: grid; grid-template-columns: 18px minmax(0, 1fr) auto; column-gap: 12px; align-items: center; border-bottom: 1px solid #34362d; }
  .todos .cb { width: 18px; height: 18px; border: 1.5px solid #8f8b7e; display: flex; align-items: center; justify-content: center; }
  .todos .done .cb { border-color: #f2b84b; background: #f2b84b; }
  .todos b { display: block; font-size: 15px; font-weight: 500; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .todos .done b { color: #8f8b7e; text-decoration: line-through; }
  .todos small { display: block; margin-top: 2px; font-size: 12px; color: #8f8b7e; }
  .todos em { font-style: normal; font-size: 11px; font-weight: 600; padding: 2px 7px; background: #f2b84b; color: #111210; }
  .search { margin: 12px 16px; height: 36px; padding: 0 12px; display: flex; align-items: center; background: #22241d; border: 1px solid #34362d; font-size: 14px; color: #8f8b7e; }
  .notes { list-style: none; margin: 0; padding: 0; }
  .notes li { height: 91px; padding: 10px 16px 0; border-top: 1px solid #34362d; }
  .notes b { font-size: 16px; font-weight: 600; }
  .notes .pin { display: inline-block; width: 8px; height: 8px; margin-left: 8px; background: #f2b84b; }
  .notes p { margin: 3px 0 0; height: 32px; overflow: hidden; font-size: 12px; line-height: 16px; color: #8f8b7e; }
  .notes .tag { display: inline-block; margin-top: 5px; padding: 1px 7px; background: #22241d; color: #8f8b7e; font-size: 11px; }
  @media (max-width: 860px) { .hero { grid-template-columns: minmax(0, 1fr); } }
"##;

const CALENDAR_MOCK: &str = r##"<div class="app" role="img" aria-label="A calendar app showing one week: a weekly standup on Tuesday, a dentist visit on Wednesday, a design review on Thursday and an all-day trip on Friday.">
<div class="app-h"><b>Sep 21 &ndash; 25</b></div>
<div class="cal-days"><span></span><span>Mon <b>21</b></span><span>Tue <b>22</b></span><span>Wed <b>23</b></span><span>Thu <b>24</b></span><span>Fri <b>25</b></span></div>
<div class="cal-allday"><span class="trip">Trip</span></div>
<div class="cal-body">
<div class="hrs"><span style="top:2px">9a</span><span style="top:46px">10a</span><span style="top:90px">11a</span><span style="top:134px">12p</span><span style="top:178px">1p</span></div>
<div class="col"><div class="ev" style="top:177px;height:42px"><b>Roadmap sync</b></div></div>
<div class="col"><div class="ev" style="top:1px;height:20px"><b>Standup</b></div></div>
<div class="col"><div class="ev" style="top:1px;height:42px"><b>Dentist</b><small>Fillmore</small></div></div>
<div class="col"><div class="ev" style="top:89px;height:64px"><b>Design review</b><small>Room 2</small></div></div>
<div class="col"></div>
</div>
</div>"##;

const TODOS_MOCK: &str = r##"<div class="app" role="img" aria-label="A todo app with four open items and one finished: buy milk, call Bob, renew passport, book flights, and pack bags done.">
<div class="app-h lg"><b>Todos</b></div>
<ul class="todos">
<li><span class="cb"></span><div><b>Buy milk</b><small>due Oct 1 &middot; #home</small></div><em>P1</em></li>
<li><span class="cb"></span><div><b>Call Bob</b><small>due Sep 30, 5 PM</small></div></li>
<li><span class="cb"></span><div><b>Renew passport</b><small>due Oct 14 &middot; #travel</small></div></li>
<li><span class="cb"></span><div><b>Book flights</b><small>#travel</small></div></li>
<li class="done"><span class="cb"><svg width="11" height="11" viewBox="0 0 12 12" aria-hidden="true"><path d="M2 6.5 L5 9.5 L10 3" fill="none" stroke="#111210" stroke-width="2"/></svg></span><div><b>Pack bags</b><small>done Sep 20</small></div></li>
</ul>
</div>"##;

const NOTES_MOCK: &str = r##"<div class="app" role="img" aria-label="A notes app with a search box and three notes: a pinned Ideas note, a Trip plan and Groceries.">
<div class="search">Search notes</div>
<ul class="notes">
<li><b>Ideas</b><span class="pin" aria-hidden="true"></span><p>Pack light. Call Bob about the roadmap. Sketch the landing page.</p><span class="tag">#work</span></li>
<li><b>Trip plan</b><p>Flights booked. Passport renewal due Oct 14. Hotel near the station.</p><span class="tag">#travel</span></li>
<li><b>Groceries</b><p>Milk, eggs, coffee. Ask about the good bread.</p><span class="tag">#home</span></li>
</ul>
</div>"##;

const HOME_BODY: &str = r##"<style>@HOME_CSS@</style>
<div class="wrap">
<header class="bar"><span class="word">almanac<i>_</i></span><a href="/llms.txt">/llms.txt</a></header>
<section class="hero">
<h1>Agent-First Calendar, Notes, and Todos</h1>
<div class="box">
<div class="box-h"><span>Ask your agent</span><button type="button" id="copy">Copy</button></div>
<pre class="prompt" id="prompt">@PROMPT@</pre>
</div>
</section>
<section class="products">
<div class="cols">
<figure>@CALENDAR@<figcaption><h3>Calendar</h3></figcaption></figure>
<figure>@NOTES@<figcaption><h3>Notes</h3></figcaption></figure>
<figure>@TODOS@<figcaption><h3>Todos</h3></figcaption></figure>
</div>
</section>
</div>
<script>
  document.getElementById('copy').onclick = function () {
    navigator.clipboard.writeText(document.getElementById('prompt').textContent);
  };
</script>"##;

pub fn html_home(base: &str) -> String {
    let origin = base.trim_end_matches('/');
    let prompt = [
        "Set up Almanac for me.".to_string(),
        format!("1. Read {origin}/llms.txt and follow it."),
        "2. Create my calendar over HTTP.".to_string(),
        "3. Store the id and key. The key is shown once.".to_string(),
        "4. Give me the three subscribe URLs and help me subscribe.".to_string(),
        "Add nothing until I ask.".to_string(),
    ]
    .join("\n");
    let body = HOME_BODY
        .replace("@HOME_CSS@", HOME_CSS)
        .replace("@PROMPT@", &esc(&prompt))
        .replace("@CALENDAR@", CALENDAR_MOCK)
        .replace("@NOTES@", NOTES_MOCK)
        .replace("@TODOS@", TODOS_MOCK);
    page("Almanac", &body, origin)
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
    let body = format!(
        r#"<div class="wrap narrow">
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
</div>"#,
        esc(&row.id),
        esc(subscribe),
        esc(todos_subscribe),
        esc(notes_subscribe),
        esc(key),
        esc(write),
        esc(key),
    );
    page("Almanac Calendar", &body, base.trim_end_matches('/'))
}
