use std::sync::Arc;

use anyhow::{Result, bail};
use sextant_core::backfill::{CalendarRecord, FileRecord, HistoryFile, Platform, ReminderRecord};
use sextant_core::{Error, Result as CoreResult};

use crate::capture::{Control, Pipeline, Sink};

/// Never produced here: OCR fixtures need macOS.
pub struct Timing {
    pub width: usize,
    pub height: usize,
    pub level: &'static str,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub missing: Vec<String>,
}

fn unsupported<T>() -> CoreResult<T> {
    Err(Error::Platform("needs macOS".into()))
}

pub struct System;

impl Platform for System {
    fn history_files(&self, _safari: bool) -> Vec<HistoryFile> {
        Vec::new()
    }
    fn read_history(&self, _file: &HistoryFile) -> CoreResult<Vec<u8>> {
        unsupported()
    }
    fn calendar(&self, _from: i64, _to: i64) -> CoreResult<Vec<CalendarRecord>> {
        unsupported()
    }
    fn reminders(&self, _from: i64, _to: i64) -> CoreResult<Vec<ReminderRecord>> {
        unsupported()
    }
    fn git_repos(&self, _folder: &str) -> Vec<String> {
        Vec::new()
    }
    fn git_log(&self, _repo: &str, _since: i64) -> CoreResult<String> {
        unsupported()
    }
    fn recent_files(&self, _since: i64) -> CoreResult<Vec<FileRecord>> {
        unsupported()
    }
}

pub fn calendar_status() -> &'static str {
    "denied"
}

pub fn reminders_status() -> &'static str {
    "denied"
}

pub fn safari_status() -> &'static str {
    "denied"
}

pub fn screen_recording_status() -> &'static str {
    "denied"
}

pub fn accessibility_status() -> &'static str {
    "denied"
}

pub fn automation_status() -> &'static str {
    "denied"
}

pub fn pump(seconds: f64) {
    std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
}

pub fn ocr_fixture_timings() -> Result<Vec<Timing>> {
    bail!("needs macOS")
}

pub fn start_live(_control: Arc<Control>, _sink: Sink) -> Result<Pipeline> {
    bail!("needs macOS")
}

pub fn request_permission(_name: &str) -> Result<bool> {
    bail!("needs macOS")
}

pub fn open_settings_pane(_pane: &str) -> Result<()> {
    bail!("needs macOS")
}

pub fn pick_folder() -> Result<Option<String>> {
    bail!("needs macOS")
}
