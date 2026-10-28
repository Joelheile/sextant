//! The live sensors on macOS (plan.md §4): frontmost app, window title and document, secure
//! input, idle time, lock and sleep state, clipboard changes, browser tab and the window capture.
//! Timing out or failing never opens a gate: callers treat a missing answer as "do not capture".

use std::ffi::c_void;
use std::ptr::NonNull;

use anyhow::Result;
use objc2_app_kit::{NSPasteboard, NSWorkspace};
use objc2_application_services::{AXError, AXUIElement};
use objc2_core_foundation::{CFRetained, CFString, CFType};
use objc2_core_graphics::{
    CGEventSource, CGEventSourceStateID, CGEventType, CGMainDisplayID,
    CGSessionCopyCurrentDictionary,
};
use sextant_core::capture::Browser;

use super::vision::Frame;
use super::{automation, screen};
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
}

impl Mac {
    pub fn new() -> Self {
        Self {
            own_pid: std::process::id() as i32,
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

fn focused_window_text(pid: i32) -> WindowText {
    // SAFETY: Accessibility calls on elements owned here, each bounded by the timeout.
    unsafe {
        let application = AXUIElement::new_application(pid);
        application.set_messaging_timeout(AX_TIMEOUT);
        attribute(&application, "AXFocusedWindow")
            .and_then(|window| window.downcast::<AXUIElement>().ok())
            .map(|window| {
                window.set_messaging_timeout(AX_TIMEOUT);
                WindowText {
                    title: text_attribute(&window, "AXTitle"),
                    doc: text_attribute(&window, "AXDocument"),
                }
            })
            .unwrap_or_default()
    }
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

    fn clipboard_count(&mut self) -> i64 {
        NSPasteboard::generalPasteboard().changeCount() as i64
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

    fn secure_input(&mut self) -> bool {
        // SAFETY: plain C call without arguments.
        unsafe { IsSecureEventInputEnabled() != 0 }
    }

    fn window_text(&mut self, app: &App) -> WindowText {
        focused_window_text(app.pid)
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
