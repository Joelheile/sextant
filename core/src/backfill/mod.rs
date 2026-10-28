//! Backfill (plan.md §6): read history that already exists and turn it into `backfill` events.
//! Everything here is pure. The app implements [`Platform`] for macOS; tests use the fake one in
//! `fixtures`.

mod history;
mod records;

pub use history::{HistoryKind, Snapshot, Visit, sessions};
pub use records::{
    CalendarRecord, FileRecord, ReminderRecord, calendar_events, commit_events, file_events,
    parse_spotlight, reminder_events,
};

use crate::error::Result;
use crate::model::Event;
use crate::settings::Sources;
use crate::time::DAY;

pub const HISTORY_DAYS: i64 = 90;

/// A browser's History file on this machine. `location` is the platform's own handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryFile {
    pub browser: String,
    pub kind: HistoryKind,
    pub location: String,
}

/// Everything backfill needs from the OS. Each call may fail on its own, for example when a
/// permission is missing, and then only that source is skipped.
pub trait Platform {
    /// History files of installed browsers. Safari is listed only when `safari` is set.
    fn history_files(&self, safari: bool) -> Vec<HistoryFile>;
    /// The file's bytes, copied into memory.
    fn read_history(&self, file: &HistoryFile) -> Result<Vec<u8>>;
    fn calendar(&self, from: i64, to: i64) -> Result<Vec<CalendarRecord>>;
    fn reminders(&self, from: i64, to: i64) -> Result<Vec<ReminderRecord>>;
    /// Repositories inside a folder the user picked, or the folder itself.
    fn git_repos(&self, folder: &str) -> Vec<String>;
    /// `git log --all --since=<since> --format=%H%x09%at%x09%s`
    fn git_log(&self, repo: &str, since: i64) -> Result<String>;
    /// Documents the user opened since `since`, by Spotlight's last-used date.
    fn recent_files(&self, since: i64) -> Result<Vec<FileRecord>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Browsers,
    Calendar,
    Git,
    Files,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Browsers => "browsers",
            Self::Calendar => "calendar",
            Self::Git => "git",
            Self::Files => "files",
        }
    }
}

/// Counts and error kinds per source. Never text from the data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceReport {
    pub stage: Stage,
    pub events: usize,
    /// Sources that failed as a whole, for example "calendar access denied".
    pub errors: Vec<String>,
}

pub struct Collected {
    pub events: Vec<Event>,
    pub reports: Vec<SourceReport>,
}

/// Reads every ticked source over the last 90 days. A failing source is reported and skipped.
/// Events after `now` are dropped: the writer would clamp them and refresh only reads up to now.
/// `on_stage` is called before each source starts.
pub fn collect(
    platform: &dyn Platform,
    sources: &Sources,
    git_folders: &[String],
    now: i64,
    mut on_stage: impl FnMut(Stage),
) -> Collected {
    let from = now - HISTORY_DAYS * DAY;
    let mut events = Vec::new();
    let mut reports = Vec::new();
    let mut run = |stage: Stage, read: &mut dyn FnMut(&mut Vec<String>) -> Vec<Event>| {
        on_stage(stage);
        let mut errors = Vec::new();
        let found = read(&mut errors);
        reports.push(SourceReport {
            stage,
            events: found.len(),
            errors,
        });
        events.extend(found);
    };

    if sources.browsers || sources.safari {
        run(Stage::Browsers, &mut |errors| {
            let mut found = Vec::new();
            for file in platform.history_files(sources.safari) {
                let wanted = match file.kind {
                    HistoryKind::Chromium => sources.browsers,
                    HistoryKind::Safari => sources.safari,
                };
                if !wanted {
                    continue;
                }
                match visits_of(platform, &file, from) {
                    Ok(visits) => found.extend(sessions(&file.browser, &visits)),
                    Err(err) => errors.push(format!("{}: {err}", file.browser)),
                }
            }
            found
        });
    }
    if sources.calendar {
        run(Stage::Calendar, &mut |errors| {
            let mut found = Vec::new();
            match platform.calendar(from, now) {
                Ok(records) => found.extend(calendar_events(&records)),
                Err(err) => errors.push(format!("calendar: {err}")),
            }
            match platform.reminders(from, now) {
                Ok(records) => found.extend(reminder_events(&records)),
                Err(err) => errors.push(format!("reminders: {err}")),
            }
            found
        });
    }
    if sources.git && !git_folders.is_empty() {
        run(Stage::Git, &mut |errors| {
            let mut found = Vec::new();
            for repo in git_folders.iter().flat_map(|f| platform.git_repos(f)) {
                match platform.git_log(&repo, from) {
                    Ok(output) => found.extend(commit_events(&repo, &output)),
                    Err(err) => errors.push(format!("git: {err}")),
                }
            }
            found
        });
    }
    if sources.files {
        run(
            Stage::Files,
            &mut |errors| match platform.recent_files(from) {
                Ok(records) => file_events(&records),
                Err(err) => {
                    errors.push(format!("files: {err}"));
                    Vec::new()
                }
            },
        );
    }

    events.retain(|e| e.ts >= from && e.ts <= now);
    events.sort_by_key(|e| e.ts);
    Collected { events, reports }
}

fn visits_of(platform: &dyn Platform, file: &HistoryFile, since: i64) -> Result<Vec<Visit>> {
    let snapshot = Snapshot::from_bytes(platform.read_history(file)?)?;
    snapshot.visits(file.kind, since)
}
