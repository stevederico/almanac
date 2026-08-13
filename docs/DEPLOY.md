# Run (this Mac)

No host. Loopback only.

1. `cp .env.example .env` and set `FEED_TOKEN` + `AGENT_KEY`.
2. `bun install`
3. `./bin/my-calendar start` — copies the LaunchAgent and bootstraps it.
4. `./bin/my-calendar url` — subscribe that URL in Calendar.app.
5. Optional: `ln -sf ~/Desktop/projects/my-calendar/bin/my-calendar ~/.local/bin/my-calendar`

Runtime copy lives in `~/.local/share/my-calendar` (launchd cannot read Desktop).
Logs: `~/Library/Logs/my-calendar.out` and `my-calendar.err`.

iPhone needs a reachable hostname (Tailscale). Do not bind `0.0.0.0` unless you mean to.
