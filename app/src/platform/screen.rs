//! Captures one window with ScreenCaptureKit (macOS 14 and later). The window is the focused
//! window of the app, never the whole screen, so other windows on the display are not in the image.
//! Needs Screen Recording permission. The image goes straight to OCR and is never written out.

use std::ptr::NonNull;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use block2::RcBlock;
use objc2::AnyThread;
use objc2::rc::{Retained, autoreleasepool};
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::CGImage;
use objc2_foundation::NSError;
use objc2_screen_capture_kit::{
    SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration, SCWindow,
};

use super::vision::Frame;

const ANSWER_WAIT: Duration = Duration::from_secs(3);
/// Smaller windows are palettes and tooltips, not what the user is reading.
const MIN_SIDE: f64 = 120.0;

/// The window of `pid` that is on screen. `title` picks between several when the Accessibility
/// title of the focused window is known.
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
                    let _ = answer.send(Err("no window on screen".into()));
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

fn pick(content: &SCShareableContent, pid: i32, title: &str) -> Option<Retained<SCWindow>> {
    // SAFETY: read-only getters on live SCK objects.
    let candidates: Vec<Retained<SCWindow>> = unsafe {
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
            .collect()
    };
    let titled = candidates.iter().find(|window| {
        // SAFETY: read-only getter.
        !title.is_empty() && unsafe { window.title() }.is_some_and(|t| t.to_string() == title)
    });
    titled.or(candidates.first()).cloned()
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
