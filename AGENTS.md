# almanac

Agent-first ICS calendar. Request a calendar. Subscribe the URL. Write events.

## Create

Prod answers `403` to an empty or default-curl User-Agent. Add `-A "Mozilla/5.0 my-agent"` to every `curl` below.

```bash
curl -sS -X POST https://almanac.dottie.ai/calendars \
  -H "Content-Type: application/json" \
  -d '{"name":"Roadmap"}'
```

Success `201`:

```json
{
  "id": "cal_ab12cd34ef56ab78",
  "name": "Roadmap",
  "subscribe": "https://almanac.dottie.ai/feed/….ics",
  "write": "https://almanac.dottie.ai/v1/c/cal_ab12cd34ef56ab78/events",
  "key": "…",
  "createdAt": "2026-08-13T20:00:00.000Z"
}
```

Store `id` and `key`. The key is the write secret. It is shown once and cannot be recovered; only its hash is stored. The same key writes events and todos. The response also has `todos.write`, `todos.subscribe`, `notes.write` and `notes.subscribe`.

## Delete

```bash
curl -sS -X DELETE "$BASE/v1/c/$ID" \
  -H "Authorization: Bearer $KEY"
```

`204` when the calendar is gone, along with its events, todos, notes, and feed URLs. A missing id or a wrong key is `401`. Deleting again is `401`. The `home` calendar cannot be deleted (`403`). Do not create or delete a calendar unless asked.

## Write

```bash
curl -sS -X PUT "$WRITE/dentist-2026-08-18" \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "summary": "Dentist",
    "start": "2026-08-18T09:00:00-07:00",
    "end": "2026-08-18T09:45:00-07:00"
  }'
```

`201` create or `200` update. Same `uid` replaces in place.

## Fields

| Field | Type | Required | Notes |
|---|---|---|---|
| `summary` | string | yes | ≤512 |
| `start` | string | yes | ISO-8601, or `YYYY-MM-DD` for all-day |
| `end` | string | no | Default +1 hour, or next day if all-day |
| `uid` | string | PUT path | `[A-Za-z0-9._@-]{1,200}` |
| `location` | string | no | ≤512 |
| `description` | string | no | ≤4000 |
| `allDay` | bool | no | Or inferred from date-only start |
| `transparent` | bool | no | Default `true` |
| `rrule` | string | no | Repeat rule, no `RRULE:` prefix. Omit or `""` is one event |
| `timeZone` | string | timed series | IANA name. Required when `rrule` is set and the event is not all-day |

## Other

```bash
curl -sS "$WRITE" -H "Authorization: Bearer $KEY"
curl -sS -X PATCH "$WRITE/dentist-2026-08-18" -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" -d '{"location":"Fillmore"}'
curl -sS -X DELETE "$WRITE/dentist-2026-08-18" -H "Authorization: Bearer $KEY"
```

Weekly series. `start` / `end` in the response are the wall clock, not `Z`.

```bash
curl -sS -X PUT "$WRITE/standup" \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "summary": "Standup",
    "start": "2026-09-22T09:00:00-07:00",
    "end": "2026-09-22T09:15:00-07:00",
    "timeZone": "America/Los_Angeles",
    "rrule": "FREQ=WEEKLY;BYDAY=TU"
  }'
```

Skip one date, or replace one date. `recurrenceId` is the original start: `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM:SS`. Not a new uid.

```bash
curl -sS -X PUT "$WRITE/standup/exdates" \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -d '{"recurrenceId":"2026-10-06T09:00:00"}'
curl -sS -X DELETE "$WRITE/standup/exdates" \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -d '{"recurrenceId":"2026-10-06T09:00:00"}'
curl -sS -X PUT "$WRITE/standup/overrides" \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -d '{"recurrenceId":"2026-10-13T09:00:00","start":"2026-10-13T10:00:00","end":"2026-10-13T10:15:00"}'
curl -sS -X DELETE "$WRITE/standup/overrides" \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -d '{"recurrenceId":"2026-10-13T09:00:00"}'
```

## Seals

New calendars default to `feed: "seal"`. Each event, todo, and note is one `alm1.` ciphertext of its JSON, sealed with that calendar's write key. The server stores the blob and the uid. It does not have the key. `PUT` body is `{"seal":"alm1.…"}`. `GET` returns `uid`, `seal`, `createdAt`, `updatedAt`. Feeds for a sealed row are a shell with `UID` and `X-ALMANAC-SEAL` only.

`feed: "plain"` is today's field API and a readable ICS, VTODO, or Atom feed. `PATCH /v1/c/{id}` with `{"feed":"seal"}` or `{"feed":"plain"}` sets the mode. Existing rows stay as they are until a client that has the key rewrites them. `scripts/reseal` does that for one calendar id passed on the command line.

`almanac seal` and `almanac open` read the key from `~/.config/almanac/hosted-calendars.json` and do not print it.

## Todos

Same key, own endpoints, own feed.

```bash
curl -sS -X PUT "$BASE/v1/c/$ID/todos/milk" \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -d '{"title":"Buy milk","due":"2026-10-01","priority":1,"tags":["home"]}'
curl -sS -X POST "$BASE/v1/c/$ID/todos" -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" -d '{"title":"Call Bob"}'
curl -sS "$BASE/v1/c/$ID/todos?status=open&tag=home" -H "Authorization: Bearer $KEY"
curl -sS -X PATCH "$BASE/v1/c/$ID/todos/milk" -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" -d '{"done":true}'
curl -sS -X DELETE "$BASE/v1/c/$ID/todos/milk" -H "Authorization: Bearer $KEY"
```

`201` create or `200` update. `POST` mints a `todo-…` uid. `PUT` keeps fields you leave out; `null` or `""` clears one.

| Field | Type | Notes |
|---|---|---|
| `title` | string | required, ≤512 |
| `description` | string | ≤4000 |
| `due` | string | `YYYY-MM-DD` or ISO-8601 datetime |
| `priority` | int | 0-9, 1 is highest, 0 is none |
| `done` | bool | `true` stamps `completedAt`, `false` clears it |
| `tags` | string[] | ≤10, each `[a-z0-9_-]{1,32}`, lowercased |

Open todos list first, then by due date. `GET /v1/c/{id}/todos` filters with `?status=open|done` and `?tag=`. Caps: 5000 todos per calendar (`409`). Events and todos share one write budget per calendar.

Todos feed: `todos.subscribe` from create or `GET /v1/c/{id}`. `text/calendar` with `VTODO`s. Its token is not the event feed's token.

## Notes

Same key, own endpoints, own feed.

```bash
curl -sS -X PUT "$BASE/v1/c/$ID/notes/ideas" \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -d '{"title":"Ideas","body":"# Ideas\n- one","tags":["work"],"pinned":true}'
curl -sS "$BASE/v1/c/$ID/notes?q=ideas&tag=work" -H "Authorization: Bearer $KEY"
curl -sS "$BASE/v1/c/$ID/notes/ideas" -H "Authorization: Bearer $KEY"
```

`POST /v1/c/{id}/notes` mints a `note-…` uid. `GET`, `PATCH`, `DELETE` on `/v1/c/{id}/notes/{uid}`. `PUT` keeps fields you leave out; `null` clears `body` or `tags`.

| Field | Type | Notes |
|---|---|---|
| `title` | string | required, ≤200 |
| `body` | string | markdown, ≤32KB |
| `tags` | string[] | as todos |
| `pinned` | bool | pinned notes list first |

The list omits bodies. Add `?body=true` for them. `?q=` matches title or body, ASCII case-insensitive, with `%` and `_` literal. Caps: 500 notes per calendar (`409`), one write budget shared with events and todos.

Notes feed: `notes.subscribe`. Atom, newest 200 notes, no bearer, its own token.

## Export

```bash
curl -sS "$BASE/v1/c/$ID/export" -H "Authorization: Bearer $KEY" > almanac.json
curl -sS "$BASE/v1/c/$ID/export?format=md" -H "Authorization: Bearer $KEY" > almanac.md
```

Everything in the calendar: events, todos, notes with bodies. No feed tokens or keys. `format` is `json` (default) or `md`. 10 exports per hour per calendar (`429`).

Subscribe: paste `subscribe` in Calendar → File → New Calendar Subscription. Use `https://`, not `webcal://`.

Machine docs: `GET /llms.txt`. Humans: `GET /`.

Agent skill: `skills/almanac/SKILL.md`. It ships with the API so the two change together. Install by linking it: `ln -sfn "$PWD/skills/almanac" ~/.agents/skills/almanac`. It reads the default calendar and key from `~/.config/almanac/hosted-calendars.json`, so it holds no secret and no calendar id. `tests/skill.rs` enforces that.

Secrets stay out of the repo. Enable the commit hook once per clone: `git config core.hooksPath scripts/hooks`. It blocks a commit or a commit message that holds a 48-character hex string (the size of a key or feed token) or any credential from `~/.config/almanac/hosted-calendars.json`, and never prints what it found. `tests/secrets.rs` runs the same checks over every tracked file and confirms `.env`, `*.db`, backups and the credentials file are git-ignored.

## Errors

`401` `{"error":"unauthorized"}` — missing or wrong key.
`400` `{"error":"summary is required"}` — send `summary` + `start`.
`404` `{"error":"not found"}` — bad uid or bad feed token.

## Rules

- Stable `uid` per event. New uid = duplicate.
- Timed events need a timezone offset.
- All-day: `"start": "2026-08-20"`.
- A series is one uid plus `rrule`. The feed emits `RRULE`. Calendar apps expand it.
- Timed series need `timeZone`: `UTC`, `America/Los_Angeles`, `America/Denver`, `America/Chicago`, `America/New_York`, `America/Phoenix`, `Pacific/Honolulu`, `Europe/London`, `Europe/Paris`, `Asia/Tokyo`, `Australia/Sydney`.
- `RRULE` parts: `FREQ` (`DAILY`, `WEEKLY`, `MONTHLY`, `YEARLY`), `INTERVAL`, `COUNT` or `UNTIL`, `BYDAY`, `BYMONTHDAY`, `BYMONTH`.
- One occurrence is an exdate or an override on that uid, not a new uid.
- Clearing `rrule`, or flipping `allDay`, fails while an exception exists. On a timed series, send `start` and `end` with an offset when you remove `rrule`.
- Calendar polls. Refresh if it is missing.
