//! What live capture needs from the OS (plan.md §4, §12). `platform` implements [`Sensors`] and
//! [`Ocr`] for macOS; `fake` replays a script so the gate tests run without any permission.

use std::fmt;
use std::thread::Thread;

use anyhow::Result;
use sextant_core::OcrLine;
use sextant_core::capture::{Browser, Element, Field, RawLine, Thumb};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub name: String,
    pub bundle_id: String,
    pub pid: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Presence {
    /// Seconds since the last input event of any kind.
    pub idle_s: f64,
    pub locked: bool,
    /// The display is asleep.
    pub asleep: bool,
}

/// Title and document of the focused window. Empty when the OS does not say.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowText {
    pub title: String,
    pub doc: String,
}

/// The front window of a browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub url: String,
    pub private: bool,
}

/// A captured window: a small grayscale thumbnail for the diff gate and the image itself, which
/// only the OCR step looks at. Pixels live in memory only and are dropped right after OCR.
pub struct Shot<I> {
    pub thumb: Thumb,
    pub image: I,
}

/// Prints the type and nothing else, so no pixel buffer reaches a log or a panic message.
impl<I> fmt::Debug for Shot<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Shot")
    }
}

/// Every call may fail or time out on its own. The caller only calls [`Sensors::capture`] after
/// all gates passed.
pub trait Sensors: Send + 'static {
    type Image: Send + 'static;

    fn presence(&mut self) -> Presence;
    fn focused_app(&mut self) -> Option<App>;
    /// Identifies the focused window of `app`, 0 when it has none. Read on every poll while the
    /// user is present, so a new window passes the gates before anything in it is read.
    fn window_id(&mut self, app: &App) -> u64;
    fn secure_input(&mut self) -> bool;
    /// Accessibility, 0.5 s timeout.
    fn window_text(&mut self, app: &App) -> WindowText;
    /// AppleScript, 2 s timeout. An error means the window could not be told apart from a private one.
    fn browser_tab(&mut self, app: &App, browser: Browser) -> Result<Tab>;
    /// Visible text of the focused window from the Accessibility tree, empty when it says nothing.
    fn window_lines(&mut self, app: &App) -> Vec<OcrLine>;
    /// The focused window of `app` only.
    fn capture(&mut self, app: &App) -> Result<Shot<Self::Image>>;
    /// Mouse presses since the last call, in screen points. Reads nothing from any app.
    fn clicks(&mut self) -> Vec<[f64; 2]>;
    /// Wakes `thread` on a mouse press, so the press is read before the window changes.
    fn wake_on_click(&mut self, thread: Thread);
    /// The element under `point` and its parents up to `capture::MAX_DEPTH`, empty when it
    /// belongs to another app than `app`.
    fn element_chain(&mut self, app: &App, point: [f64; 2]) -> Vec<Element>;
    /// The focused element of `app`. Its value is read only for a text field that is not secure
    /// and not longer than `capture::MAX_VALUE_CHARS`.
    fn focused_field(&mut self, app: &App) -> Option<Field>;
    /// The value of a field [`Sensors::focused_field`] returned before, read again.
    fn reread(&mut self, id: u64) -> Option<String>;
    /// Low Power Mode is on: capture skips screenshots and OCR and reads the tree only.
    fn low_power(&mut self) -> bool {
        false
    }
}

pub trait Ocr<I>: Send + 'static {
    fn recognize(&mut self, image: &I) -> Result<Vec<RawLine>>;
}
