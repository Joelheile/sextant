//! Captures one window with ScreenCaptureKit (macOS 14 and later). The window is the focused
//! window of the app, never the whole screen, so other windows on the display are not in the image.
//! Needs Screen Recording permission. The image goes straight to OCR and is never written out.

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use block2::RcBlock;
use objc2::AnyThread;
use objc2::rc::{Retained, autoreleasepool};
use objc2_core_foundation::{CFDictionary, CFNumber, CFNumberType, CFRetained, CFString};
use objc2_core_graphics::{
    CGImage, CGWindowListCopyWindowInfo, CGWindowListOption, kCGNullWindowID, kCGWindowNumber,
};
use objc2_foundation::NSError;
use objc2_screen_capture_kit::{
    SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration, SCWindow,
};

use sextant_core::capture::focused_window;

use super::vision::Frame;

const ANSWER_WAIT: Duration = Duration::from_secs(3);
/// Smaller windows are palettes and tooltips, not what the user is reading.
const MIN_SIDE: f64 = 120.0;

/// The focused window of `pid`. `title` is its Accessibility title, empty when unknown. Fails when
/// the window cannot be told apart from the app's other windows.
pub fn capture_window(pid: i32, title: &str) -> Result<Frame> {
    let (answer, answered) = mpsc::channel::<Result<Frame, String>>();
    let title = title.to_string();
    let handler = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            // SAFETY: SCK passes either a live object or null, valid for the duration of the call.
            let content = unsafe { Retained::retain(content) };
            let Some(content) = content else {
                let _ = answer.send(Err(describe(error)));
                return;
            };
            match pick(&content, pid, &title) {
                Some(window) => shoot(&window, answer.clone()),
                None => {
                    let _ = answer.send(Err("focused window not identified".into()));
                }
            }
        },
    );
    autoreleasepool(|_| {
        // SAFETY: the block stays alive until `answered` receives or times out.
        unsafe {
            SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
                true, true, &handler,
            );
        }
    });
    answered
        .recv_timeout(ANSWER_WAIT)
        .context("ScreenCaptureKit did not answer")?
        .map_err(|why| anyhow!(why))
}

fn describe(error: *mut NSError) -> String {
    // SAFETY: a live NSError or null.
    match unsafe { Retained::retain(error) } {
        Some(error) => format!("ScreenCaptureKit error {}", error.code()),
        None => "ScreenCaptureKit returned nothing".into(),
    }
}

/// The window numbers of everything on screen, front to back.
fn front_to_back() -> Vec<u32> {
    let options =
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
    let Some(list) = CGWindowListCopyWindowInfo(options, kCGNullWindowID) else {
        return Vec::new();
    };
    (0..list.count())
        .filter_map(|index| {
            // SAFETY: the list holds dictionaries, and each `number` is a CFNumber or null.
            unsafe {
                let entry = (list.value_at_index(index) as *const CFDictionary).as_ref()?;
                let key = kCGWindowNumber as *const CFString as *const c_void;
                let number = (entry.value(key) as *const CFNumber).as_ref()?;
                let mut id = 0_i32;
                number
                    .value(CFNumberType::SInt32Type, (&mut id as *mut i32).cast())
                    .then_some(id as u32)
            }
        })
        .collect()
}

/// The focused window of `pid`, or nothing when it cannot be told from the app's other windows.
fn pick(content: &SCShareableContent, pid: i32, title: &str) -> Option<Retained<SCWindow>> {
    let order = front_to_back();
    // SAFETY: read-only getters on live SCK objects.
    let mut candidates: Vec<(usize, Retained<SCWindow>)> = unsafe {
        content
            .windows()
            .iter()
            .filter(|window| {
                let frame = window.frame();
                window.isOnScreen()
                    && window.windowLayer() == 0
                    && window
                        .owningApplication()
                        .is_some_and(|app| app.processID() == pid)
                    && frame.size.width >= MIN_SIDE
                    && frame.size.height >= MIN_SIDE
            })
            .filter_map(|window| {
                let place = order.iter().position(|id| *id == window.windowID())?;
                Some((place, window))
            })
            .collect()
    };
    candidates.sort_by_key(|(place, _)| *place);
    let titles: Vec<String> = candidates
        .iter()
        // SAFETY: read-only getter.
        .map(|(_, window)| {
            unsafe { window.title() }
                .map(|t| t.to_string())
                .unwrap_or_default()
        })
        .collect();
    let titles: Vec<&str> = titles.iter().map(String::as_str).collect();
    let index = focused_window(&titles, title)?;
    candidates.into_iter().nth(index).map(|(_, window)| window)
}

fn shoot(window: &SCWindow, answer: mpsc::Sender<Result<Frame, String>>) {
    // SAFETY: plain Objective-C object creation from a live window.
    let (filter, config) = unsafe {
        let filter =
            SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), window);
        let config = SCStreamConfiguration::new();
        let frame = window.frame();
        let scale = f64::from(filter.pointPixelScale());
        config.setWidth((frame.size.width * scale) as usize);
        config.setHeight((frame.size.height * scale) as usize);
        config.setShowsCursor(false);
        (filter, config)
    };
    let done = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
        let frame = NonNull::new(image).map(|image| {
            // SAFETY: SCK passes a live image, valid for the duration of the call; retaining
            // keeps it afterwards.
            Frame::new(unsafe { CFRetained::retain(image) })
        });
        let _ = answer.send(frame.ok_or_else(|| describe(error)));
    });
    // SAFETY: SCK copies the block before returning.
    unsafe {
        SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
            &filter,
            &config,
            Some(&done),
        );
    }
}

/// Fails fast with a readable message when the app has no permission.
pub fn require_access() -> Result<()> {
    if objc2_core_graphics::CGPreflightScreenCaptureAccess() {
        Ok(())
    } else {
        bail!("Screen Recording is not granted")
    }
}
