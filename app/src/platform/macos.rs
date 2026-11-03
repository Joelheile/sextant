use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use objc2_app_kit::NSWorkspace;
use objc2_application_services::AXIsProcessTrusted;
use objc2_core_foundation::{CFRunLoop, kCFRunLoopDefaultMode};
use objc2_core_graphics::CGPreflightScreenCaptureAccess;
use sextant_core::capture::{Browser, classify};

use super::audio::Mac as MacAudio;
use super::automation;
use super::drag_panel;
use super::live::Mac;
use super::process;
use super::speech::{self, Parakeet};
use super::vision::{CHOSEN, Recognizer, Timing, fixture_timings};
use super::{Prompt, SettingsPane};
use crate::audio;
use crate::capture::{Control, Pipeline, Sink};

const OPEN_TIMEOUT: Duration = Duration::from_secs(5);

/// `26.0.1`, or `unknown` when `sw_vers` fails.
pub fn os_version() -> String {
    Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map_or("unknown".to_owned(), |out| {
            String::from_utf8_lossy(&out.stdout).trim().to_owned()
        })
}

pub fn screen_recording_status() -> &'static str {
    if CGPreflightScreenCaptureAccess() {
        "granted"
    } else {
        "undetermined"
    }
}

pub fn accessibility_status() -> &'static str {
    // SAFETY: a plain C call without arguments.
    if unsafe { AXIsProcessTrusted() } {
        "granted"
    } else {
        "undetermined"
    }
}

/// The bundle ids of the running browsers whose address Sextant reads.
fn running_browsers() -> Vec<String> {
    NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .filter_map(|app| app.bundleIdentifier())
        .map(|id| id.to_string())
        .filter(|id| matches!(classify(id), Some(Browser::Chromium | Browser::Arc)))
        .collect()
}

/// Automation is asked per target app, and only for a running one. With no browser open the
/// answer is not known yet.
pub fn automation_status() -> &'static str {
    let answers: Vec<Option<bool>> = running_browsers()
        .iter()
        .map(|id| automation::permission(id, false))
        .collect();
    if answers.is_empty() || answers.contains(&None) {
        "undetermined"
    } else if answers.contains(&Some(false)) {
        "denied"
    } else {
        "granted"
    }
}

fn request_automation() -> bool {
    let answers: Vec<Option<bool>> = running_browsers()
        .iter()
        .map(|id| automation::permission(id, true))
        .collect();
    !answers.is_empty() && answers.iter().all(|answer| *answer == Some(true))
}

/// Shows the system prompt and returns whether access is granted right now.
pub fn request_permission(prompt: Prompt) -> Result<bool> {
    match prompt {
        Prompt::Automation => Ok(request_automation()),
    }
}

/// Runs the main run loop for `seconds`. The command line bench has no app loop of its own, and
/// the workspace only reports the frontmost app while the main loop runs.
pub fn pump(seconds: f64) {
    // SAFETY: reads a constant exported by Core Foundation.
    CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, seconds, false);
}

/// OCR time on the synthetic fixtures for both levels at both window sizes.
pub fn ocr_fixture_timings() -> Result<Vec<Timing>> {
    fixture_timings()
}

/// Starts live capture on its own threads.
pub fn start_live(control: Arc<Control>, sink: Sink) -> Result<Pipeline> {
    Pipeline::start(Mac::new(), Recognizer::new(CHOSEN), control, sink)
}

/// Starts call audio on its own threads, with the models from `models`.
pub fn start_audio(control: Arc<Control>, sink: Sink, models: &Path) -> Result<audio::Pipeline> {
    let speech = Parakeet::new(models, audio::Runs::Cpu);
    audio::Pipeline::start(MacAudio, speech::silero(models), speech, control, sink)
}

/// The `Privacy_*` anchor of a pane in the Privacy & Security settings.
fn anchor(pane: SettingsPane) -> &'static str {
    match pane {
        SettingsPane::ScreenRecording => "Privacy_ScreenCapture",
        SettingsPane::Accessibility => "Privacy_Accessibility",
        SettingsPane::Automation => "Privacy_Automation",
    }
}

/// For the panes whose list takes an app dragged in, the check that tells when it is allowed.
fn drag_check(pane: SettingsPane) -> Option<fn() -> bool> {
    match pane {
        SettingsPane::ScreenRecording => Some(|| screen_recording_status() == "granted"),
        SettingsPane::Accessibility => Some(|| accessibility_status() == "granted"),
        _ => None,
    }
}

/// Opens `pane` in System Settings. For a pane that takes a dragged app, `hint` also opens the
/// drag panel with that text.
pub fn open_settings_pane(pane: SettingsPane, hint: Option<&str>) -> Result<()> {
    let output = process::output(
        Command::new("open").arg(format!(
            "x-apple.systempreferences:com.apple.preference.security?{}",
            anchor(pane)
        )),
        OPEN_TIMEOUT,
    )?;
    if !output.status.success() {
        bail!("System Settings did not open");
    }
    if let (Some(granted), Some(hint)) = (drag_check(pane), hint) {
        drag_panel::show(hint.to_string(), granted);
    }
    Ok(())
}

/// Opens `script`, a `.command` file, in Terminal.
pub fn open_in_terminal(script: &Path) -> Result<()> {
    let output = process::output(
        Command::new("open").args(["-a", "Terminal"]).arg(script),
        OPEN_TIMEOUT,
    )?;
    if !output.status.success() {
        bail!("Terminal did not open");
    }
    Ok(())
}

/// Moves the running `Sextant.app` to the Trash, for the uninstall. Refuses a binary that is not
/// inside an app bundle.
pub fn trash_app() -> Result<()> {
    let exe = std::env::current_exe().context("finding the app")?;
    let bundle = exe
        .ancestors()
        .nth(3)
        .filter(|path| path.extension().is_some_and(|ext| ext == "app"))
        .context("Sextant is not running from an app bundle")?;
    let url = objc2_foundation::NSURL::fileURLWithPath(&objc2_foundation::NSString::from_str(
        &bundle.to_string_lossy(),
    ));
    objc2_foundation::NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, None)
        .map_err(|err| anyhow::anyhow!("moving Sextant to the Trash: {err}"))
}
