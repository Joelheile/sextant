mod audio;
mod bench;
mod capture;
mod clock;
mod commands;
mod connect;
mod data;
mod engine;
mod home;
mod judge;
mod keys;
mod mcp;
mod models;
mod panel;
mod platform;
mod popover;
mod prefs;
mod rewind;
mod screens;
mod tray;

use std::thread;

use anyhow::Context;
use tauri::{AppHandle, Emitter, Manager, RunEvent};
use tauri_plugin_global_shortcut::ShortcutState;

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
                }
            }
            Ok(_) => {}
            Err(err) => eprintln!("capture did not start: {err:#}"),
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
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcut(rewind::SHORTCUT)
                .context("registering the search shortcut")?
                .with_handler(|app, _, event| {
                    if event.state() == ShortcutState::Pressed
                        && let Err(err) = rewind::toggle(app)
                    {
                        eprintln!("opening search failed: {err}");
                    }
                })
                .build(),
        )
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
            commands::first_map,
            commands::connect_info,
            commands::connect_claude_desktop,
            commands::set_mcp_switches,
            commands::models_status,
            commands::download_models,
            commands::cancel_download,
            commands::set_audio,
            commands::finish_onboarding,
            screens::workflow_overview,
            screens::answer_enjoyment,
            screens::workflow_detail,
            screens::correct_workflow,
            screens::automation_plan,
            screens::write_skill,
            screens::report_handoff,
            screens::results,
            screens::rewind_ask,
            screens::rewind_history,
            screens::rewind_results,
            screens::rewind_fit,
            screens::rewind_hide,
            screens::weekly_report,
            screens::save_report_image,
            screens::storage_used,
            screens::set_excluded_apps,
            screens::set_live,
            screens::set_retention,
            screens::delete_everything,
            popover::menu_status,
            popover::insights,
            popover::menu_action,
        ])
        .setup(move |app| {
            app.manage(AppState::new(dir));
            tray::build(app.handle())?;
            tray::open_window(app.handle())?;
            rewind::build(app.handle())?;
            popover::build(app.handle())?;
            let handle = app.handle().clone();
            thread::Builder::new()
                .name("sextant-start".into())
                .spawn(move || start_engine(&handle))
                .context("starting the start-up thread")?;
            judge::spawn(app.handle().clone())?;
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
