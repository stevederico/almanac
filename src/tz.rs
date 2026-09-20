pub const ALLOWED: &[&str] = &[
    "UTC",
    "America/Los_Angeles",
    "America/Denver",
    "America/Chicago",
    "America/New_York",
    "America/Phoenix",
    "Pacific/Honolulu",
    "Europe/London",
    "Europe/Paris",
    "Asia/Tokyo",
    "Australia/Sydney",
];

pub fn check(name: &str) -> Result<(), String> {
    if ALLOWED.contains(&name) {
        Ok(())
    } else {
        Err(format!(
            "unsupported timeZone. allowed: {}",
            ALLOWED.join(", ")
        ))
    }
}

pub fn is_utc(name: &str) -> bool {
    name == "UTC"
}

/// Current civil DST rules only. Not historical transitions.
pub fn vtimezone_lines(tzid: &str) -> Option<&'static [&'static str]> {
    Some(match tzid {
        "America/Los_Angeles" => LA,
        "America/Denver" => DENVER,
        "America/Chicago" => CHICAGO,
        "America/New_York" => NEW_YORK,
        "America/Phoenix" => PHOENIX,
        "Pacific/Honolulu" => HONOLULU,
        "Europe/London" => LONDON,
        "Europe/Paris" => PARIS,
        "Asia/Tokyo" => TOKYO,
        "Australia/Sydney" => SYDNEY,
        _ => return None,
    })
}

const LA: &[&str] = &[
    "BEGIN:VTIMEZONE",
    "TZID:America/Los_Angeles",
    "BEGIN:DAYLIGHT",
    "DTSTART:20070311T020000",
    "RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU",
    "TZOFFSETFROM:-0800",
    "TZOFFSETTO:-0700",
    "TZNAME:PDT",
    "END:DAYLIGHT",
    "BEGIN:STANDARD",
    "DTSTART:20071104T020000",
    "RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU",
    "TZOFFSETFROM:-0700",
    "TZOFFSETTO:-0800",
    "TZNAME:PST",
    "END:STANDARD",
    "END:VTIMEZONE",
];

const DENVER: &[&str] = &[
    "BEGIN:VTIMEZONE",
    "TZID:America/Denver",
    "BEGIN:DAYLIGHT",
    "DTSTART:20070311T020000",
    "RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU",
    "TZOFFSETFROM:-0700",
    "TZOFFSETTO:-0600",
    "TZNAME:MDT",
    "END:DAYLIGHT",
    "BEGIN:STANDARD",
    "DTSTART:20071104T020000",
    "RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU",
    "TZOFFSETFROM:-0600",
    "TZOFFSETTO:-0700",
    "TZNAME:MST",
    "END:STANDARD",
    "END:VTIMEZONE",
];

const CHICAGO: &[&str] = &[
    "BEGIN:VTIMEZONE",
    "TZID:America/Chicago",
    "BEGIN:DAYLIGHT",
    "DTSTART:20070311T020000",
    "RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU",
    "TZOFFSETFROM:-0600",
    "TZOFFSETTO:-0500",
    "TZNAME:CDT",
    "END:DAYLIGHT",
    "BEGIN:STANDARD",
    "DTSTART:20071104T020000",
    "RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU",
    "TZOFFSETFROM:-0500",
    "TZOFFSETTO:-0600",
    "TZNAME:CST",
    "END:STANDARD",
    "END:VTIMEZONE",
];

const NEW_YORK: &[&str] = &[
    "BEGIN:VTIMEZONE",
    "TZID:America/New_York",
    "BEGIN:DAYLIGHT",
    "DTSTART:20070311T020000",
    "RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU",
    "TZOFFSETFROM:-0500",
    "TZOFFSETTO:-0400",
    "TZNAME:EDT",
    "END:DAYLIGHT",
    "BEGIN:STANDARD",
    "DTSTART:20071104T020000",
    "RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU",
    "TZOFFSETFROM:-0400",
    "TZOFFSETTO:-0500",
    "TZNAME:EST",
    "END:STANDARD",
    "END:VTIMEZONE",
];

const PHOENIX: &[&str] = &[
    "BEGIN:VTIMEZONE",
    "TZID:America/Phoenix",
    "BEGIN:STANDARD",
    "DTSTART:19700101T000000",
    "TZOFFSETFROM:-0700",
    "TZOFFSETTO:-0700",
    "TZNAME:MST",
    "END:STANDARD",
    "END:VTIMEZONE",
];

const HONOLULU: &[&str] = &[
    "BEGIN:VTIMEZONE",
    "TZID:Pacific/Honolulu",
    "BEGIN:STANDARD",
    "DTSTART:19700101T000000",
    "TZOFFSETFROM:-1000",
    "TZOFFSETTO:-1000",
    "TZNAME:HST",
    "END:STANDARD",
    "END:VTIMEZONE",
];

const TOKYO: &[&str] = &[
    "BEGIN:VTIMEZONE",
    "TZID:Asia/Tokyo",
    "BEGIN:STANDARD",
    "DTSTART:19700101T000000",
    "TZOFFSETFROM:+0900",
    "TZOFFSETTO:+0900",
    "TZNAME:JST",
    "END:STANDARD",
    "END:VTIMEZONE",
];

const LONDON: &[&str] = &[
    "BEGIN:VTIMEZONE",
    "TZID:Europe/London",
    "BEGIN:DAYLIGHT",
    "DTSTART:20070325T010000",
    "RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU",
    "TZOFFSETFROM:+0000",
    "TZOFFSETTO:+0100",
    "TZNAME:BST",
    "END:DAYLIGHT",
    "BEGIN:STANDARD",
    "DTSTART:20071028T020000",
    "RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU",
    "TZOFFSETFROM:+0100",
    "TZOFFSETTO:+0000",
    "TZNAME:GMT",
    "END:STANDARD",
    "END:VTIMEZONE",
];

const PARIS: &[&str] = &[
    "BEGIN:VTIMEZONE",
    "TZID:Europe/Paris",
    "BEGIN:DAYLIGHT",
    "DTSTART:20070325T020000",
    "RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU",
    "TZOFFSETFROM:+0100",
    "TZOFFSETTO:+0200",
    "TZNAME:CEST",
    "END:DAYLIGHT",
    "BEGIN:STANDARD",
    "DTSTART:20071028T030000",
    "RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU",
    "TZOFFSETFROM:+0200",
    "TZOFFSETTO:+0100",
    "TZNAME:CET",
    "END:STANDARD",
    "END:VTIMEZONE",
];

const SYDNEY: &[&str] = &[
    "BEGIN:VTIMEZONE",
    "TZID:Australia/Sydney",
    "BEGIN:DAYLIGHT",
    "DTSTART:20081005T020000",
    "RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=1SU",
    "TZOFFSETFROM:+1000",
    "TZOFFSETTO:+1100",
    "TZNAME:AEDT",
    "END:DAYLIGHT",
    "BEGIN:STANDARD",
    "DTSTART:20080406T030000",
    "RRULE:FREQ=YEARLY;BYMONTH=4;BYDAY=1SU",
    "TZOFFSETFROM:+1100",
    "TZOFFSETTO:+1000",
    "TZNAME:AEST",
    "END:STANDARD",
    "END:VTIMEZONE",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_listed_zones_only() {
        assert!(check("UTC").is_ok());
        assert!(check("America/Los_Angeles").is_ok());
        let err = check("Mars/Base").unwrap_err();
        assert!(err.contains("America/Los_Angeles"));
        assert!(vtimezone_lines("UTC").is_none());
        let la = vtimezone_lines("America/Los_Angeles").unwrap();
        assert!(la.contains(&"TZID:America/Los_Angeles"));
        assert!(la.iter().any(|l| l.contains("BYDAY=2SU")));
    }
}
