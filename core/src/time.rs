//! Calendar math for a fixed UTC offset. Core never reads the clock.

pub(crate) const DAY: i64 = 86_400;

/// Whole local days since 1970-01-01.
pub(crate) fn day(ts: i64, utc_offset_s: i32) -> i64 {
    (ts + i64::from(utc_offset_s)).div_euclid(DAY)
}

/// `YYYY-MM-DD` of a local day number (Hinnant's civil_from_days).
pub(crate) fn date(day: i64) -> String {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

pub(crate) fn weekday(day: i64) -> &'static str {
    // 1970-01-01 was a Thursday.
    ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][day.rem_euclid(7) as usize]
}

/// `Mon 2026-03-02` for a timestamp.
pub(crate) fn label(ts: i64, utc_offset_s: i32) -> String {
    let d = day(ts, utc_offset_s);
    format!("{} {}", weekday(d), date(d))
}

/// Inverse of `date`: local day number of a civil date (Hinnant's days_from_civil).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `2026-03-02T09:40:00+01:00` for a timestamp.
pub(crate) fn iso(ts: i64, utc_offset_s: i32) -> String {
    let local = ts + i64::from(utc_offset_s);
    let seconds = local.rem_euclid(DAY);
    let offset = utc_offset_s.abs();
    format!(
        "{}T{:02}:{:02}:{:02}{}{:02}:{:02}",
        date(local.div_euclid(DAY)),
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60,
        if utc_offset_s < 0 { '-' } else { '+' },
        offset / 3600,
        offset % 3600 / 60
    )
}

/// Parses `YYYY-MM-DD`, `YYYY-MM-DDTHH:MM[:SS]` (space also works) with an optional `Z` or
/// `+HH:MM` suffix, or Unix seconds. Without a suffix the time is local at `utc_offset_s`.
/// A bare date is the start of that day, or its last second when `end_of_day` is set.
pub(crate) fn parse(text: &str, utc_offset_s: i32, end_of_day: bool) -> Option<i64> {
    let text = text.trim();
    if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
        return text.parse().ok();
    }
    let (date_part, rest) = text.split_at_checked(10)?;
    let mut fields = date_part.split('-');
    let (year, month, day) = (
        fields.next()?.parse::<i64>().ok()?,
        fields.next()?.parse::<i64>().ok()?,
        fields.next()?.parse::<i64>().ok()?,
    );
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || fields.next().is_some() {
        return None;
    }
    let midnight = days_from_civil(year, month, day) * DAY;
    if rest.is_empty() {
        let local = midnight + if end_of_day { DAY - 1 } else { 0 };
        return Some(local - i64::from(utc_offset_s));
    }
    let rest = rest.strip_prefix(['T', ' '])?;
    let split = rest.find(['Z', '+', '-']).unwrap_or(rest.len());
    let (clock, zone) = rest.split_at(split);
    let mut parts = clock.split(':');
    let hour = parts.next()?.parse::<i64>().ok()?;
    let minute = parts.next()?.parse::<i64>().ok()?;
    let second = match parts.next() {
        Some(s) => s.split('.').next()?.parse::<i64>().ok()?,
        None => 0,
    };
    if hour > 23 || minute > 59 || second > 60 || parts.next().is_some() {
        return None;
    }
    let offset = match zone {
        "" => i64::from(utc_offset_s),
        "Z" => 0,
        _ => {
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let digits: String = zone[1..].chars().filter(char::is_ascii_digit).collect();
            let (h, m) = (digits.get(..2)?.parse::<i64>().ok()?, digits.get(2..)?);
            let m = if m.is_empty() {
                0
            } else {
                m.parse::<i64>().ok()?
            };
            sign * (h * 3600 + m * 60)
        }
    };
    Some(midnight + hour * 3600 + minute * 60 + second - offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_the_fixture_monday() {
        // 2026-03-02 00:00 +01:00
        assert_eq!(label(1_772_406_000, 3600), "Mon 2026-03-02");
        assert_eq!(label(1_772_406_000 - 1, 3600), "Sun 2026-03-01");
    }

    #[test]
    fn iso_and_parse_round_trip() {
        let monday = 1_772_406_000;
        assert_eq!(
            iso(monday + 9 * 3600 + 40 * 60, 3600),
            "2026-03-02T09:40:00+01:00"
        );
        assert_eq!(iso(monday, -3600 * 5 - 1800), "2026-03-01T17:30:00-05:30");
        for text in [
            "2026-03-02",
            "2026-03-02T00:00",
            "2026-03-02 00:00:00+01:00",
            "2026-03-01T23:00:00Z",
            "1772406000",
        ] {
            assert_eq!(parse(text, 3600, false), Some(monday), "{text}");
        }
        assert_eq!(parse("2026-03-02", 3600, true), Some(monday + DAY - 1));
        assert_eq!(
            parse("2026-03-02T09:40", 0, false),
            Some(monday + 3600 + 9 * 3600 + 40 * 60)
        );
        for bad in [
            "",
            "yesterday",
            "2026-13-01",
            "2026-03-02T25:00",
            "2026-03-02x",
        ] {
            assert_eq!(parse(bad, 0, false), None, "{bad}");
        }
    }
}
