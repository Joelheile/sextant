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

use anyhow::Context;
use tauri::{Emitter, Manager, RunEvent};

use crate::commands::AppState;
use crate::engine::Engine;

fn main() -> anyhow::Result<()> {
    let dir = home::data_dir()?;
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        return mcp::run(&dir);
    }

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
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
            let key = keys::load_or_create()?;
            let handle = app.handle().clone();
            let engine = Engine::start(&dir, &key, move |name, payload| {
                let _ = handle.emit(name, payload);
            })?;
            app.manage(AppState { dir, engine });
            tray::build(app.handle())?;
            tray::open_window(app.handle())?;
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
