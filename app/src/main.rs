mod bench;
mod capture;
mod clock;
mod commands;
mod connect;
mod engine;
mod home;
mod keys;
mod mcp;
mod platform;
mod prefs;
mod tray;

use std::sync::OnceLock;
use std::thread;

use anyhow::Context;
use tauri::{AppHandle, Emitter, Manager, RunEvent};

use crate::commands::AppState;
use crate::engine::Engine;

/// Looks up the key (the Keychain may prompt and wait) and opens the database off the main
/// thread, then tells the window with a `ready` event.
fn start_engine(app: &AppHandle) {
    let state = app.state::<AppState>();
    let emitter = app.clone();
    let started = keys::load_or_create()
        .and_then(|key| {
            Engine::start(&state.dir, &key, move |name, payload| {
                let _ = emitter.emit(name, payload);
            })
        })
        .map_err(|err| format!("{err:#}"));
    let _ = state.engine.set(started);
    if let Some(Ok(engine)) = state.engine.get() {
        // Capture starts only after onboarding: the permissions are asked there.
        match prefs::load(&state.dir) {
            Ok(settings) if settings.onboarded => {
                if let Err(err) = engine.start_capture(settings) {
                    eprintln!("capture did not start: {err:#}");
                }
            }
            Ok(_) => {}
            Err(err) => eprintln!("capture did not start: {err:#}"),
        }
    }
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
            commands::pick_folder,
            commands::save_choices,
            commands::start_backfill,
            commands::first_map,
            commands::connect_info,
            commands::connect_claude_desktop,
            commands::set_mcp_switches,
            commands::finish_onboarding,
        ])
        .setup(move |app| {
            app.manage(AppState {
                dir,
                engine: OnceLock::new(),
            });
            tray::build(app.handle())?;
            tray::open_window(app.handle())?;
            let handle = app.handle().clone();
            thread::Builder::new()
                .name("sextant-start".into())
                .spawn(move || start_engine(&handle))
                .context("starting the start-up thread")?;
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
