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
    let time = ts as libc::time_t;
    // SAFETY: `tm` is plain data and `localtime_r` fills it for the one thread that owns it.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&time, &mut tm).is_null() {
            return 0;
        }
        tm.tm_gmtoff as i32
    }
}
