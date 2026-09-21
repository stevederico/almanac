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

## Backups (Railway or any host)

The service copies its own database every `BACKUP_INTERVAL_HOURS` (default 6) into `BACKUP_DIR` (default `backups/` beside the database) and keeps the newest `BACKUP_KEEP` (default 7). Set the interval to `0` to turn it off.

Those copies live on the same volume as the database. They cover a bad deploy or a corrupted file, not losing the volume.

Railway's own volume backups need the Pro plan, and Railway deletes them with the volume anyway, so they would not cover losing it either. What does survive is a copy on another machine. `scripts/pull-export` fetches `GET /v1/c/{id}/export` (every event, todo and note body, no keys) into `~/.local/share/almanac-backups` (mode 700, files 600) and keeps the newest 30. It reads the key from `~/.config/almanac/hosted-calendars.json` and never prints it. Run it by hand, or once a day with the systemd user timer:

```bash
mkdir -p ~/.config/systemd/user
cp scripts/systemd/almanac-backup.* ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now almanac-backup.timer
systemctl --user list-timers almanac-backup     # next run
journalctl --user -u almanac-backup -n 5        # last result
```

Set `ALMANAC_BACKUP_DIR` and `ALMANAC_BACKUP_KEEP` to change where and how many. Keep that folder out of anything synced or committed: it holds your notes.

Before a migration that rewrites rows, the service copies the file to `<db>.pre-<version>` and never overwrites an earlier copy.

## Write keys at rest

Keys are stored as a SHA-256 hash. A key is shown once, in the `POST /calendars` response, and cannot be recovered.

Builds from `0.8.0` on authenticate with the hash alone. They leave the old plaintext column in place so a rollback to an older build still works. Once you are sure you will not roll back past `0.8.0`, boot once with `SCRUB_LEGACY_KEYS=1`: it blanks that column (after copying the file to `<db>.pre-0.11.0`), logs `{"msg":"scrub","calendars":N}`, and then you can unset it. Rolling back to a build older than `0.8.0` after that locks every calendar out.
