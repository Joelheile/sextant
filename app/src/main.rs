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
mod login;
mod mcp;
mod models;
mod panel;
mod platform;
mod prefs;
mod screens;
mod tray;
mod tui;
mod update;

use std::path::Path;
use std::thread;
use std::time::Duration;

use anyhow::Context;
use tauri::{AppHandle, Manager, RunEvent};

use crate::commands::AppState;
use crate::engine::Engine;

const START_POLL: Duration = Duration::from_millis(200);
const START_POLLS: u32 = 50;

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

/// Shows the panel of the copy launchd starts, once it answers, as a double-click on the app
/// expects.
fn show_when_started(dir: &Path) -> anyhow::Result<()> {
    for _ in 0..START_POLLS {
        if control::running(dir) {
            return control::call(dir, &control::Request::Show);
        }
        thread::sleep(START_POLL);
    }
    anyhow::bail!("Sextant did not start through its launch agent")
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
    // At login launchd may have started Sextant already; opening the app again shows its panel.
    if control::running(&dir) {
        return control::call(&dir, &control::Request::Show);
    }
    match login::hand_to_launchd() {
        Ok(true) => return show_when_started(&dir),
        Ok(false) => {}
        Err(err) => eprintln!("Sextant runs without the launch agent: {err:#}"),
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![panel::ask])
        .setup(move |app| {
            // Menu bar only: no Dock icon or app switcher entry, in cargo builds too.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            app.manage(AppState::new(dir));
            tray::build(app.handle())?;
            panel::build(app.handle())?;
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
            update::spawn_daily(app.handle().clone())?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .context("building the Tauri app")?
        .run(|app, event| match event {
            // The tray keeps running until Quit.
            RunEvent::ExitRequested {
                code: None, api, ..
            } => api.prevent_exit(),
            // Opening Sextant.app while it runs, from Finder, Spotlight or the Dock.
            #[cfg(target_os = "macos")]
            RunEvent::Reopen { .. } => tray::open_panel(app),
            _ => {}
        });
    Ok(())
}
