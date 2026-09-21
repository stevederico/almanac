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

Store `id` and `key`. The key is the write secret.

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

Subscribe: paste `subscribe` in Calendar → File → New Calendar Subscription. Use `https://`, not `webcal://`.

Machine docs: `GET /llms.txt`. Humans: `GET /`.

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
