//! Periodic copies of the database.
//!
//! One SQLite file holds every calendar, so it is worth a copy the service
//! makes itself. `VACUUM INTO` writes a consistent snapshot through a second
//! connection while the server keeps running. These land on the same volume, so
//! they cover a bad deploy or a corrupted file, not losing the volume: pair
//! them with the host's own volume backups.

use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::sqlite::Connection;
use crate::time::now_iso;

const PREFIX: &str = "almanac-";
const SUFFIX: &str = ".db";

/// Wait this long after boot before the first copy, so startup traffic and a
/// crash-looping container do not each pay for one.
const FIRST_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupConfig {
    pub dir: PathBuf,
    /// `0` turns backups off.
    pub interval_hours: u64,
    pub keep: usize,
}

impl BackupConfig {
    /// Read `BACKUP_DIR`, `BACKUP_INTERVAL_HOURS` and `BACKUP_KEEP`. The
    /// directory defaults to `backups/` beside the database, which is on the
    /// same volume. Unset or unparseable values keep the defaults.
    pub fn from_env(db_path: &str, get: impl Fn(&str) -> Option<String>) -> Self {
        fn num<T: std::str::FromStr>(raw: Option<String>) -> Option<T> {
            raw?.trim().parse().ok()
        }
        let dir = get("BACKUP_DIR")
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(db_path)
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join("backups")
            });
        Self {
            dir,
            interval_hours: num(get("BACKUP_INTERVAL_HOURS")).unwrap_or(6),
            keep: num(get("BACKUP_KEEP")).unwrap_or(7).max(1),
        }
    }
}

/// Write one snapshot into `dir` and prune all but the newest `keep`.
/// Returns the new file's path.
pub fn run_once(db_path: &Path, dir: &Path, keep: usize) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let stamp = now_iso().replace(['-', ':', '.'], "");
    let target = dir.join(format!("{PREFIX}{stamp}{SUFFIX}"));
    let conn = Connection::open(db_path)?;
    conn.execute_batch("PRAGMA busy_timeout = 5000;")?;
    let literal = target.to_string_lossy().replace('\'', "''");
    conn.execute_batch(&format!("VACUUM INTO '{literal}'"))?;
    prune(dir, keep)?;
    Ok(target)
}

/// Delete all but the newest `keep` backups. Names sort by timestamp.
fn prune(dir: &Path, keep: usize) -> Result<(), String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| format!("read {}: {e}", dir.display()))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(PREFIX) && name.ends_with(SUFFIX))
        .collect();
    names.sort();
    let stale = names.len().saturating_sub(keep);
    for name in names.into_iter().take(stale) {
        let _ = std::fs::remove_file(dir.join(name));
    }
    Ok(())
}

/// Run `run_once` on a timer for the life of the process. Failures are logged,
/// never fatal: a full disk must not take the calendar down with it.
pub fn spawn(db_path: PathBuf, cfg: BackupConfig) {
    if cfg.interval_hours == 0 {
        return;
    }
    thread::spawn(move || {
        thread::sleep(FIRST_DELAY);
        loop {
            match run_once(&db_path, &cfg.dir, cfg.keep) {
                Ok(path) => println!(
                    "{{\"t\":\"{}\",\"msg\":\"backup\",\"file\":\"{}\"}}",
                    now_iso(),
                    path.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                ),
                Err(e) => eprintln!("backup: {e}"),
            }
            thread::sleep(Duration::from_secs(cfg.interval_hours.saturating_mul(3600)));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::util::{hex_encode, random_bytes};

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("almanac-bak-{}", hex_encode(&random_bytes(8))));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_backup_is_a_working_copy_of_the_database() {
        let dir = scratch();
        let db_path = dir.join("calendar.db");
        let db = Db::open(&db_path).unwrap();
        let (cal, _) = db.create_calendar(Some("Kept"), None, None, None).unwrap();

        let file = run_once(&db_path, &dir.join("backups"), 7).unwrap();
        let copy = Db::open(&file).unwrap();
        assert_eq!(copy.get_calendar(&cal.id).unwrap().unwrap().name, "Kept");
    }

    #[test]
    fn only_the_newest_are_kept() {
        let dir = scratch();
        let db_path = dir.join("calendar.db");
        let _db = Db::open(&db_path).unwrap();
        let backups = dir.join("backups");
        let mut made = Vec::new();
        for _ in 0..5 {
            made.push(run_once(&db_path, &backups, 3).unwrap());
            // Names carry milliseconds, so stay apart from the previous one.
            thread::sleep(Duration::from_millis(3));
        }
        let left: Vec<_> = std::fs::read_dir(&backups)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(left.len(), 3);
        for old in &made[..2] {
            assert!(!old.exists(), "{old:?} should have been pruned");
        }
        for new in &made[2..] {
            assert!(new.exists(), "{new:?} should have been kept");
        }
    }

    #[test]
    fn other_files_in_the_directory_are_left_alone() {
        let dir = scratch();
        let db_path = dir.join("calendar.db");
        let _db = Db::open(&db_path).unwrap();
        let backups = dir.join("backups");
        std::fs::create_dir_all(&backups).unwrap();
        std::fs::write(backups.join("notes.txt"), "mine").unwrap();
        run_once(&db_path, &backups, 1).unwrap();
        assert!(backups.join("notes.txt").exists());
    }

    #[test]
    fn a_quote_in_the_path_does_not_break_the_statement() {
        let dir = scratch();
        let db_path = dir.join("calendar.db");
        let _db = Db::open(&db_path).unwrap();
        let file = run_once(&db_path, &dir.join("it's here"), 7).unwrap();
        assert!(file.exists());
    }

    #[test]
    fn env_overrides_only_what_it_sets() {
        let env = |k: &str| match k {
            "BACKUP_INTERVAL_HOURS" => Some("0".to_string()),
            "BACKUP_KEEP" => Some("nope".to_string()),
            _ => None,
        };
        let cfg = BackupConfig::from_env("/app/data/calendar.db", env);
        assert_eq!(cfg.dir, PathBuf::from("/app/data/backups"));
        assert_eq!(cfg.interval_hours, 0);
        assert_eq!(cfg.keep, 7);

        let cfg = BackupConfig::from_env("x.db", |k| {
            (k == "BACKUP_DIR").then(|| "/mnt/off-volume".to_string())
        });
        assert_eq!(cfg.dir, PathBuf::from("/mnt/off-volume"));
        assert_eq!(cfg.interval_hours, 6);
    }
}
