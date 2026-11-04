//! The menu bar icon: filled while capturing, outline while paused, a dot while a call
//! is transcribed and an exclamation mark when capture is on but macOS blocks it. A click opens a
//! menu with the status line, "Pause 1 h", "Stop audio", Open and Quit. The status refreshes
//! every few seconds. Open starts `sextant tui` in Terminal, or `sextant setup` until setup is
//! done.

use std::path::Path;
use std::thread;
use std::time::Duration;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager, Wry};

use crate::commands::AppState;
use crate::engine::{AudioState, CaptureState};
use crate::{platform, prefs};

const REFRESH_EVERY: Duration = Duration::from_secs(2);

/// What a menu entry does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuAction {
    Pause,
    StopAudio,
    Open,
    Quit,
}

impl MenuAction {
    const MENU: [Self; 4] = [Self::Pause, Self::StopAudio, Self::Open, Self::Quit];

    fn id(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::StopAudio => "stop-audio",
            Self::Open => "open",
            Self::Quit => "quit",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::MENU.into_iter().find(|action| action.id() == id)
    }
}

fn perform(app: &AppHandle, action: MenuAction) -> tauri::Result<()> {
    match action {
        MenuAction::Pause => {
            toggle_pause(app);
            refresh(app);
        }
        MenuAction::StopAudio => {
            turn_audio_off(app);
            refresh(app);
        }
        MenuAction::Open => open_terminal(&app.state::<AppState>().dir),
        MenuAction::Quit => app.exit(0),
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
    let menu = Menu::with_items(app, &[&status, &pause, &stop_audio, &open, &quit])?;
    let icon = TrayIconBuilder::new()
        .icon(tauri::include_image!("icons/tray-paused.png"))
        .icon_as_template(true)
        .tooltip("Sextant")
        .menu(&menu)
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

fn capture_state(state: &AppState) -> CaptureState {
    match state.engine.get() {
        Some(Ok(engine)) => engine.capture_state(),
        _ => CaptureState::Off,
    }
}

fn audio_state(state: &AppState) -> AudioState {
    match state.engine.get() {
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

/// Which tray icon shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Icon {
    On,
    Paused,
    Alert,
    Listening,
}

impl Icon {
    fn image(self) -> Image<'static> {
        match self {
            Self::On => tauri::include_image!("icons/tray-on.png"),
            Self::Paused => tauri::include_image!("icons/tray-paused.png"),
            Self::Alert => tauri::include_image!("icons/tray-alert.png"),
            Self::Listening => tauri::include_image!("icons/tray-listening.png"),
        }
    }
}

/// What the tray and `sextant status` show.
pub struct Look {
    pub text: String,
    icon: Icon,
    pub capture: CaptureState,
    pub audio: AudioState,
}

pub fn look(state: &AppState) -> Look {
    let capture = capture_state(state);
    let (text, icon) = match capture {
        CaptureState::Off => ("Capture is off".into(), Icon::Paused),
        CaptureState::Capturing => ("Capturing".into(), Icon::On),
        CaptureState::Paused(until) => (
            format!("Paused until {}", crate::clock::hour_minute(until)),
            Icon::Paused,
        ),
        CaptureState::Stalled(_) => ("Capture stopped: restart Sextant".into(), Icon::Alert),
    };
    let blocked = capture == CaptureState::Capturing
        && !crate::commands::capture_blockers(&state.dir).is_empty();
    let (text, icon) = if blocked {
        (
            "Capture is blocked: run `sextant setup`".into(),
            Icon::Alert,
        )
    } else {
        (text, icon)
    };
    let audio = audio_state(state);
    let (text, icon) = match audio {
        AudioState::Transcribing => (format!("{text}, transcribing a call"), Icon::Listening),
        AudioState::Waiting => (format!("{text}, audio on"), icon),
        AudioState::Off => (text, icon),
    };
    Look {
        text,
        icon,
        capture,
        audio,
    }
}

fn refresh(app: &AppHandle) {
    let Some(tray) = app.try_state::<Tray>() else {
        return;
    };
    let look = look(&app.state::<AppState>());
    let pause = match look.capture {
        CaptureState::Paused(_) => "Resume",
        _ => "Pause 1 h",
    };
    let _ = tray.status.set_text(look.text);
    let _ = tray.stop_audio.set_enabled(look.audio != AudioState::Off);
    let _ = tray.pause.set_text(pause);
    let _ = tray.pause.set_enabled(look.capture != CaptureState::Off);
    let _ = tray.icon.set_icon(Some(look.icon.image()));
    let _ = tray.icon.set_icon_as_template(true);
}

/// The first launch has nothing to show in the menu yet, so it opens setup.
pub fn open_on_first_launch(app: &AppHandle) {
    let dir = &app.state::<AppState>().dir;
    if !prefs::load(dir).is_ok_and(|settings| settings.onboarded) {
        open_terminal(dir);
    }
}

pub fn open_terminal(dir: &Path) {
    let opened = std::env::current_exe()
        .map_err(anyhow::Error::from)
        .and_then(|exe| {
            let command = if prefs::load(dir).is_ok_and(|settings| settings.onboarded) {
                "tui"
            } else {
                "setup"
            };
            let script = dir.join("open.command");
            std::fs::write(&script, terminal_script(&exe, dir, command))?;
            std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
            platform::open_in_terminal(&script)
        });
    if let Err(err) = opened {
        eprintln!("opening Terminal failed: {err:#}");
    }
}

/// A shell script that runs `sextant <command>`. Dev builds pass their data folder along.
fn terminal_script(exe: &Path, dir: &Path, command: &str) -> String {
    let home = if cfg!(debug_assertions) {
        format!("export SEXTANT_HOME={}\n", quote(dir))
    } else {
        String::new()
    };
    format!("#!/bin/sh\n{home}exec {} {command}\n", quote(exe))
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_terminal_script_quotes_paths_and_keeps_dev_data_in_dev() {
        let script = terminal_script(
            Path::new("/Applications/Joel's Apps/Sextant.app/Contents/MacOS/sextant"),
            Path::new("/tmp/dev data"),
            "tui",
        );
        assert!(script.starts_with("#!/bin/sh\n"));
        assert!(script.ends_with(
            "exec '/Applications/Joel'\\''s Apps/Sextant.app/Contents/MacOS/sextant' tui\n"
        ));
        assert_eq!(
            script.contains("export SEXTANT_HOME='/tmp/dev data'\n"),
            cfg!(debug_assertions)
        );
    }
}
