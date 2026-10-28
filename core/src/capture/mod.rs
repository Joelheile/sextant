//! Live capture logic without OS APIs (plan.md §4): when to look, whether to look, whether the
//! picture changed and what to keep from the OCR result. Time and OS state are passed in.

mod browser;
mod diff;
mod focus;
mod gate;
mod ocr;
mod triggers;

use serde_json::{Value, json};

use crate::model::{Event, EventKind, Source, Window};
use crate::text::host_path;

pub use browser::{Browser, classify};
pub use diff::{CELL_DELTA, DiffGate, MIN_CELLS, SIDE, Thumb};
pub use focus::focused_window;
pub use gate::{Blocked, app, secure_input, session, tab};
pub use ocr::{MIN_CONFIDENCE, RawLine, lines};
pub use triggers::{
    Away, IDLE_LIMIT_S, INPUT_PAUSE_S, Reason, SETTLE_MS, Sample, Signal, TICK_MS, Triggers,
};

/// The `focus` event for a window the user is looking at. The address is cut to host and path.
pub fn focus_event(ts: i64, window: &Window) -> Event {
    let mut json = json!({"app": window.app, "title": window.title});
    for (key, value) in [("doc", window.doc.clone()), ("url", host_path(&window.url))] {
        if !value.is_empty() {
            json[key] = Value::String(value);
        }
    }
    event(ts, EventKind::Focus, json)
}

/// Which app copied. Never the content.
pub fn copy_event(ts: i64, app: &str) -> Event {
    event(ts, EventKind::Copy, json!({"app": app}))
}

/// Ends the open episode. Written when the user goes away, pauses or a gate closes, so the
/// previous window is not credited with the time and nothing about the gated window is kept.
pub fn idle_event(ts: i64) -> Event {
    event(ts, EventKind::Idle, json!({}))
}

pub fn pause_event(ts: i64) -> Event {
    event(ts, EventKind::Pause, json!({}))
}

fn event(ts: i64, kind: EventKind, json: Value) -> Event {
    Event {
        ts,
        kind,
        source: Source::Live,
        json,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_focus_event_keeps_host_and_path_but_not_the_query() {
        let window = Window {
            app: "Arc".into(),
            bundle_id: "company.thebrowser.Browser".into(),
            title: "Invoice".into(),
            doc: String::new(),
            url: "https://app.lexoffice.de/vouchers/INV-1?token=abc#x".into(),
        };
        let event = focus_event(10, &window);
        assert_eq!(event.json["url"], "app.lexoffice.de/vouchers/INV-1");
        assert!(event.json.get("doc").is_none());
        assert_eq!(event.source, Source::Live);
    }
}
