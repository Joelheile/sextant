//! Fake platform (plan.md §12): replays a scripted session, so the gates, the pipeline and the
//! bench run without any permission. Pictures are stand-ins: a seed that decides the thumbnail
//! and the lines the "OCR" returns.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use sextant_core::capture::{Browser, RawLine, Thumb};

use super::sensors::{App, Ocr, Presence, Sensors, Shot, Tab, WindowText};

/// The world from `at_ms` on, until the next scene.
#[derive(Debug, Clone)]
pub struct Scene {
    pub at_ms: i64,
    pub idle_s: f64,
    pub locked: bool,
    pub asleep: bool,
    pub secure_input: bool,
    pub app: Option<App>,
    pub text: WindowText,
    /// `None` makes the browser fail to answer.
    pub tab: Option<Tab>,
    /// Same seed, same picture: the diff gate sees no change.
    pub picture: u64,
    pub lines: Vec<String>,
    pub clipboard: i64,
}

impl Scene {
    pub fn new(at_ms: i64) -> Self {
        Self {
            at_ms,
            idle_s: 0.0,
            locked: false,
            asleep: false,
            secure_input: false,
            app: None,
            text: WindowText::default(),
            tab: Some(Tab {
                url: String::new(),
                private: false,
            }),
            picture: 0,
            lines: Vec::new(),
            clipboard: 0,
        }
    }
}

/// What the capture thread asked of the OS.
#[derive(Debug, Default, Clone)]
pub struct Calls {
    pub presence: u32,
    pub clipboard: u32,
    pub focused_app: u32,
    pub secure_input: u32,
    pub window_text: u32,
    pub browser_tab: u32,
    /// App name of every capture call.
    pub captured: Vec<String>,
}

impl Calls {
    /// Every call except `presence` and `focused_app`, which the triggers need on every poll.
    pub fn reads_beyond_triggers(&self) -> u32 {
        self.clipboard
            + self.secure_input
            + self.window_text
            + self.browser_tab
            + self.captured.len() as u32
    }
}

pub struct Fake {
    script: Vec<Scene>,
    clock: Arc<AtomicI64>,
    calls: Arc<Mutex<Calls>>,
}

impl Fake {
    /// `clock` holds the script time in milliseconds, set by whoever drives the session.
    pub fn new(script: Vec<Scene>, clock: Arc<AtomicI64>) -> (Self, Arc<Mutex<Calls>>) {
        let calls = Arc::new(Mutex::new(Calls::default()));
        let fake = Self {
            script,
            clock,
            calls: calls.clone(),
        };
        (fake, calls)
    }

    fn scene(&self) -> Scene {
        let now = self.clock.load(Ordering::Relaxed);
        self.script
            .iter()
            .rev()
            .find(|scene| scene.at_ms <= now)
            .or(self.script.first())
            .cloned()
            .unwrap_or_else(|| Scene::new(0))
    }

    fn log(&self, record: impl FnOnce(&mut Calls)) {
        if let Ok(mut calls) = self.calls.lock() {
            record(&mut calls);
        }
    }
}

impl Sensors for Fake {
    type Image = Vec<String>;

    fn presence(&mut self) -> Presence {
        self.log(|calls| calls.presence += 1);
        let scene = self.scene();
        Presence {
            idle_s: scene.idle_s,
            locked: scene.locked,
            asleep: scene.asleep,
        }
    }

    fn clipboard_count(&mut self) -> i64 {
        self.log(|calls| calls.clipboard += 1);
        self.scene().clipboard
    }

    fn focused_app(&mut self) -> Option<App> {
        self.log(|calls| calls.focused_app += 1);
        self.scene().app
    }

    fn secure_input(&mut self) -> bool {
        self.log(|calls| calls.secure_input += 1);
        self.scene().secure_input
    }

    fn window_text(&mut self, _app: &App) -> WindowText {
        self.log(|calls| calls.window_text += 1);
        self.scene().text
    }

    fn browser_tab(&mut self, _app: &App, _browser: Browser) -> Result<Tab> {
        self.log(|calls| calls.browser_tab += 1);
        match self.scene().tab {
            Some(tab) => Ok(tab),
            None => bail!("the browser did not answer"),
        }
    }

    fn capture(&mut self, app: &App) -> Result<Shot<Vec<String>>> {
        self.log(|calls| calls.captured.push(app.name.clone()));
        let scene = self.scene();
        Ok(Shot {
            thumb: picture(scene.picture),
            image: scene.lines,
        })
    }
}

/// A 64×64 thumbnail that is the same for the same seed and far from any other seed.
fn picture(seed: u64) -> Thumb {
    let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    let pixels: Vec<u8> = (0..64 * 64)
        .map(|_| {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            (state.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 56) as u8
        })
        .collect();
    Thumb::from_gray(64, 64, &pixels).expect("a 64x64 buffer")
}

/// Returns the scripted lines of a fake picture, one per row.
pub struct FakeOcr;

impl Ocr<Vec<String>> for FakeOcr {
    fn recognize(&mut self, image: &Vec<String>) -> Result<Vec<RawLine>> {
        Ok(image
            .iter()
            .enumerate()
            .map(|(row, text)| RawLine {
                text: text.clone(),
                confidence: 0.9,
                x: 0.05,
                y: 0.05 + row as f32 * 0.03,
                width: 0.6,
                height: 0.02,
            })
            .collect())
    }
}
