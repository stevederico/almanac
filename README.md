# almanac

an ics feed agents write and apple calendar reads. runs on this mac.

Prod: `https://almanac.dottie.ai`. Local: `http://localhost:18788`. Do not use `webcal://`.

## Quick Start

```bash
cp .env.example .env
# set FEED_TOKEN and AGENT_KEY (openssl rand -hex 24)
bun install
./bin/almanac start
./bin/almanac url
```

Calendar → File → New Calendar Subscription → paste that URL.

## Features

### Local feed
- **Loopback only** — nothing listens on the LAN
- **LaunchAgent** keeps it up after login (`com.stevederico.almanac`)
- **Runtime copy** in `~/.local/share/almanac` (launchd cannot read Desktop)
- **Stable UIDs** so edits replace, not duplicate

### Agent API
- **Bearer key** on `/v1/*`
- **PUT by uid** is the write path
- Contract: [AGENTS.md](AGENTS.md)

This Mac only. iPhone cannot see `127.0.0.1`. Tailscale later if you want the phone.

## Configuration

`.env` must be a regular file, not a symlink.

| Variable | Purpose |
|---|---|
| `HOST` | Bind address (default `localhost`) |
| `PORT` | Default `18788` |
| `FEED_TOKEN` | Secret in the subscribe URL |
| `AGENT_KEY` | Bearer token for writes |
| `CAL_NAME` | Title in Calendar (default `Almanac`) |
| `DB_PATH` | SQLite file |

## Tech Stack

| Technology | Version | Purpose |
|---|---|---|
| **Node** | 24+ | Runtime (`node:sqlite`, native TypeScript) |
| **Hono** | 4.7 | HTTP |
| **SQLite** | builtin | Event store |
| **launchd** | macOS | KeepAlive on login |

## Architecture

```
agent  --PUT /v1/events/:uid-->  SQLite on disk  --GET /feed/:token.ics-->  Calendar.app
```

## Control

```bash
almanac start|stop|restart|status|ensure|url
```
