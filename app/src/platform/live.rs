//! The live sensors on macOS (plan.md §4): frontmost app, window title and document, secure
//! input, idle time, lock and sleep state, clipboard changes, browser tab and the window capture.
//! Timing out or failing never opens a gate: callers treat a missing answer as "do not capture".

use std::collections::HashSet;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::thread::Thread;

use anyhow::Result;
use objc2_app_kit::NSWorkspace;
use objc2_application_services::{AXError, AXUIElement};
use objc2_core_foundation::{CFRetained, CFString, CFType};
use objc2_core_graphics::{
    CGEventSource, CGEventSourceStateID, CGEventType, CGMainDisplayID,
    CGSessionCopyCurrentDictionary,
};
use objc2_foundation::NSProcessInfo;
use sextant_core::OcrLine;
use sextant_core::capture::{Browser, Element, Field};

use super::vision::Frame;
use super::{act, automation, screen, tree};
use crate::capture::{App, Presence, Sensors, Shot, Tab, WindowText};

/// Accessibility calls give up after this many seconds, so a hung app cannot stall capture.
const AX_TIMEOUT: f32 = 0.5;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn IsSecureEventInputEnabled() -> u8;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGDisplayIsAsleep(display: u32) -> u32;
}

pub struct Mac {
    own_pid: i32,
    /// Apps already asked to build their Accessibility tree.
    asked: HashSet<i32>,
    /// The last focused text fields, newest last, so one that just lost focus can be read again.
    recent: Vec<(u64, Focused)>,
}

/// An element kept between polls on the capture thread.
struct Focused(CFRetained<AXUIElement>);

// SAFETY: Accessibility elements are CF objects, which may be used from any thread. Only the
// capture thread touches these.
unsafe impl Send for Focused {}

impl Mac {
    pub fn new() -> Self {
        Self {
            own_pid: std::process::id() as i32,
            asked: HashSet::new(),
            recent: Vec::new(),
        }
    }

    fn enable_tree(&mut self, pid: i32) {
        if self.asked.insert(pid) {
            tree::enable_tree(pid);
        }
    }
}

fn attribute(element: &AXUIElement, name: &'static str) -> Option<CFRetained<CFType>> {
    let key = CFString::from_static_str(name);
    let mut value: *const CFType = std::ptr::null();
    // SAFETY: `value` is a valid out pointer for one object.
    let error = unsafe { element.copy_attribute_value(&key, NonNull::from(&mut value)) };
    if error != AXError::Success {
        return None;
    }
    // SAFETY: on success the call returns an owned (+1) object.
    NonNull::new(value.cast_mut()).map(|value| unsafe { CFRetained::from_raw(value) })
}

fn text_attribute(element: &AXUIElement, name: &'static str) -> String {
    attribute(element, name)
        .and_then(|value| value.downcast::<CFString>().ok())
        .map(|text| text.to_string())
        .unwrap_or_default()
}

fn focused_window(pid: i32) -> Option<CFRetained<AXUIElement>> {
    // SAFETY: Accessibility calls on elements owned here, each bounded by the timeout.
    unsafe {
        let application = AXUIElement::new_application(pid);
        application.set_messaging_timeout(AX_TIMEOUT);
        let window = attribute(&application, "AXFocusedWindow")?
            .downcast::<AXUIElement>()
            .ok()?;
        window.set_messaging_timeout(AX_TIMEOUT);
        Some(window)
    }
}

fn focused_window_text(pid: i32) -> WindowText {
    focused_window(pid)
        .map(|window| WindowText {
            title: text_attribute(&window, "AXTitle"),
            doc: text_attribute(&window, "AXDocument"),
        })
        .unwrap_or_default()
}

impl Sensors for Mac {
    type Image = Frame;

    fn presence(&mut self) -> Presence {
        let idle_s = CGEventSource::seconds_since_last_event_type(
            CGEventSourceStateID::CombinedSessionState,
            CGEventType(u32::MAX),
        );
        let locked = match CGSessionCopyCurrentDictionary() {
            Some(session) => {
                let key = CFString::from_static_str("CGSSessionScreenIsLocked");
                // SAFETY: the key is a live CFString and the dictionary holds CF objects.
                unsafe {
                    !session
                        .value(&*key as *const CFString as *const c_void)
                        .is_null()
                }
            }
            // No session dictionary means no user session: the same as away.
            None => true,
        };
        // SAFETY: plain C call on the main display id.
        let asleep = unsafe { CGDisplayIsAsleep(CGMainDisplayID()) } != 0;
        Presence {
            idle_s,
            locked,
            asleep,
        }
    }

    fn focused_app(&mut self) -> Option<App> {
        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        let pid = app.processIdentifier();
        // Sextant's own windows are never captured.
        if pid == self.own_pid {
            return None;
        }
        Some(App {
            name: app.localizedName()?.to_string(),
            bundle_id: app
                .bundleIdentifier()
                .map(|id| id.to_string())
                .unwrap_or_default(),
            pid,
        })
    }

    fn low_power(&mut self) -> bool {
        NSProcessInfo::processInfo().isLowPowerModeEnabled()
    }

    fn secure_input(&mut self) -> bool {
        // SAFETY: plain C call without arguments.
        unsafe { IsSecureEventInputEnabled() != 0 }
    }

    fn window_text(&mut self, app: &App) -> WindowText {
        focused_window_text(app.pid)
    }

    fn window_lines(&mut self, app: &App) -> Vec<OcrLine> {
        self.enable_tree(app.pid);
        focused_window(app.pid).map(tree::lines).unwrap_or_default()
    }

    fn window_id(&mut self, app: &App) -> u64 {
        act::window(app.pid)
    }

    fn clicks(&mut self) -> Vec<[f64; 2]> {
        act::clicks()
    }

    fn wake_on_click(&mut self, thread: Thread) {
        act::watch_clicks(thread);
    }

    fn element_chain(&mut self, app: &App, point: [f64; 2]) -> Vec<Element> {
        self.enable_tree(app.pid);
        act::chain(app.pid, point, &act::element_names())
    }

    fn focused_field(&mut self, app: &App) -> Option<Field> {
        self.enable_tree(app.pid);
        let (element, field) = act::focused(app.pid, &act::field_names())?;
        if self.recent.last().is_none_or(|(id, _)| *id != field.id) {
            self.recent.push((field.id, Focused(element)));
            if self.recent.len() > 2 {
                self.recent.remove(0);
            }
        }
        Some(field)
    }

    fn reread(&mut self, id: u64) -> Option<String> {
        let (_, Focused(element)) = self.recent.iter().find(|(seen, _)| *seen == id)?;
        act::value(element)
    }

    fn browser_tab(&mut self, app: &App, browser: Browser) -> Result<Tab> {
        automation::tab(&app.bundle_id, browser)
    }

    fn capture(&mut self, app: &App) -> Result<Shot<Frame>> {
        screen::require_access()?;
        // Read on every capture, whatever the metadata setting: it only picks the window.
        let title = focused_window_text(app.pid).title;
        let image = screen::capture_window(app.pid, &title)?;
        let thumb = image.thumb()?;
        Ok(Shot { thumb, image })
    }
}
