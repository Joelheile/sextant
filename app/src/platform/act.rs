//! Presses and text fields for `sextant_core::capture` actions (plan.md §4). A global monitor
//! notes only where the left mouse button went down; the capture thread then reads the element
//! there and the focused field from the Accessibility tree. Never keystrokes, and a secure field's
//! value is never read.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::ptr::NonNull;
use std::sync::{LazyLock, Mutex, Once};
use std::thread::Thread;

use block2::RcBlock;
use crossbeam_queue::ArrayQueue;
use dispatch2::DispatchQueue;
use objc2_app_kit::{NSEvent, NSEventMask};
use objc2_application_services::{AXError, AXUIElement};
use objc2_core_foundation::{CFArray, CFNumber, CFRetained, CFString, CFType};
use objc2_core_graphics::{CGDisplayBounds, CGMainDisplayID};
use sextant_core::capture::{Element, Field, MAX_DEPTH, MAX_VALUE_CHARS, editable, pressable};

use super::tree::{self, CALL_TIMEOUT};

/// Presses waiting for the capture thread. Older ones drop when more arrive between two polls.
const PENDING: usize = 8;

static CLICKS: LazyLock<ArrayQueue<[f64; 2]>> = LazyLock::new(|| ArrayQueue::new(PENDING));
static WAKER: Mutex<Option<Thread>> = Mutex::new(None);
static MONITOR: Once = Once::new();

const ELEMENT: [&str; 6] = [
    "AXRole",
    "AXSubrole",
    "AXTitle",
    "AXDescription",
    "AXPlaceholderValue",
    "AXParent",
];
const FIELD: [&str; 6] = [
    "AXRole",
    "AXSubrole",
    "AXTitle",
    "AXDescription",
    "AXPlaceholderValue",
    "AXNumberOfCharacters",
];

/// Installs the mouse monitor once, on the main queue, and wakes `waker` on every press.
pub fn watch_clicks(waker: Thread) {
    if let Ok(mut slot) = WAKER.lock() {
        *slot = Some(waker);
    }
    MONITOR.call_once(|| {
        DispatchQueue::main().exec_async(|| {
            let block = RcBlock::new(|_event: NonNull<NSEvent>| {
                let at = NSEvent::mouseLocation();
                // AppKit counts from the bottom left of the main display, Accessibility from the
                // top left.
                let height = CGDisplayBounds(CGMainDisplayID()).size.height;
                let _ = CLICKS.force_push([at.x, height - at.y]);
                if let Ok(waker) = WAKER.lock()
                    && let Some(thread) = waker.as_ref()
                {
                    thread.unpark();
                }
            });
            // The monitor lives as long as the app.
            std::mem::forget(NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
                NSEventMask::LeftMouseDown,
                &block,
            ));
        });
    });
}

pub fn clicks() -> Vec<[f64; 2]> {
    std::iter::from_fn(|| CLICKS.pop()).collect()
}

/// Identifies an element: equal elements hash the same.
pub fn id(element: &AXUIElement) -> u64 {
    let base: &CFType = element;
    let mut hasher = DefaultHasher::new();
    base.hash(&mut hasher);
    hasher.finish()
}

fn text(values: &CFArray<CFType>, index: usize) -> String {
    values
        .get(index)
        .map(|value| tree::as_text(&value))
        .unwrap_or_default()
}

fn element(values: &CFArray<CFType>) -> Element {
    Element {
        role: text(values, 0),
        subrole: text(values, 1),
        title: text(values, 2),
        description: text(values, 3),
        placeholder: text(values, 4),
        value: String::new(),
    }
}

/// One attribute of `element`, bounded by [`CALL_TIMEOUT`].
fn attribute(element: &AXUIElement, name: &'static str) -> Option<CFRetained<CFType>> {
    let key = CFString::from_static_str(name);
    let mut value: *const CFType = std::ptr::null();
    // SAFETY: `value` is a valid out pointer for one object; the timeout bounds the call.
    let error = unsafe {
        element.set_messaging_timeout(CALL_TIMEOUT);
        element.copy_attribute_value(&key, NonNull::from(&mut value))
    };
    if error != AXError::Success {
        return None;
    }
    // SAFETY: on success the call returns an owned (+1) object.
    NonNull::new(value.cast_mut()).map(|value| unsafe { CFRetained::from_raw(value) })
}

/// The text value of `element`, `None` when it has none or it is longer than a field holds.
pub fn value(element: &AXUIElement) -> Option<String> {
    let text = attribute(element, "AXValue")?
        .downcast::<CFString>()
        .ok()?
        .to_string();
    (text.chars().count() <= MAX_VALUE_CHARS).then_some(text)
}

/// Identifies the focused window of app `pid`, 0 when it has none.
pub fn window(pid: i32) -> u64 {
    // SAFETY: an element for the app, owned here.
    let application = unsafe { AXUIElement::new_application(pid) };
    attribute(&application, "AXFocusedWindow")
        .and_then(|window| window.downcast::<AXUIElement>().ok())
        .map(|window| id(&window))
        .unwrap_or(0)
}

/// The element of app `pid` at `point` and its parents, up to the first pressable control.
/// Empty when the point belongs to another app.
pub fn chain(pid: i32, point: [f64; 2], names: &CFArray<CFString>) -> Vec<Element> {
    let mut hit: *const AXUIElement = std::ptr::null();
    // SAFETY: a hit test on the system-wide element, bounded by the timeout.
    let error = unsafe {
        let system = AXUIElement::new_system_wide();
        system.set_messaging_timeout(CALL_TIMEOUT);
        system.copy_element_at_position(point[0] as f32, point[1] as f32, NonNull::from(&mut hit))
    };
    if error != AXError::Success {
        return Vec::new();
    }
    // SAFETY: on success the call returns an owned (+1) element.
    let Some(mut current) =
        NonNull::new(hit.cast_mut()).map(|hit| unsafe { CFRetained::from_raw(hit) })
    else {
        return Vec::new();
    };
    let mut owner = 0;
    // SAFETY: `owner` is a valid out pointer.
    if unsafe { current.pid(NonNull::from(&mut owner)) } != AXError::Success || owner != pid {
        return Vec::new();
    }
    let mut out = Vec::new();
    while out.len() < MAX_DEPTH {
        let Some(values) = tree::values(&current, names) else {
            break;
        };
        let mut read = element(&values);
        if read.role == "AXStaticText" {
            read.value = value(&current).unwrap_or_default();
        }
        let done = pressable(&read);
        out.push(read);
        let parent = values
            .get(5)
            .and_then(|value| value.downcast::<AXUIElement>().ok());
        match parent {
            Some(parent) if !done => current = parent,
            _ => break,
        }
    }
    out
}

/// The focused element of app `pid`, with its value only for a text field that may be read.
pub fn focused(pid: i32, names: &CFArray<CFString>) -> Option<(CFRetained<AXUIElement>, Field)> {
    // SAFETY: an element for the app, owned here.
    let application = unsafe { AXUIElement::new_application(pid) };
    let focused = attribute(&application, "AXFocusedUIElement")?
        .downcast::<AXUIElement>()
        .ok()?;
    let values = tree::values(&focused, names)?;
    let read = element(&values);
    let short = values
        .get(5)
        .and_then(|count| count.downcast_ref::<CFNumber>().and_then(|n| n.as_i64()))
        .is_none_or(|count| count <= MAX_VALUE_CHARS as i64);
    let value = (editable(&read) && short)
        .then(|| value(&focused))
        .flatten();
    let field = Field {
        id: id(&focused),
        element: read,
        value,
    };
    Some((focused, field))
}

pub fn element_names() -> CFRetained<CFArray<CFString>> {
    tree::names(&ELEMENT)
}

pub fn field_names() -> CFRetained<CFArray<CFString>> {
    tree::names(&FIELD)
}
