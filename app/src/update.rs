//! Updates the installed app from its GitHub releases (docs/RELEASING.md): at start, then once a
//! day. The updater plugin checks the download against the public key in `tauri.conf.json`
//! before it replaces the app, then Sextant opens again. Dev builds and cargo builds never update.

use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

use crate::{clock, control, login};

const DAY_S: i64 = 24 * 60 * 60;
/// How often the thread wakes. It reads the wall clock, because a sleeping Mac pauses
/// `thread::sleep`: a single 24-hour sleep could take a week on a laptop that sleeps at night.
/// A failed check, such as one with no network yet after login, runs again on the next wake.
const WAKE_EVERY: Duration = Duration::from_secs(60 * 60);

pub fn spawn_daily(app: AppHandle) -> Result<()> {
    let installed = !cfg!(debug_assertions) && login::bundled_exe().is_some();
    if !installed {
        return Ok(());
    }
    thread::Builder::new()
        .name("sextant-update".into())
        .spawn(move || {
            let mut last_check = None;
            loop {
                let now = clock::now();
                if due(last_check, now) {
                    match tauri::async_runtime::block_on(install(&app)) {
                        Ok(true) => control::restart(&app),
                        Ok(false) => last_check = Some(now),
                        Err(err) => eprintln!("the update check failed: {err:#}"),
                    }
                }
                thread::sleep(WAKE_EVERY);
            }
        })
        .context("starting the update thread")?;
    Ok(())
}

/// Whether to check now: never checked yet, or the last check that worked is a day old.
fn due(last_check: Option<i64>, now: i64) -> bool {
    last_check.is_none_or(|last| now - last >= DAY_S)
}

/// Installs a newer release if there is one, and returns whether it did.
async fn install(app: &AppHandle) -> Result<bool> {
    let Some(update) = app.updater()?.check().await? else {
        return Ok(false);
    };
    update.download_and_install(|_, _| {}, || {}).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_at_start_then_a_day_after_the_last_check_that_worked() {
        let start = 1_800_000_000;
        assert!(due(None, start));
        assert!(!due(Some(start), start + DAY_S - 1));
        assert!(due(Some(start), start + DAY_S));
    }
}
