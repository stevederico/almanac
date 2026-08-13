# Run

Prod: `https://almanac.dottie.ai` (Railway + dottie-proxy). Local loopback still works.

1. `cp .env.example .env` and set `FEED_TOKEN` + `AGENT_KEY`.
2. `bun install`
3. `./bin/almanac start` — copies the LaunchAgent and bootstraps it.
4. `./bin/almanac url` — subscribe that URL in Calendar.app.
5. Optional: `cp ~/Desktop/projects/almanac/bin/almanac ~/.local/bin/almanac`

Runtime copy lives in `~/.local/share/almanac` (launchd cannot read Desktop).
Logs: `~/Library/Logs/almanac.out` and `almanac.err`.

iPhone needs a reachable hostname (Tailscale). Do not bind `0.0.0.0` unless you mean to.
