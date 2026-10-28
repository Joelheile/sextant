//! What live capture needs from the OS (plan.md §4, §12). `platform` implements [`Sensors`] and
//! [`Ocr`] for macOS; `fake` replays a script so the gate tests run without any permission.

use std::fmt;

use anyhow::Result;
use sextant_core::capture::{Browser, RawLine, Thumb};

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
    /// A number that changes whenever something is copied.
    fn clipboard_count(&mut self) -> i64;
    fn focused_app(&mut self) -> Option<App>;
    fn secure_input(&mut self) -> bool;
    /// Accessibility, 0.5 s timeout.
    fn window_text(&mut self, app: &App) -> WindowText;
    /// AppleScript, 2 s timeout. An error means the window could not be told apart from a private one.
    fn browser_tab(&mut self, app: &App, browser: Browser) -> Result<Tab>;
    /// The focused window of `app` only.
    fn capture(&mut self, app: &App) -> Result<Shot<Self::Image>>;
}

pub trait Ocr<I>: Send + 'static {
    fn recognize(&mut self, image: &I) -> Result<Vec<RawLine>>;
}
