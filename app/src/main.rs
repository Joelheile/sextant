mod agent;
mod audio;
mod bench;
mod capture;
mod clock;
mod commands;
mod connect;
mod data;
mod debug;
mod engine;
mod home;
mod keys;
mod mcp;
mod models;
mod panel;
mod platform;
mod popover;
mod prefs;
mod screens;
mod tray;

use std::thread;

use anyhow::Context;
use tauri::{AppHandle, Emitter, Manager, RunEvent};

use crate::commands::AppState;
use crate::engine::Engine;

/// Looks up the key (the Keychain may prompt and wait) and opens the database off the main
/// thread, then tells the window with a `ready` event. "Delete everything" waits for this.
fn start_engine(app: &AppHandle) {
    let emitter = app.clone();
    app.state::<AppState>().start(|dir| {
        let key = keys::load_or_create()?;
        let engine = Engine::start(dir, &key, move |name, payload| {
            let _ = emitter.emit(name, payload);
        })?;
        // Capture starts only after onboarding: the permissions are asked there.
        match prefs::load(dir) {
            Ok(settings) if settings.onboarded => {
                if let Err(err) = engine.start_capture(settings) {
                    eprintln!("capture did not start: {err:#}");
                    debug::record("starting capture");
                }
            }
            Ok(_) => {}
            Err(err) => {
                eprintln!("capture did not start: {err:#}");
                debug::record("reading settings");
            }
        }
        Ok(engine)
    });
    let _ = app.emit("ready", ());
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The bench never resolves the data folder, so it cannot touch real data.
    if args.first().map(String::as_str) == Some("bench") {
        return bench::run(&args[1..]);
    }
    let dir = home::data_dir()?;
    if args.first().map(String::as_str) == Some("mcp") {
        return mcp::run(&dir);
    }

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::engine_status,
            commands::get_state,
            commands::permission_status,
            commands::request_permission,
            commands::open_settings_pane,
            commands::relaunch,
            commands::pick_folder,
            commands::save_choices,
            commands::start_backfill,
            commands::connect_info,
            commands::connect_claude_desktop,
            commands::set_mcp_switches,
            commands::models_status,
            commands::download_models,
            commands::cancel_download,
            commands::set_audio,
            commands::finish_onboarding,
            screens::storage_used,
            debug::debug_report,
            screens::set_excluded_apps,
            screens::set_live,
            screens::set_retention,
            screens::delete_everything,
            screens::set_excluded_sites,
            screens::set_storage_cap,
            screens::forget_recent,
            screens::sent_log,
            screens::uninstall,
            popover::menu_status,
            popover::insights,
            popover::menu_action,
            agent::agent_status,
            agent::set_agent,
            agent::run_agent,
        ])
        .setup(move |app| {
            app.manage(AppState::new(dir));
            tray::build(app.handle())?;
            tray::open_window(app.handle())?;
            popover::build(app.handle())?;
            let handle = app.handle().clone();
            thread::Builder::new()
                .name("sextant-start".into())
                .spawn(move || start_engine(&handle))
                .context("starting the start-up thread")?;
            agent::spawn_weekly(app.handle().clone())?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .context("building the Tauri app")?
        .run(|_, event| {
            // Closing the window destroys it; the tray keeps running until Quit.
            if let RunEvent::ExitRequested {
                code: None, api, ..
            } = event
            {
                api.prevent_exit();
            }
        });
    Ok(())
}
