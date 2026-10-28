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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_the_fixture_monday() {
        // 2026-03-02 00:00 +01:00
        assert_eq!(label(1_772_406_000, 3600), "Mon 2026-03-02");
        assert_eq!(label(1_772_406_000 - 1, 3600), "Sun 2026-03-01");
    }
}
