//! Privacy (plan.md §9), for the window and the control socket. Reads go through the
//! read-only connection, changes through the worker thread. Text from the database is redacted
//! again on the way out, as in the MCP server.

use sextant_core::{Live, Sent, Settings};

use crate::commands::{AppState, Reply, msg};
use crate::keys::Keychain;
use crate::{clock, connect, platform};

const RETENTION_CHOICES: [Option<u32>; 4] = [Some(30), Some(90), Some(365), None];
const STORAGE_CAP_CHOICES: [Option<u32>; 5] =
    [Some(1000), Some(2000), Some(5000), Some(10000), None];
/// "Delete the last 15 minutes, hour or day".
const FORGET_CHOICES: [i64; 3] = [15, 60, 1440];
const SENT_ROWS: usize = 50;

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

pub(crate) fn sent(state: &AppState) -> Reply<Vec<Sent>> {
    state
        .engine()?
        .read(|store| store.sent(SENT_ROWS))
        .map_err(msg)
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
