//! Reads the Accessibility tree of the focused window for `sextant_core::capture::window_lines`
//! (plan.md §4). One batched call per element, a time budget per window. Text only, no pixels.

use std::ptr::NonNull;
use std::time::{Duration, Instant};

use objc2_application_services::{
    AXCopyMultipleAttributeOptions, AXError, AXUIElement, AXValue, AXValueType,
};
use objc2_core_foundation::{
    CFArray, CFBoolean, CFNumber, CFRetained, CFString, CFType, CGPoint, CGSize,
};
use sextant_core::OcrLine;
use sextant_core::capture::{AxRead, AxSource, window_lines};

/// Each call gives up after this long, so a hung app cannot stall capture.
pub const CALL_TIMEOUT: f32 = 0.1;
/// The whole walk of one window.
const WALK_BUDGET: Duration = Duration::from_millis(120);

const ATTRIBUTES: [&str; 8] = [
    "AXRole",
    "AXSubrole",
    "AXValue",
    "AXTitle",
    "AXDescription",
    "AXPosition",
    "AXSize",
    "AXChildren",
];

struct Walker {
    deadline: Instant,
    names: CFRetained<CFArray<CFString>>,
}

impl AxSource for Walker {
    type Element = CFRetained<AXUIElement>;

    fn read(&mut self, element: &Self::Element) -> Option<AxRead<Self::Element>> {
        let values = values(element, &self.names)?;
        let get = |i: usize| values.get(i);
        let text = |i: usize| get(i).map(|v| as_text(&v)).unwrap_or_default();
        let position = get(5).and_then(|v| point(&v));
        let size = get(6).and_then(|v| size(&v));
        Some(AxRead {
            role: text(0),
            subrole: text(1),
            value: text(2),
            title: text(3),
            description: text(4),
            frame: position
                .zip(size)
                .map(|(p, s)| [p.x, p.y, s.width, s.height]),
            children: get(7).map(|v| children(&v)).unwrap_or_default(),
        })
    }

    fn expired(&mut self) -> bool {
        Instant::now() >= self.deadline
    }
}

/// `names` of `element` in one call, bounded by [`CALL_TIMEOUT`]. A missing attribute comes back
/// as an error value, which reads as empty.
pub fn values(
    element: &AXUIElement,
    names: &CFArray<CFString>,
) -> Option<CFRetained<CFArray<CFType>>> {
    let mut values: *const CFArray = std::ptr::null();
    // SAFETY: `names` is an array of CFStrings and `values` a valid out pointer. The timeout
    // bounds the call.
    let error = unsafe {
        element.set_messaging_timeout(CALL_TIMEOUT);
        element.copy_multiple_attribute_values(
            names.as_opaque(),
            AXCopyMultipleAttributeOptions(0),
            NonNull::from(&mut values),
        )
    };
    if error != AXError::Success {
        return None;
    }
    // SAFETY: on success the call returns an owned (+1) array of CF objects.
    Some(unsafe {
        CFRetained::cast_unchecked(CFRetained::from_raw(NonNull::new(values.cast_mut())?))
    })
}

/// The attribute names as one CF array, for [`values`].
pub fn names(attributes: &[&'static str]) -> CFRetained<CFArray<CFString>> {
    let names: Vec<CFRetained<CFString>> = attributes
        .iter()
        .map(|name| CFString::from_static_str(name))
        .collect();
    CFArray::from_retained_objects(&names)
}

/// A string, or a number as text: checkboxes report 0, 1 or 2.
pub fn as_text(value: &CFType) -> String {
    if let Some(text) = value.downcast_ref::<CFString>() {
        return text.to_string();
    }
    if let Some(number) = value.downcast_ref::<CFNumber>() {
        return number.as_i64().map(|n| n.to_string()).unwrap_or_default();
    }
    if let Some(flag) = value.downcast_ref::<CFBoolean>() {
        return if flag.as_bool() { "1" } else { "0" }.into();
    }
    String::new()
}

fn ax_value<T: Default>(value: &CFType, kind: AXValueType) -> Option<T> {
    let value = value.downcast_ref::<AXValue>()?;
    let mut out = T::default();
    // SAFETY: `out` has the layout `kind` names, and the call writes exactly one of it.
    let ok = unsafe { value.value(kind, NonNull::from(&mut out).cast()) };
    ok.then_some(out)
}

fn point(value: &CFType) -> Option<CGPoint> {
    ax_value(value, AXValueType::CGPoint)
}

fn size(value: &CFType) -> Option<CGSize> {
    ax_value(value, AXValueType::CGSize)
}

fn children(value: &CFType) -> Vec<CFRetained<AXUIElement>> {
    let Some(array) = value.downcast_ref::<CFArray>() else {
        return Vec::new();
    };
    // SAFETY: AXChildren is an array of AXUIElements.
    let array = unsafe { &*(array as *const CFArray).cast::<CFArray<AXUIElement>>() };
    array.to_vec()
}

/// Electron and Chromium apps only build their tree for assistive apps that ask for it.
/// Harmless for every other app, which reports the attribute as unsupported.
pub fn enable_tree(pid: i32) {
    // SAFETY: setting one attribute on an element owned here, bounded by the timeout.
    unsafe {
        let application = AXUIElement::new_application(pid);
        application.set_messaging_timeout(CALL_TIMEOUT);
        let key = CFString::from_static_str("AXManualAccessibility");
        let _ = application.set_attribute_value(&key, CFBoolean::new(true));
    }
}

/// The visible text of `window`, empty when the app does not answer in time.
pub fn lines(window: CFRetained<AXUIElement>) -> Vec<OcrLine> {
    let mut walker = Walker {
        deadline: Instant::now() + WALK_BUDGET,
        names: names(&ATTRIBUTES),
    };
    window_lines(&mut walker, window)
}
