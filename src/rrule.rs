/// Canonicalize a subset of RFC 5545 RRULE. Does not expand occurrences.
pub fn canonicalize(raw: &str, all_day: bool) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("rrule is empty".into());
    }
    if raw.len() > 400 {
        return Err("rrule is too long".into());
    }
    if raw.to_ascii_uppercase().starts_with("RRULE:") {
        return Err("rrule must not include the RRULE: prefix".into());
    }
    if raw.chars().any(|c| c.is_whitespace()) {
        return Err("invalid rrule".into());
    }

    let mut freq: Option<&'static str> = None;
    let mut interval: Option<u32> = None;
    let mut count: Option<u32> = None;
    let mut until: Option<String> = None;
    let mut byday: Option<String> = None;
    let mut bymonthday: Option<String> = None;
    let mut bymonth: Option<String> = None;

    for part in raw.split(';') {
        if part.is_empty() {
            return Err("invalid rrule".into());
        }
        let (key, value) = part
            .split_once('=')
            .ok_or_else(|| "invalid rrule".to_string())?;
        if value.is_empty() {
            return Err("invalid rrule".into());
        }
        let key = key.to_ascii_uppercase();
        match key.as_str() {
            "FREQ" => {
                if freq.is_some() {
                    return Err("invalid rrule".into());
                }
                freq = Some(parse_freq(value)?);
            }
            "INTERVAL" => {
                if interval.is_some() {
                    return Err("invalid rrule".into());
                }
                let n = parse_u32(value).ok_or_else(|| "invalid rrule".to_string())?;
                if n == 0 || n > 999 {
                    return Err("invalid rrule".into());
                }
                interval = Some(n);
            }
            "COUNT" => {
                if count.is_some() || until.is_some() {
                    return Err("invalid rrule".into());
                }
                let n = parse_u32(value).ok_or_else(|| "invalid rrule".to_string())?;
                if n == 0 || n > 366 {
                    return Err("invalid rrule".into());
                }
                count = Some(n);
            }
            "UNTIL" => {
                if until.is_some() || count.is_some() {
                    return Err("invalid rrule".into());
                }
                until = Some(parse_until(value, all_day)?);
            }
            "BYDAY" => {
                if byday.is_some() {
                    return Err("invalid rrule".into());
                }
                byday = Some(value.to_string());
            }
            "BYMONTHDAY" => {
                if bymonthday.is_some() {
                    return Err("invalid rrule".into());
                }
                bymonthday = Some(parse_monthdays(value)?);
            }
            "BYMONTH" => {
                if bymonth.is_some() {
                    return Err("invalid rrule".into());
                }
                bymonth = Some(parse_months(value)?);
            }
            _ => return Err(format!("invalid rrule: {key}")),
        }
    }

    let freq = freq.ok_or_else(|| "rrule requires FREQ".to_string())?;
    if let Some(days) = byday {
        byday = Some(parse_byday(&days, freq)?);
    }

    let mut out = vec![format!("FREQ={freq}")];
    if let Some(n) = interval {
        if n != 1 {
            out.push(format!("INTERVAL={n}"));
        }
    }
    if let Some(months) = bymonth {
        out.push(format!("BYMONTH={months}"));
    }
    if let Some(days) = bymonthday {
        out.push(format!("BYMONTHDAY={days}"));
    }
    if let Some(days) = byday {
        out.push(format!("BYDAY={days}"));
    }
    if let Some(n) = count {
        out.push(format!("COUNT={n}"));
    }
    if let Some(until) = until {
        out.push(format!("UNTIL={until}"));
    }
    Ok(out.join(";"))
}

fn parse_freq(value: &str) -> Result<&'static str, String> {
    match value.to_ascii_uppercase().as_str() {
        "DAILY" => Ok("DAILY"),
        "WEEKLY" => Ok("WEEKLY"),
        "MONTHLY" => Ok("MONTHLY"),
        "YEARLY" => Ok("YEARLY"),
        _ => Err("invalid rrule".into()),
    }
}

fn parse_u32(value: &str) -> Option<u32> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn parse_until(value: &str, all_day: bool) -> Result<String, String> {
    if all_day {
        if value.len() == 8 && valid_yyyymmdd(value) {
            return Ok(value.to_string());
        }
        return Err("invalid rrule".into());
    }
    if value.len() == 16 && value.as_bytes()[8] == b'T' && value.ends_with('Z') {
        let date = &value[..8];
        let time = &value[9..15];
        if valid_yyyymmdd(date) && valid_hhmmss(time) {
            return Ok(value.to_ascii_uppercase());
        }
    }
    Err("invalid rrule".into())
}

fn valid_yyyymmdd(value: &str) -> bool {
    if value.len() != 8 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let y: i32 = value[0..4].parse().unwrap_or(0);
    let m: u32 = value[4..6].parse().unwrap_or(0);
    let d: u32 = value[6..8].parse().unwrap_or(0);
    m >= 1 && m <= 12 && d >= 1 && d <= days_in_month(y, m)
}

fn valid_hhmmss(value: &str) -> bool {
    if value.len() != 6 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let hh: u32 = value[0..2].parse().unwrap_or(99);
    let mm: u32 = value[2..4].parse().unwrap_or(99);
    let ss: u32 = value[4..6].parse().unwrap_or(99);
    hh <= 23 && mm <= 59 && ss <= 59
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn parse_months(value: &str) -> Result<String, String> {
    let mut out = Vec::new();
    for part in value.split(',') {
        let n = parse_u32(part).ok_or_else(|| "invalid rrule".to_string())?;
        if n == 0 || n > 12 {
            return Err("invalid rrule".into());
        }
        out.push(n.to_string());
    }
    if out.is_empty() {
        return Err("invalid rrule".into());
    }
    Ok(out.join(","))
}

fn parse_monthdays(value: &str) -> Result<String, String> {
    let mut out = Vec::new();
    for part in value.split(',') {
        let (neg, digits) = part
            .strip_prefix('-')
            .map(|d| (true, d))
            .or_else(|| part.strip_prefix('+').map(|d| (false, d)))
            .unwrap_or((false, part));
        let n = parse_u32(digits).ok_or_else(|| "invalid rrule".to_string())?;
        if n == 0 || n > 31 {
            return Err("invalid rrule".into());
        }
        if neg {
            if n != 1 {
                return Err("invalid rrule".into());
            }
            out.push("-1".to_string());
        } else {
            out.push(n.to_string());
        }
    }
    if out.is_empty() {
        return Err("invalid rrule".into());
    }
    Ok(out.join(","))
}

fn parse_byday(value: &str, freq: &str) -> Result<String, String> {
    let ordinals_ok = matches!(freq, "MONTHLY" | "YEARLY");
    let mut out = Vec::new();
    for part in value.split(',') {
        if part.is_empty() {
            return Err("invalid rrule".into());
        }
        let (ordinal, day) = split_byday(part)?;
        if ordinal.is_some() && !ordinals_ok {
            return Err("invalid rrule".into());
        }
        let day = day.to_ascii_uppercase();
        if !matches!(day.as_str(), "MO" | "TU" | "WE" | "TH" | "FR" | "SA" | "SU") {
            return Err("invalid rrule".into());
        }
        match ordinal {
            Some(n) => out.push(format!("{n}{day}")),
            None => out.push(day),
        }
    }
    if out.is_empty() {
        return Err("invalid rrule".into());
    }
    Ok(out.join(","))
}

fn split_byday(part: &str) -> Result<(Option<i32>, &str), String> {
    let bytes = part.as_bytes();
    let mut i = 0;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        i += 1;
    }
    let digits = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == bytes.len() || bytes.len() - i != 2 {
        return Err("invalid rrule".into());
    }
    if digits == i {
        return Ok((None, &part[i..]));
    }
    let neg = bytes[0] == b'-';
    let start = usize::from(bytes[0] == b'+' || bytes[0] == b'-');
    let n: i32 = part[start..i]
        .parse()
        .map_err(|_| "invalid rrule".to_string())?;
    if n == 0 || n > 5 {
        return Err("invalid rrule".into());
    }
    let n = if neg { -n } else { n };
    if n != -1 && !(1..=5).contains(&n) {
        return Err("invalid rrule".into());
    }
    Ok((Some(n), &part[i..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_weekly() {
        assert_eq!(
            canonicalize("freq=weekly;INTERVAL=1;BYDAY=tu,th", false).unwrap(),
            "FREQ=WEEKLY;BYDAY=TU,TH"
        );
    }

    #[test]
    fn keeps_interval_and_monthly_ordinal() {
        assert_eq!(
            canonicalize("FREQ=MONTHLY;INTERVAL=2;BYDAY=+1MO", false).unwrap(),
            "FREQ=MONTHLY;INTERVAL=2;BYDAY=1MO"
        );
    }

    #[test]
    fn accepts_until_forms() {
        assert_eq!(
            canonicalize("FREQ=DAILY;UNTIL=20261231T235959Z", false).unwrap(),
            "FREQ=DAILY;UNTIL=20261231T235959Z"
        );
        assert_eq!(
            canonicalize("FREQ=DAILY;UNTIL=20261231", true).unwrap(),
            "FREQ=DAILY;UNTIL=20261231"
        );
    }

    #[test]
    fn rejects_unknown_parts_and_conflicts() {
        assert!(canonicalize("FREQ=WEEKLY;BYHOUR=9", false).is_err());
        assert!(canonicalize("FREQ=DAILY;COUNT=2;UNTIL=20261231T000000Z", false).is_err());
        assert!(canonicalize("FREQ=WEEKLY;BYDAY=1MO", false).is_err());
        assert!(canonicalize("RRULE:FREQ=DAILY", false).is_err());
        assert!(canonicalize("FREQ=DAILY;UNTIL=20261231", false).is_err());
        assert!(canonicalize("FREQ=DAILY;UNTIL=20261231T000000Z", true).is_err());
        assert!(canonicalize("INTERVAL=2", false).is_err());
    }
}
