use std::collections::HashMap;
use std::sync::Mutex;

use crate::util::unix_millis;

/// Caps and rate limits. One struct so tests can shrink them.
#[derive(Debug, Clone)]
pub struct Limits {
    /// `POST /calendars` per client per hour.
    pub creates_per_client_hour: u32,
    /// `POST /calendars` across every client per hour. Backstop for when the
    /// client address is wrong (see `proxy_hops`) or spoofed.
    pub creates_global_hour: u32,
    /// Calendars the service will ever hold.
    pub max_calendars: i64,
    /// Events per calendar.
    pub max_events: i64,
    /// Exdates plus overrides per series.
    pub max_exceptions: usize,
    /// Writes per calendar per minute. The `home` calendar is exempt.
    pub writes_per_minute: u32,
    /// Proxies in front of the service that each append to `X-Forwarded-For`.
    /// The client is the entry this many from the right. `0` ignores the header.
    pub proxy_hops: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            creates_per_client_hour: 10,
            creates_global_hour: 300,
            max_calendars: 100_000,
            max_events: 2_000,
            max_exceptions: 500,
            writes_per_minute: 120,
            proxy_hops: 1,
        }
    }
}

pub const HOUR_MS: i64 = 3_600_000;
pub const MINUTE_MS: i64 = 60_000;

/// Distinct keys held at once. A spoofed-address flood cannot grow the map
/// past this; new keys are refused instead.
const MAX_KEYS: usize = 100_000;

/// Fixed-window counters keyed by string.
#[derive(Default)]
pub struct Limiter {
    hits: Mutex<HashMap<String, (i64, u32)>>,
}

impl Limiter {
    /// Count one hit. `Err(secs)` is how long until the window resets.
    pub fn check(&self, key: &str, max: u32, window_ms: i64) -> Result<(), u64> {
        self.check_at(unix_millis(), key, max, window_ms)
    }

    pub fn check_at(&self, now: i64, key: &str, max: u32, window_ms: i64) -> Result<(), u64> {
        let mut map = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= MAX_KEYS / 2 {
            // No window is longer than an hour, so anything older is dead.
            map.retain(|_, (start, _)| now - *start < HOUR_MS);
        }
        if map.len() >= MAX_KEYS && !map.contains_key(key) {
            return Err(60);
        }
        let entry = map.entry(key.to_string()).or_insert((now, 0));
        if now - entry.0 >= window_ms {
            *entry = (now, 0);
        }
        if entry.1 >= max {
            let remaining_ms = entry.0 + window_ms - now;
            return Err(((remaining_ms + 999) / 1000).max(1) as u64);
        }
        entry.1 += 1;
        Ok(())
    }
}

/// Who is calling: the `X-Forwarded-For` entry `hops` from the right, else the
/// socket peer. Trusting the leftmost entry would let a caller pick their own
/// bucket.
pub fn client_id(forwarded_for: &str, peer: &str, hops: usize) -> String {
    if hops > 0 {
        let entries: Vec<&str> = forwarded_for
            .split(',')
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .collect();
        if let Some(entry) = entries.len().checked_sub(hops).and_then(|i| entries.get(i)) {
            return (*entry).to_string();
        }
    }
    peer.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_max_then_reports_retry() {
        let l = Limiter::default();
        for _ in 0..3 {
            assert!(l.check_at(0, "k", 3, MINUTE_MS).is_ok());
        }
        assert_eq!(l.check_at(10_000, "k", 3, MINUTE_MS), Err(50));
        assert_eq!(l.check_at(10_001, "k", 3, MINUTE_MS), Err(50));
        assert_eq!(l.check_at(59_999, "k", 3, MINUTE_MS), Err(1));
        assert!(l.check_at(10_000, "other", 3, MINUTE_MS).is_ok());
    }

    #[test]
    fn window_resets() {
        let l = Limiter::default();
        assert!(l.check_at(0, "k", 1, MINUTE_MS).is_ok());
        assert!(l.check_at(59_999, "k", 1, MINUTE_MS).is_err());
        assert!(l.check_at(60_000, "k", 1, MINUTE_MS).is_ok());
    }

    #[test]
    fn client_is_the_nth_entry_from_the_right() {
        assert_eq!(client_id("1.1.1.1, 2.2.2.2", "9.9.9.9", 1), "2.2.2.2");
        assert_eq!(client_id("1.1.1.1, 2.2.2.2", "9.9.9.9", 2), "1.1.1.1");
        // A spoofed leftmost entry does not pick the bucket.
        assert_eq!(client_id("6.6.6.6, 1.1.1.1", "9.9.9.9", 1), "1.1.1.1");
        assert_eq!(client_id("1.1.1.1", "9.9.9.9", 2), "9.9.9.9");
        assert_eq!(client_id("", "9.9.9.9", 1), "9.9.9.9");
        assert_eq!(client_id("1.1.1.1", "9.9.9.9", 0), "9.9.9.9");
    }
}
