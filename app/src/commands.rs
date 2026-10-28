//! The typed commands the UI calls (plan.md §9). All are `async`, so Tauri runs them off the main
//! thread. Slow work goes through the engine's worker thread and reports by events.

use std::fmt::Display;
use std::path::PathBuf;

use serde::Serialize;
use sextant_core::connect::claude_code_command;
use sextant_core::{Settings, Sources};
use tauri::State;

use crate::engine::Engine;
use crate::{clock, connect, platform, prefs};

pub struct AppState {
    pub dir: PathBuf,
    pub engine: Engine,
}

type Reply<T> = Result<T, String>;

fn msg(err: impl Display) -> String {
    err.to_string()
}

const MAX_ROWS: usize = 200;

#[tauri::command]
pub async fn get_state(state: State<'_, AppState>) -> Reply<Settings> {
    prefs::load(&state.dir).map_err(msg)
}

#[derive(Serialize)]
pub struct Permission {
    name: &'static str,
    status: &'static str,
}

/// Status of the permissions the ticked sources need.
#[tauri::command]
pub async fn permission_status(state: State<'_, AppState>) -> Reply<Vec<Permission>> {
    let sources = prefs::load(&state.dir).map_err(msg)?.sources;
    let mut needed = Vec::new();
    if sources.calendar {
        needed.push(Permission {
            name: "calendar",
            status: platform::calendar_status(),
        });
        needed.push(Permission {
            name: "reminders",
            status: platform::reminders_status(),
        });
    }
    if sources.safari {
        needed.push(Permission {
            name: "safari",
            status: platform::safari_status(),
        });
    }
    Ok(needed)
}

/// Shows the system prompt for `calendar` or `reminders`. Returns whether access was granted.
#[tauri::command]
pub async fn request_permission(name: String) -> Reply<bool> {
    tauri::async_runtime::spawn_blocking(move || platform::request_permission(&name))
        .await
        .map_err(msg)?
        .map_err(msg)
}

/// Opens `full-disk-access`, `calendars` or `reminders` in System Settings.
#[tauri::command]
pub async fn open_settings_pane(pane: String) -> Reply<()> {
    platform::open_settings_pane(&pane).map_err(msg)
}

/// The folder dialog. `None` when cancelled.
#[tauri::command]
pub async fn pick_folder() -> Reply<Option<String>> {
    tauri::async_runtime::spawn_blocking(platform::pick_folder)
        .await
        .map_err(msg)?
        .map_err(msg)
}

#[tauri::command]
pub async fn save_choices(
    state: State<'_, AppState>,
    sources: Sources,
    git_folders: Vec<String>,
) -> Reply<Settings> {
    prefs::update(&state.dir, |s| {
        s.sources = sources;
        s.git_folders = git_folders;
    })
    .map_err(msg)
}

/// Starts the backfill on the worker thread. Progress arrives as `backfill-progress` events,
/// then `backfill-done` or `backfill-failed`.
#[tauri::command]
pub async fn start_backfill(state: State<'_, AppState>) -> Reply<()> {
    state.engine.start_backfill().map_err(msg)
}

#[derive(Serialize)]
pub struct MapRow {
    id: i64,
    name: String,
    steps: Vec<String>,
    hours_week: f64,
    cases: usize,
    action: &'static str,
    rough: bool,
}

#[derive(Serialize)]
pub struct FirstMap {
    hours_back: f64,
    total: usize,
    rows: Vec<MapRow>,
}

/// The ranked workflows, one page at a time.
#[tauri::command]
pub async fn first_map(state: State<'_, AppState>, offset: usize, limit: usize) -> Reply<FirstMap> {
    state
        .engine
        .read(|store| {
            let all = store.workflows()?;
            let rows = all
                .iter()
                .skip(offset)
                .take(limit.min(MAX_ROWS))
                .map(|w| MapRow {
                    id: w.id,
                    name: w.name.clone(),
                    steps: w.steps.iter().map(|s| s.name.clone()).collect(),
                    hours_week: w.hours_week,
                    cases: w.case_count,
                    action: w.action.as_str(),
                    rough: w.rough,
                })
                .collect();
            Ok(FirstMap {
                hours_back: store.hours_back()?,
                total: all.len(),
                rows,
            })
        })
        .map_err(msg)
}

#[derive(Serialize)]
pub struct ConnectInfo {
    binary: String,
    claude_code_command: String,
    desktop_config: String,
}

#[tauri::command]
pub async fn connect_info() -> Reply<ConnectInfo> {
    let binary = std::env::current_exe()
        .map_err(msg)?
        .to_string_lossy()
        .into_owned();
    Ok(ConnectInfo {
        claude_code_command: claude_code_command(&binary),
        desktop_config: connect::desktop_config_path()
            .map_err(msg)?
            .to_string_lossy()
            .into_owned(),
        binary,
    })
}

/// Adds Sextant to the Claude Desktop config and keeps a backup of the old file.
#[tauri::command]
pub async fn connect_claude_desktop() -> Reply<connect::Written> {
    let binary = std::env::current_exe()
        .map_err(msg)?
        .to_string_lossy()
        .into_owned();
    let path = connect::desktop_config_path().map_err(msg)?;
    connect::write_desktop_config(&path, &binary, clock::now()).map_err(msg)
}

/// The two MCP tool groups can be switched off on their own.
#[tauri::command]
pub async fn set_mcp_switches(
    state: State<'_, AppState>,
    rewind: bool,
    workflows: bool,
) -> Reply<Settings> {
    prefs::update(&state.dir, |s| {
        s.mcp_rewind = rewind;
        s.mcp_workflows = workflows;
    })
    .map_err(msg)
}

#[tauri::command]
pub async fn finish_onboarding(state: State<'_, AppState>) -> Reply<Settings> {
    prefs::update(&state.dir, |s| s.onboarded = true).map_err(msg)
}
