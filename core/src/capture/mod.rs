//! Live capture logic without OS APIs: when to look, whether to look, whether the
//! picture changed and what to keep from the OCR result. Time and OS state are passed in.

mod act;
mod ax;
mod browser;
mod diff;
mod focus;
mod gate;
mod health;
mod ocr;
mod triggers;

use serde_json::{Value, json};

use crate::model::{Event, EventKind, Window};
use crate::text::host_path;

pub use act::{
    Edits, Element, Field, MAX_DEPTH, MAX_VALUE_CHARS, editable, press_event, pressable,
};
pub use ax::{AxRead, AxSource, MAX_NODES, enough, window_lines};
pub use browser::{Browser, classify};
pub use diff::{CELL_DELTA, DiffGate, MIN_CELLS, SIDE, Thumb};
pub use focus::focused_window;
pub use gate::{Blocked, app, call, secure_input, session, site, tab};
pub use health::{Stall, stall};
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

/// Ends the open activity. Written when the user goes away, pauses or a gate closes, so the
/// previous window is not credited with the time and nothing about the gated window is kept.
pub fn idle_event(ts: i64) -> Event {
    event(ts, EventKind::Idle, json!({}))
}

pub fn pause_event(ts: i64) -> Event {
    event(ts, EventKind::Pause, json!({}))
}

fn event(ts: i64, kind: EventKind, json: Value) -> Event {
    Event { ts, kind, json }
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
    }
}
