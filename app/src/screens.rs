//! The typed commands behind rewind search and Privacy (plan.md §9). Reads go through the
//! read-only connection, changes through the worker thread. Text from the database is redacted
//! again on the way out, as in the MCP server.

use serde::Serialize;
use sextant_core::{Live, Sent, Session, Settings, Store, Turn, keywords, redact};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use crate::commands::{AppState, Reply, blocking, msg};
use crate::keys::Keychain;
use crate::platform::language::Note;
use crate::{clock, connect, data, platform};

const RETENTION_CHOICES: [Option<u32>; 4] = [Some(30), Some(90), Some(365), None];
const STORAGE_CAP_CHOICES: [Option<u32>; 5] =
    [Some(1000), Some(2000), Some(5000), Some(10000), None];
/// "Delete the last 15 minutes, hour or day".
const FORGET_CHOICES: [i64; 3] = [15, 60, 1440];
const SENT_ROWS: usize = 50;

#[derive(Clone, Serialize)]
pub struct LineView {
    ts: i64,
    text: String,
}

#[derive(Clone, Serialize)]
pub struct SessionView {
    source: &'static str,
    app: String,
    title: String,
    url: String,
    start: i64,
    end: i64,
    lines: Vec<LineView>,
}

#[derive(Clone, Serialize)]
pub struct SearchPage {
    sessions: Vec<SessionView>,
}

fn session_view(session: Session, store: &Store) -> SessionView {
    let secret = store.secret();
    SessionView {
        source: match session.source {
            sextant_core::HitSource::Screen => "screen",
            sextant_core::HitSource::Mic => "mic",
            sextant_core::HitSource::System => "system",
        },
        app: session.app,
        title: redact(&session.title, &secret),
        url: redact(&session.url, &secret),
        start: session.start,
        end: session.end,
        lines: session
            .lines
            .into_iter()
            .map(|(ts, text)| LineView {
                ts,
                text: redact(&text, &secret),
            })
            .collect(),
    }
}

#[derive(Serialize)]
pub struct Storage {
    bytes: u64,
}

#[tauri::command]
pub async fn storage_used(state: State<'_, AppState>) -> Reply<Storage> {
    let dir = state.dir.clone();
    let bytes = tauri::async_runtime::spawn_blocking(move || data::storage_used(&dir))
        .await
        .map_err(msg)?;
    Ok(Storage { bytes })
}

#[tauri::command]
pub async fn set_excluded_apps(state: State<'_, AppState>, apps: Vec<String>) -> Reply<Settings> {
    let apps: Vec<String> = apps
        .into_iter()
        .map(|app| app.trim().to_owned())
        .filter(|app| !app.is_empty())
        .collect();
    state
        .engine()?
        .change_settings(move |s| s.excluded_apps = apps.clone())
        .map_err(msg)
}

#[tauri::command]
pub async fn set_live(state: State<'_, AppState>, live: Live) -> Reply<Settings> {
    state
        .engine()?
        .change_settings(move |s| {
            // Audio has its own switch, which also starts and stops the pipeline.
            s.live = Live {
                audio: s.live.audio,
                ..live
            };
        })
        .map_err(msg)
}

#[tauri::command]
pub async fn set_excluded_sites(state: State<'_, AppState>, sites: Vec<String>) -> Reply<Settings> {
    let sites: Vec<String> = sites
        .into_iter()
        .map(|site| site.trim().to_lowercase())
        .filter(|site| !site.is_empty())
        .collect();
    state
        .engine()?
        .change_settings(move |s| s.excluded_sites = sites.clone())
        .map_err(msg)
}

/// Caps raw data at 1, 2, 5 or 10 GB, or not at all (`None`), and applies it now.
#[tauri::command]
pub async fn set_storage_cap(app: AppHandle, mb: Option<u32>) -> Reply<Settings> {
    blocking(app, move |state| {
        if !STORAGE_CAP_CHOICES.contains(&mb) {
            return Err("choose 1, 2, 5 or 10 GB, or no cap".into());
        }
        let engine = state.engine()?;
        let settings = engine
            .change_settings(move |s| s.storage_cap_mb = mb)
            .map_err(msg)?;
        if let Some(mb) = mb {
            engine
                .write(move |store| {
                    store
                        .cap_storage(clock::local(), u64::from(mb) * 1_000_000)
                        .map(|_| ())
                })
                .map_err(msg)?;
        }
        Ok(settings)
    })
    .await
}

/// Deletes everything captured in the last 15 minutes, hour or day.
#[tauri::command]
pub async fn forget_recent(app: AppHandle, minutes: i64) -> Reply<()> {
    blocking(app, move |state| {
        if !FORGET_CHOICES.contains(&minutes) {
            return Err("choose 15 minutes, an hour or a day".into());
        }
        let when = clock::local();
        state
            .engine()?
            .write(move |store| {
                store
                    .forget(when, when.now - minutes * 60, i64::MAX)
                    .map(|_| ())
            })
            .map_err(msg)
    })
    .await
}

/// What agents received: each MCP answer, the bridge's included, newest first.
#[tauri::command]
pub async fn sent_log(app: AppHandle) -> Reply<Vec<Sent>> {
    blocking(app, |state| {
        state
            .engine()?
            .read(|store| store.sent(SENT_ROWS))
            .map_err(msg)
    })
    .await
}

/// Keeps raw data for 30, 90 or 365 days, or forever (`None`), and applies it now.
#[tauri::command]
pub async fn set_retention(app: AppHandle, days: Option<u32>) -> Reply<Settings> {
    blocking(app, move |state| {
        if !RETENTION_CHOICES.contains(&days) {
            return Err("choose 30, 90, 365 days or forever".into());
        }
        let engine = state.engine()?;
        let settings = engine
            .change_settings(move |s| s.retention_days = days)
            .map_err(msg)?;
        if let Some(days) = days {
            engine
                .write(move |store| store.retention(clock::local(), days).map(|_| ()))
                .map_err(msg)?;
        }
        Ok(settings)
    })
    .await
}

/// Stops capture and audio, closes the database, empties the data folder, deletes the key and
/// starts the app again. The UI asks for a confirmation first.
#[tauri::command]
pub async fn delete_everything(app: AppHandle) -> Reply<()> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        handle.state::<AppState>().delete_everything(&Keychain)
    })
    .await
    .map_err(msg)?
    .map_err(|err| format!("{err:#}"))?;
    app.restart()
}

/// "Delete everything", then takes Sextant out of the Claude Desktop config, moves the app to
/// the Trash and quits. Dev builds keep the app.
#[tauri::command]
pub async fn uninstall(app: AppHandle) -> Reply<()> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<()> {
        handle.state::<AppState>().delete_everything(&Keychain)?;
        connect::remove_from_desktop_config(&connect::desktop_config_path()?)?;
        if !cfg!(debug_assertions) {
            platform::trash_app()?;
        }
        Ok(())
    })
    .await
    .map_err(msg)?
    .map_err(|err| format!("{err:#}"))?;
    app.exit(0);
    Ok(())
}

const REWIND_RESULTS: usize = 5;
const REWIND_PAGE: usize = 20;

#[derive(Serialize)]
pub struct TurnView {
    id: i64,
    ts: i64,
    question: String,
    answer: String,
}

impl From<Turn> for TurnView {
    fn from(turn: Turn) -> Self {
        Self {
            id: turn.id,
            ts: turn.ts,
            question: turn.question,
            answer: turn.answer,
        }
    }
}

#[derive(Serialize)]
pub struct Asked {
    turn: TurnView,
    page: SearchPage,
}

/// What `rewind_ask` streams while it works: the sessions as soon as they are found, then the
/// answer text so far as the model writes it.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Progress {
    Sessions { page: SearchPage },
    Answer { text: String },
}

const DAY_S: i64 = 86_400;
/// Lines per session shown to the model, and characters per line, so a prompt with five
/// sessions stays well inside the model's 4,096-token context.
const NOTE_LINES: usize = 6;
const NOTE_CHARS: usize = 200;

/// The span a plan's `time` names, in whole local days: today is 0, yesterday 1.
fn days_back(time: &str) -> Option<(i64, i64)> {
    match time {
        "today" => Some((0, 0)),
        "yesterday" => Some((1, 1)),
        "this week" => Some((6, 0)),
        "last week" => Some((13, 7)),
        "this month" => Some((30, 0)),
        _ => None,
    }
}

/// From local midnight `since` days ago to the end of the day `until` days ago.
fn plan_range(now: i64, time: &str) -> (Option<i64>, Option<i64>) {
    let Some((since, until)) = days_back(time) else {
        return (None, None);
    };
    let midnight = now - (now + i64::from(clock::utc_offset(now))).rem_euclid(DAY_S);
    (
        Some(midnight - since * DAY_S),
        Some(midnight - (until - 1) * DAY_S - 1),
    )
}

/// Sessions for a question, best first. The on-device model picks the terms and the time span
/// when it can; otherwise the question's keywords are searched over all time. A span that finds
/// nothing is dropped, since people often misremember when.
fn recall_views(state: &AppState, question: &str, now: i64) -> Result<Vec<SessionView>, String> {
    let settings = crate::prefs::load(&state.dir).map_err(msg)?;
    let (mut terms, range) = match platform::language::plan(question) {
        Some(plan) => (plan.terms, plan_range(now, &plan.time)),
        None => (Vec::new(), (None, None)),
    };
    if terms.iter().all(|term| term.trim().is_empty()) {
        terms = keywords(question);
    }
    if terms.is_empty() {
        return Err("Add a name, topic or word you remember.".into());
    }
    state
        .engine()?
        .read(move |store| {
            let mut found = store.recall(&settings, &terms, range, REWIND_RESULTS)?;
            if found.is_empty() && range != (None, None) {
                found = store.recall(&settings, &terms, (None, None), REWIND_RESULTS)?;
            }
            Ok(found.into_iter().map(|s| session_view(s, store)).collect())
        })
        .map_err(msg)
}

/// A short answer from the on-device model that cites one session, which moves to the top.
/// Empty when the model is unavailable or nothing was found.
fn answer(
    question: &str,
    sessions: &mut [SessionView],
    on_text: impl FnMut(&str) + Send,
) -> String {
    let notes: Vec<Note> = sessions
        .iter()
        .map(|session| Note {
            app: session.app.clone(),
            title: session.title.clone(),
            ts: session.end,
            lines: session
                .lines
                .iter()
                .take(NOTE_LINES)
                .map(|line| line.text.chars().take(NOTE_CHARS).collect())
                .collect(),
        })
        .collect();
    if notes.is_empty() {
        return String::new();
    }
    let Some(answer) = platform::language::answer(question, &notes, on_text) else {
        return String::new();
    };
    if (1..=sessions.len()).contains(&answer.note) {
        sessions[..answer.note].rotate_right(1);
    }
    answer.text
}

/// Answers a question in the rewind search panel and adds it to the history. Streams the sessions
/// and then the answer to `progress`, so the panel shows them before the turn is saved.
#[tauri::command]
pub async fn rewind_ask(
    app: AppHandle,
    question: String,
    progress: Channel<Progress>,
) -> Reply<Asked> {
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err("Type a question first.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let now = clock::now();
        let mut sessions = recall_views(&state, &question, now)?;
        progress
            .send(Progress::Sessions {
                page: SearchPage {
                    sessions: sessions.clone(),
                },
            })
            .map_err(msg)?;
        let text = answer(&question, &mut sessions, |text| {
            if let Err(err) = progress.send(Progress::Answer { text: text.into() }) {
                eprintln!("streaming the answer failed: {err}");
            }
        });
        let turn = state
            .engine()?
            .write(move |store| store.add_turn(now, &question, &text))
            .map_err(msg)?;
        Ok(Asked {
            turn: turn.into(),
            page: SearchPage { sessions },
        })
    })
    .await
    .map_err(msg)?
}

/// The sessions for a past question, found again so retention and exclusions apply.
#[tauri::command]
pub async fn rewind_results(app: AppHandle, question: String) -> Reply<SearchPage> {
    tauri::async_runtime::spawn_blocking(move || {
        let sessions = recall_views(&app.state::<AppState>(), &question, clock::now())?;
        Ok(SearchPage { sessions })
    })
    .await
    .map_err(msg)?
}

/// Up to 20 past questions older than turn `before` (or the newest), newest first.
#[tauri::command]
pub async fn rewind_history(app: AppHandle, before: Option<i64>) -> Reply<Vec<TurnView>> {
    blocking(app, move |state| {
        state
            .engine()?
            .read(|store| store.turns(before, REWIND_PAGE))
            .map(|turns| turns.into_iter().map(TurnView::from).collect())
            .map_err(msg)
    })
    .await
}

/// Fits the panel to its content, `height` logical pixels.
#[tauri::command]
pub async fn rewind_fit(app: AppHandle, height: f64) -> Reply<()> {
    crate::rewind::fit_panel(&app, height).map_err(msg)
}

#[tauri::command]
pub async fn rewind_hide(app: AppHandle) -> Reply<()> {
    crate::rewind::hide(&app).map_err(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_time_becomes_whole_local_days_before_now() {
        let now = 1_800_000_000;
        let (Some(from), Some(to)) = plan_range(now, "yesterday") else {
            panic!("yesterday has a span");
        };
        assert_eq!(to - from + 1, DAY_S);
        assert!(to < now && now - to <= DAY_S);
        let (Some(week), Some(today_end)) = plan_range(now, "this week") else {
            panic!("this week has a span");
        };
        assert_eq!(today_end - week + 1, 7 * DAY_S);
        assert!(today_end >= now);
        assert_eq!(plan_range(now, "any time"), (None, None));
    }
}
