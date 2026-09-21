# Run

Prod is `https://almanac.dottie.ai`: the `almanac` service on Railway, volume `/app/data`, behind dottie-proxy. It lives in the bixby project. A separate Railway project also named `almanac` is empty. Do not deploy there.

`railway up` from this repo ships the current tree. Redeploying the existing build does not.

Prod answers `403` to an empty or default-curl User-Agent. Check with `-A "Mozilla/5.0 me"`: `/health`, then the boot lines in the Railway logs. A `home` sync log line means `AGENT_KEY`, `FEED_TOKEN`, or `CAL_NAME` drifted from the database.

## Local

Optional loopback. The server binds `localhost` (`127.0.0.1` and `::1`). Other devices cannot reach it. Do not bind `0.0.0.0` unless you mean to.

1. `cp .env.example .env` and set `FEED_TOKEN` + `AGENT_KEY` (`openssl rand -hex 24`).
2. Rust toolchain (`cargo`) on PATH, and system `libsqlcipher` (SQLCipher 4).
3. `cargo run --release`. Health: `http://localhost:18788/health`.

On macOS, `./bin/almanac start` builds the binary, copies it to `~/.local/share/almanac` (launchd cannot read Desktop), and bootstraps the LaunchAgent `com.stevederico.almanac`. `./bin/almanac url` prints the subscribe URL. Logs: `~/Library/Logs/almanac.out` and `almanac.err`. The script defaults `ALMANAC_ROOT` to `~/Desktop/projects/almanac`; set it when the checkout is somewhere else. It always binds `localhost`.

## Database encryption

`DB_KEY` is the passphrase for the SQLite file (`openssl rand -hex 32`). It is not stored in the file. With it set, a stolen `calendar.db` or an on-volume backup does not show event text, todo titles, note bodies, names, or feed tokens. Unset, the file stays plaintext, which is how tests and an existing deploy behave until the key is set.

The running process can still read the file, and so can anyone with the write key or a feed URL. Someone who also has `DB_KEY` can decrypt the file. `scripts/pull-export` stays a plaintext JSON copy on the machine that runs it.

On the next boot with `DB_KEY` set, a plaintext file is encrypted and the plaintext copy is deleted. A wrong key, or an encrypted file with no key, refuses to start and does not overwrite the file.

Prod, when deploying this: set `DB_KEY` on the bixby `almanac` service first, then deploy, then check `/health` and that the volume file does not start with `SQLite format 3`.

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
