//! Records the capture side hands to the writer.

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Focus,
    Idle,
    Copy,
    Calendar,
    Reminder,
    Visit,
    Commit,
    File,
    Pause,
    Press,
    Edit,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Focus => "focus",
            Self::Idle => "idle",
            Self::Copy => "copy",
            Self::Calendar => "calendar",
            Self::Reminder => "reminder",
            Self::Visit => "visit",
            Self::Commit => "commit",
            Self::File => "file",
            Self::Pause => "pause",
            Self::Press => "press",
            Self::Edit => "edit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Live,
    Backfill,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Backfill => "backfill",
        }
    }
}

/// Payload keys the finder reads (plan.md §7). The writer stores whatever JSON it gets.
///
/// - `focus`: `app`, `title`, `doc`, `url`, `text` (optional OCR heading or form title)
/// - `visit`: `url`, `title`, `dur_s`
/// - `calendar`: `title`, `end_ts`
/// - `copy`: `app`
/// - `reminder`: `title`, `due_ts`, `done`
/// - `press`: `app`, `role`, `label`
/// - `edit`: `app`, `role`, `label`, `value`
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Payload {
    pub app: String,
    pub title: String,
    pub doc: String,
    pub url: String,
    pub text: String,
    pub dur_s: i64,
    pub end_ts: i64,
    pub due_ts: Option<i64>,
    pub done: bool,
    pub role: String,
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub ts: i64,
    pub kind: EventKind,
    pub source: Source,
    pub json: serde_json::Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Window {
    pub app: String,
    pub bundle_id: String,
    pub title: String,
    pub doc: String,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct OcrLine {
    pub text: String,
    /// x, y, width, height on a 256 grid.
    pub bbox: [u8; 4],
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub ts: i64,
    pub window: Window,
    pub lines: Vec<OcrLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaker {
    Mic,
    System,
}

impl Speaker {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mic => "mic",
            Self::System => "system",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "mic" => Some(Self::Mic),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Utterance {
    pub ts_start: i64,
    pub ts_end: i64,
    pub source: Speaker,
    pub text: String,
}

#[derive(Debug, Clone)]
pub enum Item {
    Event(Event),
    Frame(Frame),
    Utterance(Utterance),
}
