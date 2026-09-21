# To-do

- Back up every calendar off the Railway volume. `scripts/pull-export` only saves the calendar in `~/.config/almanac/hosted-calendars.json`. The copies in `/app/data/backups` sit on that same volume. Railway volume backups need Pro and are deleted with the volume. `src/backup.rs` still says to pair those copies with the host's volume backups.
