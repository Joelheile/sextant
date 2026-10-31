use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use objc2_app_kit::NSWorkspace;
use objc2_application_services::AXIsProcessTrusted;
use objc2_core_foundation::{CFRunLoop, kCFRunLoopDefaultMode};
use objc2_core_graphics::CGPreflightScreenCaptureAccess;
use sextant_core::backfill::{
    CalendarRecord, FileRecord, HistoryFile, HistoryKind, Platform, ReminderRecord, parse_spotlight,
};
use sextant_core::capture::{Browser, classify};
use sextant_core::{Error, Result as CoreResult};

use super::audio::Mac as MacAudio;
use super::automation;
use super::drag_panel;
use super::eventkit::{self, Kind};
use super::live::Mac;
use super::process;
use super::speech::{self, Parakeet};
use super::vision::{CHOSEN, Recognizer, Timing, fixture_timings};
use super::{Prompt, SettingsPane};
use crate::audio;
use crate::capture::{Control, Pipeline, Sink};
use crate::clock;

/// Chromium browsers: name and the folder under Application Support that holds their profiles.
const CHROMIUM: [(&str, &str); 4] = [
    ("Google Chrome", "Google/Chrome"),
    ("Arc", "Arc/User Data"),
    ("Brave", "BraveSoftware/Brave-Browser"),
    ("Microsoft Edge", "Microsoft Edge"),
];
const REPO_DEPTH: u32 = 3;
const MAX_FILES: usize = 5000;
const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const SPOTLIGHT_TIMEOUT: Duration = Duration::from_secs(30);
const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
/// The folder dialog waits for a person.
const DIALOG_TIMEOUT: Duration = Duration::from_secs(600);

pub struct System;

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn platform_error(context: &str, err: impl std::fmt::Display) -> Error {
    Error::Platform(format!("{context}: {err}"))
}

impl Platform for System {
    fn history_files(&self, safari: bool) -> Vec<HistoryFile> {
        let support = home().join("Library/Application Support");
        let mut files = Vec::new();
        for (browser, folder) in CHROMIUM {
            for profile in chromium_profiles(&support.join(folder)) {
                files.push(HistoryFile {
                    browser: browser.into(),
                    kind: HistoryKind::Chromium,
                    location: profile.join("History").to_string_lossy().into_owned(),
                });
            }
        }
        let safari_file = home().join("Library/Safari/History.db");
        // Without Full Disk Access the file looks missing or forbidden; both mean "ask for access".
        let present = match fs::metadata(&safari_file) {
            Ok(_) => true,
            Err(err) => err.kind() == ErrorKind::PermissionDenied,
        };
        if safari && present {
            files.push(HistoryFile {
                browser: "Safari".into(),
                kind: HistoryKind::Safari,
                location: safari_file.to_string_lossy().into_owned(),
            });
        }
        files
    }

    fn read_history(&self, file: &HistoryFile) -> CoreResult<Vec<u8>> {
        fs::read(&file.location).map_err(|err| match err.kind() {
            ErrorKind::PermissionDenied => Error::Platform("needs Full Disk Access".into()),
            _ => platform_error("reading history", err),
        })
    }

    fn calendar(&self, from: i64, to: i64) -> CoreResult<Vec<CalendarRecord>> {
        eventkit::events(from, to)
    }

    fn reminders(&self, from: i64, to: i64) -> CoreResult<Vec<ReminderRecord>> {
        eventkit::reminders(from, to)
    }

    fn git_repos(&self, folder: &str) -> Vec<String> {
        let mut repos = Vec::new();
        find_repos(Path::new(folder), REPO_DEPTH, &mut repos);
        repos
    }

    fn git_log(&self, repo: &str, since: i64) -> CoreResult<String> {
        let age = (clock::now() - since).max(0);
        let output = process::output(
            Command::new("git")
                .args(["-C", repo, "log", "--all"])
                .arg(format!("--since={age} seconds ago"))
                .arg("--format=%H%x09%at%x09%s")
                .env("GIT_OPTIONAL_LOCKS", "0"),
            GIT_TIMEOUT,
        )
        .map_err(|err| platform_error("running git", err))?;
        if !output.status.success() {
            return Err(Error::Platform("git log failed".into()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn recent_files(&self, since: i64) -> CoreResult<Vec<FileRecord>> {
        let age = (clock::now() - since).max(0);
        let output = process::output(
            Command::new("mdfind")
                .arg("-onlyin")
                .arg(home())
                .args(["-attr", "kMDItemLastUsedDate"])
                .arg(format!("kMDItemLastUsedDate >= $time.now(-{age})")),
            SPOTLIGHT_TIMEOUT,
        )
        .map_err(|err| platform_error("running mdfind", err))?;
        let mut files = parse_spotlight(&String::from_utf8_lossy(&output.stdout));
        files.sort_by_key(|f| std::cmp::Reverse(f.last_used));
        files.truncate(MAX_FILES);
        Ok(files)
    }
}

/// Profile folders of one Chromium browser: `Default` and `Profile N`, if they hold a History file.
fn chromium_profiles(root: &Path) -> Vec<PathBuf> {
    let mut profiles: Vec<PathBuf> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            (name == "Default" || name.starts_with("Profile "))
                && entry.path().join("History").is_file()
        })
        .map(|entry| entry.path())
        .collect();
    profiles.sort();
    profiles
}

/// Repositories at or below `dir`, not descending into a repository, hidden folders or build output.
fn find_repos(dir: &Path, depth: u32, repos: &mut Vec<String>) {
    if dir.join(".git").exists() {
        repos.push(dir.to_string_lossy().into_owned());
        return;
    }
    if depth == 0 {
        return;
    }
    let mut children: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            !name.starts_with('.')
                && !["node_modules", "target", "Library"].contains(&&*name)
                && entry.file_type().is_ok_and(|t| t.is_dir())
        })
        .map(|entry| entry.path())
        .collect();
    children.sort();
    for child in children {
        find_repos(&child, depth - 1, repos);
    }
}

pub fn calendar_status() -> &'static str {
    eventkit::status(Kind::Events)
}

pub fn reminders_status() -> &'static str {
    eventkit::status(Kind::Reminders)
}

/// Safari's history is readable only with Full Disk Access.
pub fn safari_status() -> &'static str {
    let file = home().join("Library/Safari/History.db");
    match fs::File::open(file) {
        Ok(_) => "granted",
        Err(_) => "denied",
    }
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

/// Shows the system prompt. Calendar and reminders block until the user answers; automation
/// returns whether access is granted right now.
pub fn request_permission(prompt: Prompt) -> Result<bool> {
    let kind = match prompt {
        Prompt::Calendar => Kind::Events,
        Prompt::Reminders => Kind::Reminders,
        Prompt::Automation => return Ok(request_automation()),
    };
    eventkit::request(kind).context("asking for permission")
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
        SettingsPane::FullDiskAccess => "Privacy_AllFiles",
        SettingsPane::Calendars => "Privacy_Calendars",
        SettingsPane::Reminders => "Privacy_Reminders",
        SettingsPane::ScreenRecording => "Privacy_ScreenCapture",
        SettingsPane::Accessibility => "Privacy_Accessibility",
        SettingsPane::Automation => "Privacy_Automation",
    }
}

/// For the panes whose list takes an app dragged in, the check that tells when it is allowed.
fn drag_check(pane: SettingsPane) -> Option<fn() -> bool> {
    match pane {
        SettingsPane::FullDiskAccess => Some(|| safari_status() == "granted"),
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

/// The macOS folder dialog. `None` when the user cancels.
pub fn pick_folder(prompt: &str) -> Result<Option<String>> {
    let output = process::output(
        Command::new("osascript").args([
            "-e",
            "on run argv",
            "-e",
            "POSIX path of (choose folder with prompt (item 1 of argv))",
            "-e",
            "end run",
            prompt,
        ]),
        DIALOG_TIMEOUT,
    )?;
    if !output.status.success() {
        return Ok(None);
    }
    let path = String::from_utf8_lossy(&output.stdout)
        .trim()
        .trim_end_matches('/')
        .to_string();
    Ok((!path.is_empty()).then_some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sextant-app-{}-{label}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("test setup");
        dir
    }

    fn git(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args([
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn finds_repositories_up_to_three_levels_deep() {
        let root = temp("repos");
        for path in [
            "a/.git",
            "b/c/.git",
            "b/c/d/.git",
            "node_modules/x/.git",
            ".hidden/y/.git",
            "e/f/g/h/.git",
        ] {
            fs::create_dir_all(root.join(path)).expect("test setup");
        }
        let mut repos = Vec::new();
        find_repos(&root, REPO_DEPTH, &mut repos);
        let names: Vec<&str> = repos
            .iter()
            .map(|r| {
                r.strip_prefix(root.to_str().expect("utf8"))
                    .expect("under root")
            })
            .collect();
        assert_eq!(names, ["/a", "/b/c"]);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn reads_the_git_log_of_a_temp_repository() {
        let root = temp("gitlog");
        git(&root, &["init", "-q"]);
        fs::write(root.join("a.txt"), "a").expect("test setup");
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", "Fix invoice total"]);

        let repo = root.to_string_lossy().into_owned();
        let log = System.git_log(&repo, clock::now() - 3600).expect("git log");
        let events = sextant_core::backfill::commit_events(&repo, &log);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].json["subject"], "Fix invoice total");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn lists_chromium_profiles_that_have_a_history_file() {
        let root = temp("chromium");
        for path in [
            "Default/History",
            "Profile 2/History",
            "Profile 3/Cookies",
            "Guest Profile/History",
        ] {
            let file = root.join(path);
            fs::create_dir_all(file.parent().expect("parent")).expect("test setup");
            fs::write(file, "").expect("test setup");
        }
        let names: Vec<String> = chromium_profiles(&root)
            .iter()
            .map(|p| p.file_name().expect("name").to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Default", "Profile 2"]);
        fs::remove_dir_all(root).expect("cleanup");
    }
}
