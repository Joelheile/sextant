//! The typed commands the UI calls (plan.md §9). All are `async`, so Tauri runs them off the main
//! thread. Slow work goes through the engine's worker thread and reports by events.

use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

use serde::Serialize;
use sextant_core::connect::claude_code_command;
use sextant_core::{Settings, Sources};
use tauri::State;

use crate::engine::Engine;
use crate::keys::KeyVault;
use crate::{clock, connect, data, models, platform, prefs};

pub struct AppState {
    pub dir: PathBuf,
    /// Set once by the start-up thread: the engine, or why it did not start.
    pub engine: OnceLock<Result<Engine, String>>,
    /// Held while the engine starts and while the data is deleted, so neither runs inside the
    /// other. `true` once the data is deleted: the engine must not start after that.
    deleted: Mutex<bool>,
}

pub(crate) type Reply<T> = Result<T, String>;

impl AppState {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            engine: OnceLock::new(),
            deleted: Mutex::new(false),
        }
    }

    /// Runs `start` and keeps its result, unless the data was deleted first.
    pub(crate) fn start(&self, start: impl FnOnce(&Path) -> anyhow::Result<Engine>) {
        let deleted = self.deleted.lock().unwrap_or_else(PoisonError::into_inner);
        let started = if *deleted {
            Err("the data was deleted".into())
        } else {
            start(&self.dir).map_err(|err| format!("{err:#}"))
        };
        let _ = self.engine.set(started);
    }

    /// Waits for an engine that is starting, then wipes the data folder and the key.
    pub(crate) fn delete_everything(&self, vault: &dyn KeyVault) -> anyhow::Result<()> {
        let mut deleted = self.deleted.lock().unwrap_or_else(PoisonError::into_inner);
        *deleted = true;
        let engine = self.engine.get().and_then(|started| started.as_ref().ok());
        data::delete_everything(&self.dir, engine, vault)
    }

    pub(crate) fn engine(&self) -> Reply<&Engine> {
        match self.engine.get() {
            Some(started) => started.as_ref().map_err(Clone::clone),
            None => Err("Sextant is still starting".into()),
        }
    }
}

pub(crate) fn msg(err: impl Display) -> String {
    err.to_string()
}

pub(crate) const MAX_ROWS: usize = 200;

/// `false` while the engine starts, `true` once it runs, an error if it could not start. The UI
/// asks again whenever the `ready` event arrives.
#[tauri::command]
pub async fn engine_status(state: State<'_, AppState>) -> Reply<bool> {
    match state.engine.get() {
        None => Ok(false),
        Some(started) => started.as_ref().map(|_| true).map_err(Clone::clone),
    }
}

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
    let settings = prefs::load(&state.dir).map_err(msg)?;
    let sources = &settings.sources;
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
    if settings.live.ocr {
        needed.push(Permission {
            name: "screen-recording",
            status: platform::screen_recording_status(),
        });
    }
    if settings.live.metadata {
        needed.push(Permission {
            name: "accessibility",
            status: platform::accessibility_status(),
        });
        needed.push(Permission {
            name: "automation",
            status: platform::automation_status(),
        });
    }
    Ok(needed)
}

/// Shows the system prompt for `calendar`, `reminders`, `screen-recording`, `accessibility` or
/// `automation`. Returns whether access is granted.
#[tauri::command]
pub async fn request_permission(name: String) -> Reply<bool> {
    tauri::async_runtime::spawn_blocking(move || platform::request_permission(&name))
        .await
        .map_err(msg)?
        .map_err(msg)
}

/// Opens `full-disk-access`, `calendars`, `reminders`, `screen-recording`, `accessibility` or
/// `automation` in System Settings.
#[tauri::command]
pub async fn open_settings_pane(pane: String) -> Reply<()> {
    platform::open_settings_pane(&pane).map_err(msg)
}

/// The folder dialog with `prompt` as its message. `None` when cancelled.
#[tauri::command]
pub async fn pick_folder(prompt: String) -> Reply<Option<String>> {
    tauri::async_runtime::spawn_blocking(move || platform::pick_folder(&prompt))
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
    state.engine()?.start_backfill().map_err(msg)
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
        .engine()?
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

#[derive(Serialize)]
pub struct Models {
    installed: bool,
    downloading: bool,
    audio: bool,
    bytes: u64,
}

/// What the audio consent step and the settings show.
#[tauri::command]
pub async fn models_status(state: State<'_, AppState>) -> Reply<Models> {
    let engine = state.engine()?;
    Ok(Models {
        installed: engine.models_installed(),
        downloading: engine.downloading(),
        audio: prefs::load(&state.dir).map_err(msg)?.live.audio,
        bytes: models::total_bytes(models::ASSETS),
    })
}

/// Starts the one-time model download. The UI calls it only after the user agreed. Progress
/// arrives as `models-progress` events, then `models-done` or `models-failed`.
#[tauri::command]
pub async fn download_models(state: State<'_, AppState>) -> Reply<()> {
    state.engine()?.download_models().map_err(msg)
}

#[tauri::command]
pub async fn cancel_download(state: State<'_, AppState>) -> Reply<()> {
    state.engine()?.cancel_download();
    Ok(())
}

/// Switches call audio on or off. Off closes every stream within a second.
#[tauri::command]
pub async fn set_audio(state: State<'_, AppState>, on: bool) -> Reply<Settings> {
    state.engine()?.set_audio(on).map_err(msg)?;
    prefs::load(&state.dir).map_err(msg)
}

#[tauri::command]
pub async fn finish_onboarding(state: State<'_, AppState>) -> Reply<Settings> {
    let settings = prefs::update(&state.dir, |s| s.onboarded = true).map_err(msg)?;
    state
        .engine()?
        .start_capture(settings.clone())
        .map_err(msg)?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::Duration;

    use sextant_core::fixtures::TEST_KEY;

    use super::*;
    use crate::keys::KeyVault;

    struct FakeVault(AtomicBool);

    impl KeyVault for FakeVault {
        fn delete(&self) -> anyhow::Result<()> {
            self.0.store(false, Ordering::SeqCst);
            Ok(())
        }
    }

    fn temp(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sextant-state-{}-{label}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("test setup");
        dir
    }

    #[test]
    fn the_engine_is_unavailable_until_the_start_up_thread_reports() {
        let state = AppState::new(PathBuf::new());
        assert_eq!(
            state.engine().err().as_deref(),
            Some("Sextant is still starting")
        );
        assert!(state.engine.set(Err("no key".into())).is_ok());
        assert_eq!(state.engine().err().as_deref(), Some("no key"));
    }

    #[test]
    fn a_delete_during_start_up_waits_for_the_engine_and_leaves_nothing_behind() {
        let dir = temp("during");
        let state = Arc::new(AppState::new(dir.clone()));
        let vault = Arc::new(FakeVault(AtomicBool::new(true)));
        let (opening, opened) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let start = thread::spawn({
            let state = state.clone();
            move || {
                state.start(|dir| {
                    opening.send(()).expect("test setup");
                    released.recv().expect("test setup");
                    Engine::start(dir, &TEST_KEY, |_, _| {})
                });
            }
        });
        opened.recv().expect("test setup");
        let wipe = thread::spawn({
            let (state, vault) = (state.clone(), vault.clone());
            move || state.delete_everything(&*vault)
        });
        thread::sleep(Duration::from_millis(100));
        release.send(()).expect("test setup");

        start.join().expect("start-up thread");
        wipe.join().expect("wipe thread").expect("deletes");

        assert_eq!(fs::read_dir(&dir).expect("the folder stays").count(), 0);
        assert!(!vault.0.load(Ordering::SeqCst), "the key is gone");
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn a_delete_before_the_engine_starts_stops_it_from_starting() {
        let dir = temp("before");
        fs::write(dir.join("settings.json"), "{}").expect("test setup");
        let state = AppState::new(dir.clone());
        let vault = FakeVault(AtomicBool::new(true));

        state.delete_everything(&vault).expect("deletes");
        state.start(|dir| Engine::start(dir, &TEST_KEY, |_, _| {}));

        assert_eq!(fs::read_dir(&dir).expect("the folder stays").count(), 0);
        assert!(state.engine().is_err());
        fs::remove_dir_all(dir).expect("cleanup");
    }
}
