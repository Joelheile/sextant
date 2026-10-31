//! The hidden, translucent windows the popover and the rewind bar are: no frame, above other
//! windows on every desktop, and hidden again when they lose focus.

use tauri::utils::config::WindowEffectsConfig;
use tauri::window::Effect;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

/// Builds the window hidden, so showing it later has no delay.
pub fn build(
    app: &AppHandle,
    label: &str,
    title: &str,
    (width, height): (f64, f64),
    radius: f64,
) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::default())
        .title(title)
        .inner_size(width, height)
        .decorations(false)
        .transparent(true)
        .effects(WindowEffectsConfig {
            effects: vec![Effect::LiquidGlassRegular],
            state: None,
            radius: Some(radius),
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

pub fn get(app: &AppHandle, label: &str) -> tauri::Result<WebviewWindow> {
    app.get_webview_window(label)
        .ok_or(tauri::Error::WindowNotFound)
}
