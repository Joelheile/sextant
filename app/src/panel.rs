//! The menu bar panel: a left click on the tray icon opens it under the icon, and it hides when
//! it loses focus. It shows the week as a funnel, from what Sextant captured to the tasks, the
//! work that repeats and the saved routines, with pause and audio controls, and walks through
//! setup until it is done. The page is plain HTML and JS in `app/ui`, built once at start-up and
//! hidden, so a click shows it without a delay. It reaches the app through the `ask` command,
//! which answers only the requests listed in `Ask`.

use serde::Deserialize;
use serde_json::Value;
use tauri::utils::config::WindowEffectsConfig;
use tauri::window::Effect;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, Rect, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

use crate::commands::{AppState, Reply, msg};
use crate::control::{self, Request};
use crate::platform::SettingsPane;

const LABEL: &str = "panel";
const WIDTH: f64 = 400.0;
const HEIGHT: f64 = 520.0;
const MIN_HEIGHT: f64 = 200.0;
const MAX_HEIGHT: f64 = 720.0;
const GAP: f64 = 6.0;
const EDGE: f64 = 8.0;
const RADIUS: f64 = 14.0;

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::default())
        .title("Sextant")
        .inner_size(WIDTH, HEIGHT)
        .decorations(false)
        .transparent(true)
        .effects(WindowEffectsConfig {
            effects: vec![Effect::LiquidGlassRegular],
            state: None,
            radius: Some(RADIUS),
            color: None,
            interactive: false,
        })
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .resizable(false)
        .visible(false)
        .build()?;
    let hidden = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Focused(false) = event {
            let _ = hidden.hide();
        }
    });
    Ok(())
}

fn panel(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    app.get_webview_window(LABEL)
        .ok_or(tauri::Error::WindowNotFound)
}

/// Shows the panel under the tray icon at `icon`, or hides it when it is open.
pub fn toggle(app: &AppHandle, icon: Rect) -> tauri::Result<()> {
    if panel(app)?.is_visible()? {
        return panel(app)?.hide();
    }
    show(app, Some(icon))
}

/// Shows the panel under the tray icon at `icon`, or in the middle of the screen when macOS has
/// not placed the icon on a screen yet, as on the first launch.
pub fn show(app: &AppHandle, icon: Option<Rect>) -> tauri::Result<()> {
    let window = panel(app)?;
    match icon {
        Some(icon) => place_under(&window, icon)?,
        None => window.center()?,
    }
    window.show()?;
    window.set_focus()?;
    window.emit_to(LABEL, "panel-open", ())
}

fn place_under(window: &WebviewWindow, icon: Rect) -> tauri::Result<()> {
    let scale = window.scale_factor()?;
    let at = icon.position.to_physical::<f64>(scale);
    let size = icon.size.to_physical::<f64>(scale);
    let Some(monitor) = window.monitor_from_point(at.x, at.y)? else {
        return window.center();
    };
    let area = monitor.work_area();
    let width = WIDTH * scale;
    let left = f64::from(area.position.x) + EDGE * scale;
    let right = f64::from(area.position.x) + f64::from(area.size.width) - width - EDGE * scale;
    let x = (at.x + size.width / 2.0 - width / 2.0).clamp(left, right.max(left));
    window.set_position(PhysicalPosition::new(x, at.y + size.height + GAP * scale))
}

/// What the page may ask for: the panel view and setup. Settings and deletion stay in the CLI.
#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Ask {
    Status,
    Week {
        at: i64,
    },
    Routines,
    /// Runs the chosen agent on the week. Takes minutes; `Routines` shows it as running.
    FindRoutines,
    Pause,
    Resume,
    StopAudio,
    Permissions,
    /// Opens the System Settings pane where you drag Sextant into the list.
    Allow {
        pane: SettingsPane,
    },
    /// Shows the macOS prompt that allows reading the browser tab address.
    Automation,
    FinishSetup,
    /// Opens Sextant again, so a new Screen Recording grant applies.
    Relaunch,
    /// Sets the panel height to the page's, so the glass ends where the content does.
    Fit {
        height: f64,
    },
    Quit,
}

#[tauri::command]
pub async fn ask(app: AppHandle, ask: Ask) -> Reply<Value> {
    let request = match ask {
        Ask::Status => Request::Status,
        Ask::Week { at } => Request::Week { at },
        Ask::Routines => Request::Agent,
        Ask::FindRoutines => Request::RunAgent,
        Ask::Pause => Request::Pause,
        Ask::Resume => Request::Resume,
        Ask::StopAudio => Request::Audio { on: false },
        Ask::Permissions => Request::Permissions,
        Ask::Allow { pane } => Request::Allow { pane },
        Ask::Automation => Request::AskAutomation,
        Ask::FinishSetup => Request::FinishSetup,
        Ask::Relaunch => control::restart(&app),
        Ask::Fit { height } => {
            let size = LogicalSize::new(WIDTH, height.clamp(MIN_HEIGHT, MAX_HEIGHT));
            panel(&app)
                .and_then(|window| window.set_size(size))
                .map_err(msg)?;
            return Ok(Value::Null);
        }
        Ask::Quit => {
            app.exit(0);
            return Ok(Value::Null);
        }
    };
    tauri::async_runtime::spawn_blocking(move || control::handle(&app.state::<AppState>(), request))
        .await
        .map_err(msg)?
}
