# almanac

Agent-first ICS calendar. Request a calendar. Subscribe the URL. Write events.

## Create

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

## Other

```bash
curl -sS "$WRITE" -H "Authorization: Bearer $KEY"
curl -sS -X PATCH "$WRITE/dentist-2026-08-18" -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" -d '{"location":"Fillmore"}'
curl -sS -X DELETE "$WRITE/dentist-2026-08-18" -H "Authorization: Bearer $KEY"
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
- No RRULE. One row per occurrence.
- Calendar polls. Refresh if it is missing.
