# almanac

Agent-first calendar, notes, and todos. Request a calendar, subscribe the URL, agents write.

Prod: `https://almanac.dottie.ai`. Do not use `webcal://`.

## Use

`POST /calendars` returns `id`, `subscribe`, `write`, and `key`. The key is shown once; only its hash is stored. The same key writes events, todos, and notes on that calendar. There is no delete, so a create is permanent.

Subscribe: Calendar → File → New Calendar Subscription → paste the `subscribe` URL. Use `https://`.

| Resource | Write | Feed |
|---|---|---|
| Events | `PUT /v1/c/:id/events/:uid` | ICS |
| Todos | `PUT /v1/c/:id/todos/:uid` | VTODO |
| Notes | `PUT /v1/c/:id/notes/:uid` | Atom, `?q=` search |
| Export | `GET /v1/c/:id/export` | JSON or Markdown, no secrets |

Contract: [AGENTS.md](AGENTS.md). Machine-readable: `GET /llms.txt`. Agent skill: [skills/almanac](skills/almanac/SKILL.md). It reads the default calendar from `~/.config/almanac/hosted-calendars.json`.

## Prod

The `almanac` service on Railway, volume `/app/data`, behind dottie-proxy. It lives in the bixby project. A separate Railway project also named `almanac` is empty. Do not deploy there. Steps: [docs/DEPLOY.md](docs/DEPLOY.md).

## Local

Optional. The process binds loopback (`localhost`, `127.0.0.1`, and `::1`). Nothing else on the network can connect, including a phone. Use prod for that.

```bash
cp .env.example .env
# set FEED_TOKEN and AGENT_KEY (openssl rand -hex 24)
cargo run --release
```

`.env` must be a regular file, not a symlink. Default port `18788`. Health: `http://localhost:18788/health`.

`FEED_TOKEN` and `AGENT_KEY` seed the local `home` calendar only. Hosted calendars from `POST /calendars` each get their own key.

On macOS, `./bin/almanac start` builds a release binary, copies it to `~/.local/share/almanac` (launchd cannot read Desktop), and installs the LaunchAgent `com.stevederico.almanac`. `./bin/almanac url` prints the subscribe URL. The script's default repo path is `~/Desktop/projects/almanac`; set `ALMANAC_ROOT` if the checkout lives elsewhere. It always binds `localhost`.

| Variable | Purpose |
|---|---|
| `HOST` | Bind address (default `localhost`) |
| `PORT` | Default `18788` |
| `FEED_TOKEN` | Secret in the home calendar's subscribe URL |
| `AGENT_KEY` | Bearer token for the home calendar |
| `CAL_NAME` | Title in Calendar (default `Almanac`) |
| `DB_PATH` | SQLite file |
| `PUBLIC_BASE` | Origin used in feed URLs when set |

Other caps and backup settings are in `.env.example`.

## Stack

Rust 2021, no crates. System SQLite (`libsqlite3`).

```
agent  --PUT /v1/c/:id/events/:uid-->  SQLite  --GET /feed/:token.ics-->  Calendar
```
