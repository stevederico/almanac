# Run

Prod: `https://almanac.dottie.ai` (Railway + dottie-proxy). Local loopback still works.

1. `cp .env.example .env` and set `FEED_TOKEN` + `AGENT_KEY`.
2. Rust toolchain (`cargo`) on PATH. System `libsqlite3` (macOS has it).
3. `./bin/almanac start` — builds the binary, copies the LaunchAgent, bootstraps it.
4. `./bin/almanac url` — subscribe that URL in Calendar.app.
5. Optional: `cp ~/Desktop/projects/almanac/bin/almanac ~/.local/bin/almanac`

Runtime copy lives in `~/.local/share/almanac` (launchd cannot read Desktop).
Logs: `~/Library/Logs/almanac.out` and `almanac.err`.

iPhone needs a reachable hostname (Tailscale). Do not bind `0.0.0.0` unless you mean to.
