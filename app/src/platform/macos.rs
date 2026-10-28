use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use sextant_core::backfill::{
    CalendarRecord, FileRecord, HistoryFile, HistoryKind, Platform, ReminderRecord, parse_spotlight,
};
use sextant_core::{Error, Result as CoreResult};

use super::eventkit::{self, Kind};
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
        let output = Command::new("git")
            .args(["-C", repo, "log", "--all"])
            .arg(format!("--since={age} seconds ago"))
            .arg("--format=%H%x09%at%x09%s")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .map_err(|err| platform_error("running git", err))?;
        if !output.status.success() {
            return Err(Error::Platform("git log failed".into()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn recent_files(&self, since: i64) -> CoreResult<Vec<FileRecord>> {
        let age = (clock::now() - since).max(0);
        let output = Command::new("mdfind")
            .arg("-onlyin")
            .arg(home())
            .args(["-attr", "kMDItemLastUsedDate"])
            .arg(format!("kMDItemLastUsedDate >= $time.now(-{age})"))
            .output()
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

/// Shows the system prompt for `calendar` or `reminders`. Blocks until the user answers.
pub fn request_permission(name: &str) -> Result<bool> {
    let kind = match name {
        "calendar" => Kind::Events,
        "reminders" => Kind::Reminders,
        _ => bail!("{name} has no system prompt"),
    };
    eventkit::request(kind).context("asking for permission")
}

const PANES: [(&str, &str); 3] = [
    ("full-disk-access", "Privacy_AllFiles"),
    ("calendars", "Privacy_Calendars"),
    ("reminders", "Privacy_Reminders"),
];

pub fn open_settings_pane(pane: &str) -> Result<()> {
    let (_, anchor) = PANES
        .iter()
        .find(|(name, _)| *name == pane)
        .with_context(|| format!("unknown settings pane {pane}"))?;
    let status = Command::new("open")
        .arg(format!(
            "x-apple.systempreferences:com.apple.preference.security?{anchor}"
        ))
        .status()
        .context("running open")?;
    if !status.success() {
        bail!("System Settings did not open");
    }
    Ok(())
}

/// The macOS folder dialog. `None` when the user cancels.
pub fn pick_folder() -> Result<Option<String>> {
    let output = Command::new("osascript")
        .args([
            "-e",
            r#"POSIX path of (choose folder with prompt "Pick the folder that holds your code")"#,
        ])
        .output()
        .context("running osascript")?;
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
