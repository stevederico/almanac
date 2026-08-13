# my-calendar

ICS feed for Apple Calendar. Agents write events over HTTP. Humans only subscribe.

## When to use

Add, move, or drop something on Steve's calendar. Do not open Calendar.app. Do not build a UI.

## Auth

```
Authorization: Bearer $AGENT_KEY
```

`AGENT_KEY` and `FEED_TOKEN` live in `.env` (local) or the host env. Never log them. Never put them in the ICS URL except `FEED_TOKEN`.

Base URL: `http://127.0.0.1:18788` or `http://localhost:18788` (this Mac only).

## Write (prefer this)

Idempotent. Same `uid` updates in place and bumps `SEQUENCE` so Calendar replaces the event.

```bash
curl -sS -X PUT "$BASE/v1/events/dentist-2026-08-18" \
  -H "Authorization: Bearer $AGENT_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "summary": "Dentist",
    "start": "2026-08-18T09:00:00-07:00",
    "end": "2026-08-18T09:45:00-07:00",
    "location": "Fillmore"
  }'
```

Success `201` (create) or `200` (update):

```json
{
  "uid": "dentist-2026-08-18",
  "summary": "Dentist",
  "description": "",
  "location": "Fillmore",
  "start": "2026-08-18T16:00:00.000Z",
  "end": "2026-08-18T16:45:00.000Z",
  "allDay": false,
  "transparent": true,
  "sequence": 0,
  "createdAt": "2026-08-13T17:20:00.000Z",
  "updatedAt": "2026-08-13T17:20:00.000Z"
}
```

## Fields

| Field | Type | Required | Notes |
|---|---|---|---|
| `summary` | string | yes | ≤512 |
| `start` | string | yes | ISO-8601 datetime, or `YYYY-MM-DD` for all-day |
| `end` | string | no | Default +1 hour, or next day if all-day |
| `uid` | string | PUT path | `[A-Za-z0-9._@-]{1,200}`. Pick a stable id. |
| `location` | string | no | ≤512 |
| `description` | string | no | ≤4000 |
| `allDay` | bool | no | Or inferred from `YYYY-MM-DD` start |
| `transparent` | bool | no | Default `true` (does not mark busy) |

## Other endpoints

### Create without a uid

```bash
curl -sS -X POST "$BASE/v1/events" \
  -H "Authorization: Bearer $AGENT_KEY" \
  -H "Content-Type: application/json" \
  -d '{"summary":"Call","start":"2026-08-14T15:00:00-07:00"}'
```

Returns `201` with a generated `evt-<uuid>` uid. Prefer PUT with your own uid.

### Patch

```bash
curl -sS -X PATCH "$BASE/v1/events/dentist-2026-08-18" \
  -H "Authorization: Bearer $AGENT_KEY" \
  -H "Content-Type: application/json" \
  -d '{"start":"2026-08-18T10:00:00-07:00","end":"2026-08-18T10:45:00-07:00"}'
```

### List / get / delete

```bash
curl -sS "$BASE/v1/events" -H "Authorization: Bearer $AGENT_KEY"
curl -sS "$BASE/v1/events/dentist-2026-08-18" -H "Authorization: Bearer $AGENT_KEY"
curl -sS -X DELETE "$BASE/v1/events/dentist-2026-08-18" -H "Authorization: Bearer $AGENT_KEY"
```

Delete is `204`. The uid disappears from the feed; Calendar drops it on the next poll.

### Subscribe (humans)

`GET $BASE/feed/$FEED_TOKEN.ics` → `text/calendar`. No bearer. Wrong token → `404`.

## Errors

```json
{"error":"unauthorized"}
```

`401` missing/wrong bearer. Fix: send `Authorization: Bearer $AGENT_KEY`.

```json
{"error":"summary is required"}
```

`400` validation. Fix: send `summary` + `start`.

```json
{"error":"not found"}
```

`404` unknown uid (GET/PATCH/DELETE) or wrong feed token.

## Rules

- Stable `uid` per event. New uid = duplicate on the phone.
- Timed events: include a timezone offset (`-07:00`). Stored as UTC.
- All-day: `"start": "2026-08-20"` (no clock).
- No RRULE. One row per occurrence.
- Calendar polls. Changes are not instant. Refresh the calendar if you need it now.
