use std::time::{SystemTime, UNIX_EPOCH};

use sextant_core::Local;

/// Unix seconds. Core never reads the clock; the app passes it in.
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The clock and its UTC offset, read together for each call so a daylight-saving change applies.
pub fn local() -> Local {
    let now = now();
    Local {
        now,
        utc_offset_s: utc_offset(now),
    }
}

/// Seconds east of UTC in the local time zone at `ts`.
pub fn utc_offset(ts: i64) -> i32 {
    broken_down(ts).map_or(0, |tm| tm.tm_gmtoff as i32)
}

/// Local clock time of `ts` as `HH:MM`.
pub fn hour_minute(ts: i64) -> String {
    broken_down(ts).map_or_else(String::new, |tm| {
        format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
    })
}

/// Local date and time of `ts` as `YYYY-MM-DD HH:MM`.
pub fn date_time(ts: i64) -> String {
    broken_down(ts).map_or_else(String::new, |tm| {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min
        )
    })
}

fn broken_down(ts: i64) -> Option<libc::tm> {
    let time = ts as libc::time_t;
    // SAFETY: `tm` is plain data and `localtime_r` fills it for the one thread that owns it.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        (!libc::localtime_r(&time, &mut tm).is_null()).then_some(tm)
    }
}
