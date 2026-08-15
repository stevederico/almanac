use crate::util::unix_millis;

/// Instant → `YYYYMMDDTHHMMSSZ`.
pub fn to_ics_utc(iso: &str) -> Result<String, String> {
    let ms = parse_rfc3339_millis(iso).ok_or_else(|| format!("invalid date: {iso}"))?;
    let (y, m, d, hh, mm, ss, _) = civil_from_millis(ms);
    Ok(format!("{y:04}{m:02}{d:02}T{hh:02}{mm:02}{ss:02}Z"))
}

/// Calendar date `YYYY-MM-DD` → `YYYYMMDD`.
pub fn to_ics_date(ymd: &str) -> Result<String, String> {
    let (y, m, d) = parse_ymd(ymd).ok_or_else(|| format!("invalid all-day date: {ymd}"))?;
    Ok(format!("{y:04}{m:02}{d:02}"))
}

/// Exclusive next DATE for all-day DTEND.
pub fn next_date(ymd: &str) -> Result<String, String> {
    let (y, m, d) = parse_ymd(ymd).ok_or_else(|| format!("invalid all-day date: {ymd}"))?;
    let (y, m, d) = add_days(y, m, d, 1).ok_or_else(|| format!("invalid all-day date: {ymd}"))?;
    Ok(format!("{y:04}-{m:02}-{d:02}"))
}

pub fn now_iso() -> String {
    format_utc_millis(unix_millis())
}

pub fn parse_instant(value: &str) -> Option<String> {
    parse_rfc3339_millis(value).map(format_utc_millis)
}

pub fn add_hour(iso: &str) -> String {
    let ms = parse_rfc3339_millis(iso).expect("stored instant");
    format_utc_millis(ms + 3_600_000)
}

pub fn is_ymd(value: &str) -> bool {
    parse_ymd(value).is_some()
}

fn parse_ymd(value: &str) -> Option<(i32, u32, u32)> {
    if value.len() != 10 || value.as_bytes()[4] != b'-' || value.as_bytes()[7] != b'-' {
        return None;
    }
    let y: i32 = value[0..4].parse().ok()?;
    let m: u32 = value[5..7].parse().ok()?;
    let d: u32 = value[8..10].parse().ok()?;
    if m == 0 || m > 12 || d == 0 || d > days_in_month(y, m) {
        return None;
    }
    Some((y, m, d))
}

/// RFC3339 with optional millis and Z / ±HH:MM.
fn parse_rfc3339_millis(value: &str) -> Option<i64> {
    let b = value.as_bytes();
    if b.len() < 20 {
        return None;
    }
    let (y, mo, d) = parse_ymd(&value[0..10])?;
    if b[10] != b'T' && b[10] != b't' {
        return None;
    }
    let hh: u32 = value[11..13].parse().ok()?;
    if b[13] != b':' {
        return None;
    }
    let mm: u32 = value[14..16].parse().ok()?;
    if b[16] != b':' {
        return None;
    }
    let ss: u32 = value[17..19].parse().ok()?;
    if hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    let mut i = 19;
    let mut millis: i64 = 0;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let frac = &value[start..i];
        if frac.is_empty() {
            return None;
        }
        let padded = format!("{frac:0<3}");
        millis = padded[..3].parse().ok()?;
    }
    let offset_min: i32 = if i < b.len() && (b[i] == b'Z' || b[i] == b'z') {
        if i + 1 != b.len() {
            return None;
        }
        0
    } else if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        let sign = if b[i] == b'-' { -1 } else { 1 };
        let rest = &value[i + 1..];
        if rest.len() != 5 || rest.as_bytes()[2] != b':' {
            return None;
        }
        let oh: i32 = rest[0..2].parse().ok()?;
        let om: i32 = rest[3..5].parse().ok()?;
        if oh > 23 || om > 59 {
            return None;
        }
        sign * (oh * 60 + om)
    } else {
        return None;
    };
    let days = days_from_civil(y, mo, d);
    let local = days * 86_400 + i64::from(hh) * 3600 + i64::from(mm) * 60 + i64::from(ss);
    let utc_secs = local - i64::from(offset_min) * 60;
    Some(utc_secs * 1000 + millis)
}

fn format_utc_millis(ms: i64) -> String {
    let (y, m, d, hh, mm, ss, milli) = civil_from_millis(ms);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{milli:03}Z")
}

fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let mut y = i64::from(y);
    let m = i64::from(m);
    let d = i64::from(d);
    y -= i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_millis(ms: i64) -> (i32, u32, u32, u32, u32, u32, u32) {
    let mut secs = ms.div_euclid(1000);
    let milli = ms.rem_euclid(1000) as u32;
    if secs < 0 {
        // keep civil math on non-negative days via euclid; secs can be negative
    }
    let days = secs.div_euclid(86_400);
    secs = secs.rem_euclid(86_400);
    let hh = (secs / 3600) as u32;
    let mm = ((secs % 3600) / 60) as u32;
    let ss = (secs % 60) as u32;
    let (y, m, d) = civil_from_days(days);
    (y, m, d, hh, mm, ss, milli)
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let y = y + i64::from(m <= 2);
    (y as i32, m as u32, d as u32)
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn is_leap(y: i32) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

fn add_days(y: i32, m: u32, d: u32, add: i32) -> Option<(i32, u32, u32)> {
    let z = days_from_civil(y, m, d) + i64::from(add);
    let (y, m, d) = civil_from_days(z);
    Some((y, m, d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_z_and_offset() {
        assert_eq!(
            parse_instant("2026-08-16T20:05:00.000Z").unwrap(),
            "2026-08-16T20:05:00.000Z"
        );
        assert_eq!(
            parse_instant("2026-08-16T13:05:00-07:00").unwrap(),
            "2026-08-16T20:05:00.000Z"
        );
        assert_eq!(
            to_ics_utc("2026-08-16T20:05:00.000Z").unwrap(),
            "20260816T200500Z"
        );
        assert_eq!(next_date("2026-08-16").unwrap(), "2026-08-17");
        assert_eq!(
            add_hour("2026-08-16T19:00:00.000Z"),
            "2026-08-16T20:00:00.000Z"
        );
    }
}
