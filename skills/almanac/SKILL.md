---
name: almanac
description: >
  Hosted agent-first calendar, todos and notes on one key. Use when creating a
  calendar, adding or moving an event, adding or finishing a todo, saving or
  searching a note, exporting or backing up, or when he says almanac, "put
  this on my calendar", "add an event", "add a todo", "remind me", "save a
  note", or "back up my calendar".
---

# almanac

A default calendar already exists. Events, todos and notes all live on it and one key writes all three. Do not `POST /calendars` unless asked to create another, and do not delete a calendar unless asked.

```bash
BASE=https://almanac.dottie.ai
CREDS=$HOME/.config/almanac/hosted-calendars.json
ID=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))[0]['id'])" "$CREDS")
KEY=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))[0]['key'])" "$CREDS")
AUTH="Authorization: Bearer $KEY"; J="Content-Type: application/json"
```

The default calendar is the first entry of `~/.config/almanac/hosted-calendars.json` (a JSON list of `id`, `name`, `subscribe`, `write`, `key`, `createdAt`). Nothing about it is hardcoded here.

Never print, log or paste `$KEY`. A key is shown once, when its calendar is created, and the server keeps only a hash of it, so that file is the only copy. Test a key by status code only.

Every request needs a `User-Agent`. `-A "Claude-Agent"` works. An empty or default curl one gets a Cloudflare 403.

## Seals

`GET $BASE/v1/c/$ID` includes `feed`. `seal` is the default for a new calendar. `plain` is the readable field API.

On a `seal` calendar, seal before every write and open after every read, for events, todos, and notes. The server stores ciphertext and cannot search or merge fields inside it. Fetch the list with no `q`, `tag`, or `status`, open each seal, then filter locally.

```bash
BIN="$HOME/Projects/almanac/target/release/almanac"
printf '%s' "$JSON" | "$BIN" seal --cal "$ID" --kind event --uid "$UID" | {
  read -r SEAL
  curl -sS -A "Claude-Agent" -X PUT "$BASE/v1/c/$ID/events/$UID" -H "$AUTH" -H "$J" \
    -d "{\"seal\":\"$SEAL\"}"
}
printf '%s' "$SEAL" | "$BIN" open --cal "$ID" --kind event --uid "$UID"
```

`--kind` is `event`, `todo`, or `note`. The binary reads the key from the credentials file and never prints it. A sealed feed is a shell with `X-ALMANAC-SEAL` and no summary, start, or body. Do not set `feed` to `plain` unless he asks. `PATCH $BASE/v1/c/$ID` with `{"feed":"plain"}` or `{"feed":"seal"}` is that switch. `scripts/reseal` rewrites one named calendar to seals. Run it only when he names the calendar. His current calendar stays `plain` until then.

## Events

On a `plain` calendar:

```bash
curl -sS -A "Claude-Agent" -X PUT "$BASE/v1/c/$ID/events/<uid>" -H "$AUTH" -H "$J" \
  -d '{"summary":"Dentist","start":"2026-08-18T09:00:00-07:00","end":"2026-08-18T09:45:00-07:00"}'
```

List `GET /v1/c/$ID/events`. Change `PATCH …/events/<uid>`. Remove `DELETE …/events/<uid>`. Timed events need an offset, all-day is `"start":"2026-08-20"`. Repeats: `"rrule":"FREQ=WEEKLY;BYDAY=TU"` plus `"timeZone"`. Skip or change one date: `PUT …/events/<uid>/exdates` or `/overrides` with `recurrenceId`. Full rules: `$BASE/llms.txt`.

## Todos

On a `plain` calendar:

```bash
curl -sS -A "Claude-Agent" -X PUT "$BASE/v1/c/$ID/todos/<uid>" -H "$AUTH" -H "$J" \
  -d '{"title":"Buy milk","due":"2026-10-01","priority":1,"tags":["home"]}'
curl -sS -A "Claude-Agent" "$BASE/v1/c/$ID/todos?status=open&tag=home" -H "$AUTH"
curl -sS -A "Claude-Agent" -X PATCH "$BASE/v1/c/$ID/todos/<uid>" -H "$AUTH" -H "$J" -d '{"done":true}'
```

`title` is required (≤512). `due` is `YYYY-MM-DD` or an ISO datetime with offset. `priority` is 0-9, 1 highest. `done:true` stamps `completedAt`, `false` clears it. Tags are up to 10 of `[a-z0-9_-]`. `POST …/todos` (no uid) mints one. Get and delete are `…/todos/<uid>`. Cap 5000.

## Notes

On a `plain` calendar:

```bash
curl -sS -A "Claude-Agent" -X PUT "$BASE/v1/c/$ID/notes/<uid>" -H "$AUTH" -H "$J" \
  -d '{"title":"Ideas","body":"# Ideas\n- one","tags":["work"],"pinned":true}'
curl -sS -A "Claude-Agent" "$BASE/v1/c/$ID/notes?q=ideas&tag=work" -H "$AUTH"
```

`title` required (≤200), `body` is markdown (≤32KB). The list leaves bodies out: add `&body=true`, or `GET …/notes/<uid>` for one. `q` matches title or body, ASCII case-insensitive. Cap 500.

## Rules that apply to all three

- Stable `uid` per item. `PUT` is idempotent: same uid replaces in place and keeps any field you leave out. A new uid is a duplicate. Send `null` to clear a field.
- Wrong types are a `400`, not ignored. Read the error text.
- One write budget of 120 per minute covers events, todos and notes together. A `429` has `retry-after`.
- `401` is a wrong key. `404` is a bad uid.

## Export and feeds

`GET $BASE/v1/c/$ID/export?format=json|md` returns everything (events, todos, notes with bodies) with no secrets. 10 per hour.

Back up: run `~/Projects/almanac/scripts/pull-export`. It saves a private copy of the export in `~/.local/share/almanac-backups` (keeps the newest 30) and prints one line with the counts. Use it when asked to back up, and before anything risky. Never paste a backup's contents: it holds his notes. It reads the key itself. Scheduling it (a systemd timer, see `docs/DEPLOY.md`) is his call: do not install it unless asked.

`GET $BASE/v1/c/$ID` lists the calendar's feed URLs (events, todos, notes). Those URLs carry read tokens: give them out only when asked. The events feed is already subscribed in Calendar.app. The todos (VTODO `.ics`) and notes (Atom) feeds are not subscribed anywhere yet.

## Other

Create (only if asked): `POST $BASE/calendars` with `{"name":"Optional Title"}`. The response is the only time the `key` is shown. Store `id` and `key` in `~/.config/almanac/hosted-calendars.json` at once, mode 600, never in a synced folder.

Delete (only if asked): `DELETE $BASE/v1/c/$ID` with the same key. `204` when it is gone, including its events, todos, notes and feeds. A missing id or a wrong key is `401`. The `home` calendar cannot be deleted.

Old local home calendar (`AGENT_KEY` in `~/Projects/almanac/.env`, `PUT $BASE/v1/events/<uid>`) is not the default.

Docs: `$BASE/llms.txt`. Do not use webcal://.
