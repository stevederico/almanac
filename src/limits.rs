use std::collections::HashMap;
use std::sync::Mutex;

use crate::util::unix_millis;

/// Caps and rate limits. One struct so tests can shrink them.
#[derive(Debug, Clone)]
pub struct Limits {
    /// `POST /calendars` per client per hour.
    pub creates_per_client_hour: u32,
    /// `POST /calendars` across every client per hour.
    ///
    /// Only a backstop for the case where a caller controls its own bucket
    /// (no trusted proxy, or `proxy_hops` set too high), which defeats the
    /// per-client limit. It is deliberately far above real demand: tripping it
    /// blocks creation for everyone, so it is logged when it fires.
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

impl Limits {
    /// Override the tunable limits from the environment. Anything unset or
    /// unparseable keeps its default.
    pub fn from_env(mut self, get: impl Fn(&str) -> Option<String>) -> Self {
        fn num<T: std::str::FromStr>(raw: Option<String>) -> Option<T> {
            raw?.trim().parse().ok()
        }
        if let Some(v) = num(get("MAX_CREATES_PER_CLIENT_HOUR")) {
            self.creates_per_client_hour = v;
        }
        if let Some(v) = num(get("MAX_CREATES_PER_HOUR")) {
            self.creates_global_hour = v;
        }
        if let Some(v) = num(get("MAX_EVENTS_PER_CALENDAR")) {
            self.max_events = v;
        }
        if let Some(v) = num(get("MAX_WRITES_PER_MINUTE")) {
            self.writes_per_minute = v;
        }
        if let Some(v) = num(get("TRUSTED_PROXY_HOPS")) {
            self.proxy_hops = v;
        }
        self
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            creates_per_client_hour: 10,
            creates_global_hour: 2_000,
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
/// past this; new keys are refused instead until the next sweep.
const MAX_KEYS: usize = 100_000;

/// How often dead keys are swept out.
///
/// Sweeping is O(keys), so it runs on a timer, never per request. Doing it
/// whenever the map was merely large cost 600us per call at 60k keys -- on a
/// global mutex, which serialized every request in the process.
const SWEEP_INTERVAL_MS: i64 = MINUTE_MS;

#[derive(Default)]
struct Counters {
    hits: HashMap<String, (i64, u32)>,
    last_sweep: i64,
    #[cfg(test)]
    sweeps: u64,
}

/// Fixed-window counters keyed by string.
#[derive(Default)]
pub struct Limiter {
    counters: Mutex<Counters>,
}

impl Limiter {
    /// Count one hit. `Err(secs)` is how long until the window resets.
    pub fn check(&self, key: &str, max: u32, window_ms: i64) -> Result<(), u64> {
        self.check_at(unix_millis(), key, max, window_ms)
    }

    pub fn check_at(&self, now: i64, key: &str, max: u32, window_ms: i64) -> Result<(), u64> {
        let mut c = self.counters.lock().unwrap_or_else(|e| e.into_inner());
        // Outside the range, not merely past it: a negative gap is the clock
        // stepping backwards, which would otherwise postpone every sweep
        // until it caught up again.
        let since = now - c.last_sweep;
        if !(0..SWEEP_INTERVAL_MS).contains(&since) {
            // No window is longer than an hour, so anything older is dead.
            c.hits.retain(|_, (start, _)| now - *start < HOUR_MS);
            c.last_sweep = now;
            #[cfg(test)]
            {
                c.sweeps += 1;
            }
        }
        if c.hits.len() >= MAX_KEYS && !c.hits.contains_key(key) {
            return Err(60);
        }
        let map = &mut c.hits;
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
    fn sweeps_on_a_timer_not_on_every_call() {
        let l = Limiter::default();
        // Many calls inside one interval cost one sweep, not one each.
        for i in 0..50 {
            assert!(l.check_at(i * 1000, &format!("k{i}"), 10, MINUTE_MS).is_ok());
        }
        {
            let c = l.counters.lock().unwrap();
            assert_eq!(c.sweeps, 0, "swept per call instead of per interval");
            assert_eq!(c.hits.len(), 50, "nothing was dead yet");
        }
        // Past the interval, one sweep drops everything older than an hour.
        assert!(l.check_at(HOUR_MS + MINUTE_MS, "fresh", 10, MINUTE_MS).is_ok());
        {
            let c = l.counters.lock().unwrap();
            assert_eq!(c.sweeps, 1);
            assert_eq!(c.hits.len(), 1, "stale keys survived the sweep");
        }
        // Still one sweep for the whole next interval, however busy it gets.
        for i in 0..50 {
            let t = HOUR_MS + MINUTE_MS + i * 100;
            assert!(l.check_at(t, &format!("n{i}"), 10, MINUTE_MS).is_ok());
        }
        assert_eq!(l.counters.lock().unwrap().sweeps, 1);
    }

    #[test]
    fn a_backwards_clock_still_sweeps() {
        let l = Limiter::default();
        assert!(l.check_at(HOUR_MS, "a", 10, MINUTE_MS).is_ok());
        let before = l.counters.lock().unwrap().sweeps;
        assert!(l.check_at(0, "b", 10, MINUTE_MS).is_ok());
        assert_eq!(l.counters.lock().unwrap().sweeps, before + 1);
    }

    #[test]
    fn env_overrides_only_what_it_sets() {
        let env = |k: &str| match k {
            "MAX_CREATES_PER_HOUR" => Some("5000".to_string()),
            "TRUSTED_PROXY_HOPS" => Some(" 2 ".to_string()),
            "MAX_EVENTS_PER_CALENDAR" => Some("not-a-number".to_string()),
            _ => None,
        };
        let l = Limits::default().from_env(env);
        assert_eq!(l.creates_global_hour, 5_000);
        assert_eq!(l.proxy_hops, 2);
        assert_eq!(l.max_events, Limits::default().max_events);
        assert_eq!(
            l.creates_per_client_hour,
            Limits::default().creates_per_client_hour
        );
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
