//! OS access for backfill and live capture (plan.md §4, §6, §12). `cfg` appears only here:
//! macOS gets the real implementation, other systems get one that reports "needs macOS" so the
//! crate still builds.

use serde::Deserialize;

/// The permissions that have a system prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Prompt {
    Calendar,
    Reminders,
    Automation,
}

/// A pane of System Settings > Privacy & Security.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SettingsPane {
    FullDiskAccess,
    Calendars,
    Reminders,
    ScreenRecording,
    Accessibility,
    Automation,
}

#[cfg(target_os = "macos")]
mod act;
#[cfg(target_os = "macos")]
mod audio;
#[cfg(target_os = "macos")]
mod automation;
#[cfg(target_os = "macos")]
mod drag_panel;
#[cfg(target_os = "macos")]
mod eventkit;
#[cfg(target_os = "macos")]
pub mod language;
#[cfg(target_os = "macos")]
mod live;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod process;
#[cfg(target_os = "macos")]
mod render;
#[cfg(target_os = "macos")]
mod screen;
#[cfg(target_os = "macos")]
mod speech;
#[cfg(target_os = "macos")]
mod tree;
#[cfg(not(target_os = "macos"))]
mod unsupported;
#[cfg(target_os = "macos")]
mod vision;

#[cfg(target_os = "macos")]
pub use macos::{
    System, accessibility_status, automation_status, calendar_status, ocr_fixture_timings,
    open_settings_pane, pick_folder, pump, reminders_status, request_permission, safari_status,
    screen_recording_status, start_audio, start_live,
};
#[cfg(target_os = "macos")]
pub use speech::{Parakeet, silero};
#[cfg(not(target_os = "macos"))]
pub use unsupported::{Parakeet, language, silero};
#[cfg(not(target_os = "macos"))]
pub use unsupported::{
    System, accessibility_status, automation_status, calendar_status, ocr_fixture_timings,
    open_settings_pane, pick_folder, pump, reminders_status, request_permission, safari_status,
    screen_recording_status, start_audio, start_live,
};
