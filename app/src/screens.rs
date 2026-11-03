//! Privacy (plan.md §9), for the window and the control socket. Reads go through the
//! read-only connection, changes through the worker thread. Text from the database is redacted
//! again on the way out, as in the MCP server.

use serde::Serialize;
use sextant_core::{Live, Sent, Settings};
use tauri::{AppHandle, Manager, State};

use crate::commands::{AppState, Reply, blocking, msg};
use crate::keys::Keychain;
use crate::{clock, connect, data, platform};

const RETENTION_CHOICES: [Option<u32>; 4] = [Some(30), Some(90), Some(365), None];
const STORAGE_CAP_CHOICES: [Option<u32>; 5] =
    [Some(1000), Some(2000), Some(5000), Some(10000), None];
/// "Delete the last 15 minutes, hour or day".
const FORGET_CHOICES: [i64; 3] = [15, 60, 1440];
const SENT_ROWS: usize = 50;

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
pub async fn set_excluded_apps(app: AppHandle, apps: Vec<String>) -> Reply<Settings> {
    blocking(app, move |state| exclude_apps(state, apps)).await
}

pub(crate) fn exclude_apps(state: &AppState, apps: Vec<String>) -> Reply<Settings> {
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
pub async fn set_live(app: AppHandle, live: Live) -> Reply<Settings> {
    blocking(app, move |state| capture_sources(state, live)).await
}

pub(crate) fn capture_sources(state: &AppState, live: Live) -> Reply<Settings> {
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
pub async fn set_excluded_sites(app: AppHandle, sites: Vec<String>) -> Reply<Settings> {
    blocking(app, move |state| exclude_sites(state, sites)).await
}

pub(crate) fn exclude_sites(state: &AppState, sites: Vec<String>) -> Reply<Settings> {
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
    blocking(app, move |state| cap_storage(state, mb)).await
}

pub(crate) fn cap_storage(state: &AppState, mb: Option<u32>) -> Reply<Settings> {
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
}

/// Deletes everything captured in the last 15 minutes, hour or day.
#[tauri::command]
pub async fn forget_recent(app: AppHandle, minutes: i64) -> Reply<()> {
    blocking(app, move |state| forget(state, minutes)).await
}

pub(crate) fn forget(state: &AppState, minutes: i64) -> Reply<()> {
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
}

/// What agents received: each MCP answer, the bridge's included, newest first.
#[tauri::command]
pub async fn sent_log(app: AppHandle) -> Reply<Vec<Sent>> {
    blocking(app, sent).await
}

pub(crate) fn sent(state: &AppState) -> Reply<Vec<Sent>> {
    state
        .engine()?
        .read(|store| store.sent(SENT_ROWS))
        .map_err(msg)
}

/// Keeps raw data for 30, 90 or 365 days, or forever (`None`), and applies it now.
#[tauri::command]
pub async fn set_retention(app: AppHandle, days: Option<u32>) -> Reply<Settings> {
    blocking(app, move |state| keep_for(state, days)).await
}

pub(crate) fn keep_for(state: &AppState, days: Option<u32>) -> Reply<Settings> {
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
    tauri::async_runtime::spawn_blocking(move || remove_sextant(&handle.state::<AppState>()))
        .await
        .map_err(msg)??;
    app.exit(0);
    Ok(())
}

/// Everything "Uninstall" does before Sextant quits.
pub(crate) fn remove_sextant(state: &AppState) -> Reply<()> {
    (|| -> anyhow::Result<()> {
        state.delete_everything(&Keychain)?;
        connect::remove_from_desktop_config(&connect::desktop_config_path()?)?;
        if !cfg!(debug_assertions) {
            platform::trash_app()?;
        }
        Ok(())
    })()
    .map_err(|err| format!("{err:#}"))
}
