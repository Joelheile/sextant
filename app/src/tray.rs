//! The menu bar icon (plan.md §9): filled while capturing, outline while paused, and a menu with
//! the status line, "Pause 1 h", Open and Quit. The status refreshes every few seconds.

use std::thread;
use std::time::Duration;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, Wry};

use crate::commands::AppState;
use crate::engine::CaptureState;

const REFRESH_EVERY: Duration = Duration::from_secs(2);

/// The handles to change after the menu is built.
pub struct Tray {
    icon: TrayIcon,
    status: MenuItem<Wry>,
    pause: MenuItem<Wry>,
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", "Starting", false, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "Pause 1 h", false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Open Sextant", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Sextant", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&status, &pause, &open, &quit])?;
    let icon = TrayIconBuilder::new()
        .icon(tauri::include_image!("icons/tray-paused.png"))
        .icon_as_template(true)
        .tooltip("Sextant")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => {
                if let Err(err) = open_window(app) {
                    eprintln!("opening the window failed: {err}");
                }
            }
            "pause" => {
                toggle_pause(app);
                refresh(app);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    app.manage(Tray {
        icon,
        status,
        pause,
    });
    let handle = app.clone();
    thread::Builder::new()
        .name("sextant-tray".into())
        .spawn(move || {
            loop {
                let on_main = handle.clone();
                let _ = handle.run_on_main_thread(move || refresh(&on_main));
                thread::sleep(REFRESH_EVERY);
            }
        })
        .map_err(tauri::Error::Io)?;
    Ok(())
}

fn state(app: &AppHandle) -> CaptureState {
    match app.state::<AppState>().engine.get() {
        Some(Ok(engine)) => engine.capture_state(),
        _ => CaptureState::Off,
    }
}

fn toggle_pause(app: &AppHandle) {
    if let Some(Ok(engine)) = app.state::<AppState>().engine.get() {
        match engine.capture_state() {
            CaptureState::Paused(_) => engine.resume_capture(),
            CaptureState::Capturing => engine.pause_capture(),
            CaptureState::Off => {}
        }
    }
}

/// Local clock time of Unix time `ts` as `HH:MM`.
fn clock_time(ts: i64) -> String {
    let local = ts + i64::from(crate::clock::utc_offset(ts));
    let minutes = local.rem_euclid(86_400) / 60;
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

fn refresh(app: &AppHandle) {
    let Some(tray) = app.try_state::<Tray>() else {
        return;
    };
    let state = state(app);
    let (text, pause, icon): (String, &str, Image<'static>) = match state {
        CaptureState::Off => (
            "Capture is off".into(),
            "Pause 1 h",
            tauri::include_image!("icons/tray-paused.png"),
        ),
        CaptureState::Capturing => (
            "Capturing".into(),
            "Pause 1 h",
            tauri::include_image!("icons/tray-on.png"),
        ),
        CaptureState::Paused(until) => (
            format!("Paused until {}", clock_time(until)),
            "Resume",
            tauri::include_image!("icons/tray-paused.png"),
        ),
    };
    let _ = tray.status.set_text(text);
    let _ = tray.pause.set_text(pause);
    let _ = tray.pause.set_enabled(state != CaptureState::Off);
    let _ = tray.icon.set_icon(Some(icon));
    let _ = tray.icon.set_icon_as_template(true);
}

pub fn open_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        return window.set_focus();
    }
    WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
        .title("Sextant")
        .inner_size(880.0, 620.0)
        .min_inner_size(560.0, 420.0)
        .build()?;
    Ok(())
}
