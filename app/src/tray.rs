//! The menu bar icon (plan.md §9): filled while capturing, outline while paused, a dot while a call
//! is transcribed and an exclamation mark when capture is on but macOS blocks it. A click opens the
//! popover; a right-click opens a menu with the status line, "Pause 1 h", "Stop audio", Search,
//! Open and Quit. The status refreshes every few seconds.

use std::thread;
use std::time::Duration;

use serde::Deserialize;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, Wry};

use crate::commands::AppState;
use crate::engine::{AudioState, CaptureState};

const REFRESH_EVERY: Duration = Duration::from_secs(2);

/// What a menu entry or a popover button does. `Hide` closes the popover and only the popover
/// sends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MenuAction {
    Pause,
    StopAudio,
    Search,
    Open,
    Quit,
    Hide,
}

impl MenuAction {
    const MENU: [Self; 5] = [
        Self::Pause,
        Self::StopAudio,
        Self::Search,
        Self::Open,
        Self::Quit,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::StopAudio => "stop-audio",
            Self::Search => "search",
            Self::Open => "open",
            Self::Quit => "quit",
            Self::Hide => "hide",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::MENU.into_iter().find(|action| action.id() == id)
    }
}

/// Runs the action for both the native menu and the popover.
pub fn perform(app: &AppHandle, action: MenuAction) -> tauri::Result<()> {
    match action {
        MenuAction::Pause => {
            toggle_pause(app);
            refresh(app);
        }
        MenuAction::StopAudio => {
            turn_audio_off(app);
            refresh(app);
        }
        MenuAction::Search => crate::rewind::show(app)?,
        MenuAction::Open => open_window(app)?,
        MenuAction::Quit => app.exit(0),
        MenuAction::Hide => crate::popover::hide(app)?,
    }
    Ok(())
}

/// The handles to change after the menu is built.
pub struct Tray {
    icon: TrayIcon,
    status: MenuItem<Wry>,
    pause: MenuItem<Wry>,
    stop_audio: MenuItem<Wry>,
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", "Starting", false, None::<&str>)?;
    let pause = MenuItem::with_id(
        app,
        MenuAction::Pause.id(),
        "Pause 1 h",
        false,
        None::<&str>,
    )?;
    let stop_audio = MenuItem::with_id(
        app,
        MenuAction::StopAudio.id(),
        "Stop audio",
        false,
        None::<&str>,
    )?;
    let search = MenuItem::with_id(
        app,
        MenuAction::Search.id(),
        "Search",
        true,
        Some(crate::rewind::SHORTCUT),
    )?;
    let open = MenuItem::with_id(
        app,
        MenuAction::Open.id(),
        "Open Sextant",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(
        app,
        MenuAction::Quit.id(),
        "Quit Sextant",
        true,
        None::<&str>,
    )?;
    let menu = Menu::with_items(app, &[&status, &pause, &stop_audio, &search, &open, &quit])?;
    let icon = TrayIconBuilder::new()
        .icon(tauri::include_image!("icons/tray-paused.png"))
        .icon_as_template(true)
        .tooltip("Sextant")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|icon, event| {
            if let TrayIconEvent::Click {
                rect,
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
                && let Err(err) = crate::popover::toggle(icon.app_handle(), rect)
            {
                eprintln!("opening the popover failed: {err}");
            }
        })
        .on_menu_event(|app, event| {
            if let Some(action) = MenuAction::from_id(event.id().as_ref())
                && let Err(err) = perform(app, action)
            {
                eprintln!("menu action {action:?} failed: {err}");
            }
        })
        .build(app)?;
    app.manage(Tray {
        icon,
        status,
        pause,
        stop_audio,
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

fn audio_state(app: &AppHandle) -> AudioState {
    match app.state::<AppState>().engine.get() {
        Some(Ok(engine)) => engine.audio_state(),
        _ => AudioState::Off,
    }
}

/// One click: all streams and taps close within a second. The setting stays off until the user
/// switches audio on again.
fn turn_audio_off(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(Ok(engine)) = state.engine.get() else {
        return;
    };
    if let Err(err) = engine.set_audio(false) {
        eprintln!("stopping audio failed: {err:#}");
    }
}

fn toggle_pause(app: &AppHandle) {
    if let Some(Ok(engine)) = app.state::<AppState>().engine.get() {
        match engine.capture_state() {
            CaptureState::Paused(_) => engine.resume_capture(),
            CaptureState::Capturing | CaptureState::Stalled(_) => engine.pause_capture(),
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
        CaptureState::Stalled(_) => (
            "Capture stopped: restart Sextant".into(),
            "Pause 1 h",
            tauri::include_image!("icons/tray-alert.png"),
        ),
    };
    let blocked = state == CaptureState::Capturing
        && !crate::commands::capture_blockers(&app.state::<AppState>().dir).is_empty();
    let (text, icon) = if blocked {
        (
            "Capture is blocked: allow access in Sextant".into(),
            tauri::include_image!("icons/tray-alert.png"),
        )
    } else {
        (text, icon)
    };
    let audio = audio_state(app);
    let (text, icon) = match audio {
        AudioState::Transcribing => (
            format!("{text}, transcribing a call"),
            tauri::include_image!("icons/tray-listening.png"),
        ),
        AudioState::Waiting => (format!("{text}, audio on"), icon),
        AudioState::Off => (text, icon),
    };
    let _ = tray.status.set_text(text);
    let _ = tray.stop_audio.set_enabled(audio != AudioState::Off);
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
