//! The rewind search panel: a translucent bar at the bottom centre of the screen that
//! Cmd-Shift-Space opens. Answers stack upward like a chat, and past questions come from the
//! database. The panel hides when it loses focus. Sextant never captures its own windows.

use tauri::{AppHandle, Emitter, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::panel;

pub const SHORTCUT: &str = "CommandOrControl+Shift+Space";
const LABEL: &str = "rewind";
const WIDTH: f64 = 720.0;
const MIN_HEIGHT: f64 = 64.0;
/// The panel never covers more of the screen than this; older answers scroll inside it.
const MAX_SHARE: f64 = 0.7;
const BOTTOM_GAP: f64 = 24.0;
const RADIUS: f64 = 16.0;

/// Builds the hidden panel once at start-up, so the shortcut shows it without a delay.
pub fn build(app: &AppHandle) -> tauri::Result<()> {
    panel::build(app, LABEL, "Rewind search", (WIDTH, MIN_HEIGHT), RADIUS)
}

/// The shortcut: shows the panel on the screen under the pointer, or hides it when it is open.
pub fn toggle(app: &AppHandle) -> tauri::Result<()> {
    let window = panel::get(app, LABEL)?;
    if window.is_visible()? && window.is_focused()? {
        return window.hide();
    }
    show(app)
}

pub fn show(app: &AppHandle) -> tauri::Result<()> {
    let window = panel::get(app, LABEL)?;
    let height = window.inner_size()?.height as f64 / window.scale_factor()?;
    fit(&window, height)?;
    window.show()?;
    window.set_focus()?;
    crate::platform::language::prewarm();
    window.emit_to(LABEL, "rewind-open", ())
}

/// True while the panel shows, so background model work can wait and leave the model free.
pub fn is_open(app: &AppHandle) -> bool {
    panel::get(app, LABEL)
        .and_then(|window| window.is_visible())
        .unwrap_or(false)
}

/// Sizes the panel to its content, `height` logical pixels, keeping it at the bottom centre of
/// the screen under the pointer.
pub fn fit(window: &WebviewWindow, height: f64) -> tauri::Result<()> {
    let pointer = window.cursor_position()?;
    let Some(monitor) = window
        .monitor_from_point(pointer.x, pointer.y)?
        .or(window.primary_monitor()?)
    else {
        return Ok(());
    };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let (area_width, area_height) = (f64::from(area.size.width), f64::from(area.size.height));
    let width = (WIDTH * scale).min(area_width);
    let height = (height * scale).clamp(MIN_HEIGHT * scale, area_height * MAX_SHARE);
    window.set_size(PhysicalSize::new(width, height))?;
    window.set_position(PhysicalPosition::new(
        f64::from(area.position.x) + (area_width - width) / 2.0,
        f64::from(area.position.y) + area_height - height - BOTTOM_GAP * scale,
    ))
}

pub fn fit_panel(app: &AppHandle, height: f64) -> tauri::Result<()> {
    fit(&panel::get(app, LABEL)?, height)
}

pub fn hide(app: &AppHandle) -> tauri::Result<()> {
    panel::get(app, LABEL)?.hide()
}
