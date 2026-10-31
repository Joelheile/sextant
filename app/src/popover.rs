//! The menu bar popover: a click on the tray icon opens it under the icon. It shows what Sextant
//! captures right now, your day and the settings. It hides when it loses
//! focus, like the rewind panel. Right-click keeps the native menu.

use serde::Serialize;
use sextant_core::{Insights, Live};
use tauri::{AppHandle, Emitter, PhysicalPosition, Rect, WebviewWindow};

use crate::clock;
use crate::commands::{Reply, blocking, capture_blockers, msg};
use crate::engine::{AudioState, CaptureState};
use crate::panel;
use crate::tray::{MenuAction, perform};

const LABEL: &str = "popover";
const WIDTH: f64 = 380.0;
const HEIGHT: f64 = 600.0;
const GAP: f64 = 6.0;
const EDGE: f64 = 8.0;
const RADIUS: f64 = 14.0;

/// Builds the hidden popover once at start-up, so a click shows it without a delay.
pub fn build(app: &AppHandle) -> tauri::Result<()> {
    panel::build(app, LABEL, "Sextant", (WIDTH, HEIGHT), RADIUS)
}

fn popover(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    panel::get(app, LABEL)
}

/// Shows the popover under the tray icon at `icon`, or hides it when it is open.
pub fn toggle(app: &AppHandle, icon: Rect) -> tauri::Result<()> {
    let window = popover(app)?;
    if window.is_visible()? {
        return window.hide();
    }
    let scale = window.scale_factor()?;
    let at = icon.position.to_physical::<f64>(scale);
    let size = icon.size.to_physical::<f64>(scale);
    let width = WIDTH * scale;
    let mut x = at.x + size.width / 2.0 - width / 2.0;
    if let Some(monitor) = window.monitor_from_point(at.x, at.y)? {
        let area = monitor.work_area();
        let left = f64::from(area.position.x) + EDGE * scale;
        let right = f64::from(area.position.x) + f64::from(area.size.width) - width - EDGE * scale;
        x = x.clamp(left, right.max(left));
    }
    window.set_position(PhysicalPosition::new(x, at.y + size.height + GAP * scale))?;
    window.show()?;
    window.set_focus()?;
    window.emit_to(LABEL, "popover-open", ())
}

pub fn hide(app: &AppHandle) -> tauri::Result<()> {
    popover(app)?.hide()
}

/// The live line at the top of the popover.
#[derive(Serialize)]
pub struct Status {
    /// `off`, `capturing` or `paused`.
    capture: &'static str,
    paused_until: Option<i64>,
    audio: AudioState,
    last_capture: Option<i64>,
    last_app: Option<String>,
    /// Permissions capture needs that macOS has not granted.
    blocked_by: Vec<&'static str>,
    live: Live,
    now: i64,
}

#[tauri::command]
pub async fn menu_status(app: AppHandle) -> Reply<Status> {
    blocking(app, move |state| {
        let engine = state.engine()?;
        let (capture, paused_until) = match engine.capture_state() {
            CaptureState::Off => ("off", None),
            CaptureState::Capturing => ("capturing", None),
            CaptureState::Paused(until) => ("paused", Some(until)),
        };
        let last = engine.read(|store| store.last_capture()).map_err(msg)?;
        let settings = crate::prefs::load(&state.dir).map_err(msg)?;
        Ok(Status {
            capture,
            paused_until,
            audio: engine.audio_state(),
            last_capture: last.as_ref().map(|(ts, _)| *ts),
            last_app: last.map(|(_, app)| app),
            blocked_by: capture_blockers(&state.dir),
            live: settings.live,
            now: clock::now(),
        })
    })
    .await
}

#[derive(Serialize)]
pub struct AppTimeView {
    app: String,
    minutes: f64,
}

#[derive(Serialize)]
pub struct InsightsView {
    active_minutes: f64,
    days: Vec<f64>,
    apps: Vec<AppTimeView>,
    switches: usize,
    switches_per_hour: Option<f64>,
    longest_focus_minutes: f64,
    lines: usize,
    call_minutes: f64,
    automatable_hours: f64,
    automatable_hours_last_week: f64,
    hours_back: f64,
}

#[tauri::command]
pub async fn insights(app: AppHandle) -> Reply<InsightsView> {
    blocking(app, move |state| {
        let (found, hours_back): (Insights, f64) = state
            .engine()?
            .read(|store| Ok((store.insights(clock::local())?, store.hours_back()?)))
            .map_err(msg)?;
        Ok(InsightsView {
            active_minutes: found.active_minutes,
            days: found.days,
            apps: found
                .apps
                .into_iter()
                .map(|a| AppTimeView {
                    app: a.app,
                    minutes: a.minutes,
                })
                .collect(),
            switches: found.switches,
            switches_per_hour: found.switches_per_hour,
            longest_focus_minutes: found.longest_focus_minutes,
            lines: found.lines,
            call_minutes: found.call_minutes,
            automatable_hours: found.automatable_hours,
            automatable_hours_last_week: found.automatable_hours_last_week,
            hours_back,
        })
    })
    .await
}

/// The popover's buttons: `pause` (or resume), `search`, `open`, `quit`, and `hide` for Escape.
/// Search and Open close the popover first. The popover has no Stop audio button.
#[tauri::command]
pub fn menu_action(app: AppHandle, action: MenuAction) -> Reply<()> {
    if matches!(action, MenuAction::Search | MenuAction::Open) {
        hide(&app).map_err(msg)?;
    }
    perform(&app, action).map_err(msg)
}
