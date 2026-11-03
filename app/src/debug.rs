//! The debug report (D1) users paste into a GitHub issue: the errors and timings the app keeps
//! for it, and the command that builds it. Errors are kept as kinds, never with their text.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use sextant_core::Facts;
use tauri::AppHandle;

use crate::commands::{AppState, Reply, blocking, msg};
use crate::{clock, platform, prefs};

const KEPT_ERRORS: usize = 20;

struct Log {
    errors: VecDeque<(i64, &'static str)>,
    refresh_ms: Option<f64>,
    capture_to_search_ms: Option<f64>,
}

static LOG: Mutex<Log> = Mutex::new(Log {
    errors: VecDeque::new(),
    refresh_ms: None,
    capture_to_search_ms: None,
});

fn log() -> MutexGuard<'static, Log> {
    LOG.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Keeps the kind of a failure, such as `refresh`, for the report. Callers still print the error.
pub fn record(kind: &'static str) {
    let mut log = log();
    if log.errors.len() == KEPT_ERRORS {
        log.errors.pop_front();
    }
    log.errors.push_back((clock::now(), kind));
}

pub fn refreshed(took: Duration) {
    log().refresh_ms = Some(took.as_secs_f64() * 1000.0);
}

/// Keeps the slowest record of a flush, from capture until searchable.
pub fn flushed(latencies: &[Duration]) {
    if let Some(slowest) = latencies.iter().max() {
        log().capture_to_search_ms = Some(slowest.as_secs_f64() * 1000.0);
    }
}

fn permissions() -> Vec<(&'static str, &'static str)> {
    vec![
        ("accessibility", platform::accessibility_status()),
        ("screen-recording", platform::screen_recording_status()),
        ("automation", platform::automation_status()),
    ]
}

#[tauri::command]
pub async fn debug_report(app: AppHandle) -> Reply<String> {
    blocking(app, report).await
}

pub(crate) fn report(state: &AppState) -> Reply<String> {
    let settings = prefs::load(&state.dir).map_err(msg)?;
    let engine = state.engine()?;
    let mut numbers = Vec::new();
    let errors = {
        let log = log();
        numbers.extend(log.refresh_ms.map(|ms| ("last refresh ms", ms)));
        numbers.extend(
            log.capture_to_search_ms
                .map(|ms| ("capture to search ms", ms)),
        );
        log.errors.iter().copied().collect()
    };
    numbers.extend(engine.capture_numbers());
    let facts = Facts {
        app_version: env!("CARGO_PKG_VERSION"),
        macos: platform::os_version(),
        permissions: permissions(),
        numbers,
        errors,
    };
    engine
        .read(|store| store.debug_report(&settings, &facts, clock::local()))
        .map_err(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_keeps_the_last_20_error_kinds() {
        for _ in 0..KEPT_ERRORS {
            record("flush");
        }
        record("refresh");
        let log = log();
        assert_eq!(log.errors.len(), KEPT_ERRORS);
        assert!(log.errors.iter().any(|(_, kind)| *kind == "refresh"));
    }
}
