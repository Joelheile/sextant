mod agent;
mod audio;
mod bench;
mod capture;
mod cli;
mod clock;
mod commands;
mod connect;
mod control;
mod data;
mod debug;
mod engine;
mod home;
mod keys;
mod mcp;
mod models;
mod platform;
mod prefs;
mod screens;
mod tray;
mod tui;

use std::thread;

use anyhow::Context;
use tauri::{AppHandle, Manager, RunEvent};

use crate::commands::AppState;
use crate::engine::Engine;

/// Looks up the key (the Keychain may prompt and wait) and opens the database off the main
/// thread. "Delete everything" waits for this.
fn start_engine(app: &AppHandle) {
    app.state::<AppState>().start(|dir| {
        let key = keys::load_or_create()?;
        let engine = Engine::start(dir, &key)?;
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
    if args
        .first()
        .is_some_and(|command| cli::COMMANDS.contains(&command.as_str()))
    {
        return cli::run(&dir, &args);
    }

    tauri::Builder::default()
        .setup(move |app| {
            app.manage(AppState::new(dir));
            tray::build(app.handle())?;
            if let Err(err) = control::serve(app.handle().clone()) {
                eprintln!("the sextant command will not reach this app: {err:#}");
            }
            tray::open_on_first_launch(app.handle());
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
            // The tray keeps running until Quit.
            if let RunEvent::ExitRequested {
                code: None, api, ..
            } = event
            {
                api.prevent_exit();
            }
        });
    Ok(())
}
