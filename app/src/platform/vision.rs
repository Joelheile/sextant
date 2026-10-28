//! Vision text recognition (plan.md §4) on a captured window, plus the grayscale thumbnail the
//! diff gate compares. Pixels stay in [`Frame`]: it prints only its size and nothing here logs.

use std::fmt;
use std::thread;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use objc2::AnyThread;
use objc2::rc::{Retained, autoreleasepool};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo,
};
use objc2_foundation::{NSArray, NSDictionary, NSString};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRecognizedTextObservation, VNRequest,
    VNRequestTextRecognitionLevel,
};
use sextant_core::capture::{RawLine, Thumb};

use super::render::{KINDS, LANGUAGES, SIZES, render};
use crate::capture::{Ocr, percentile};

/// Which Vision model runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Fast,
    Accurate,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Accurate => "accurate",
        }
    }
}

/// A captured window. The image is immutable, so it can move between threads.
pub struct Frame(CFRetained<CGImage>);

// SAFETY: a CGImage is immutable after creation and Core Graphics documents it as thread safe.
unsafe impl Send for Frame {}

impl Frame {
    pub fn new(image: CFRetained<CGImage>) -> Self {
        Self(image)
    }

    pub fn size(&self) -> (usize, usize) {
        (
            CGImage::width(Some(&self.0)),
            CGImage::height(Some(&self.0)),
        )
    }

    /// The grayscale thumbnail the diff gate compares.
    pub fn thumb(&self) -> Result<Thumb> {
        let (width, height) = self.size();
        let space = CGColorSpace::new_device_gray().context("no gray color space")?;
        let mut pixels = vec![0u8; width * height];
        // SAFETY: the buffer is width * height bytes, one byte per gray pixel, and outlives the
        // context, which is dropped before the buffer is read.
        let context = unsafe {
            CGBitmapContextCreate(
                pixels.as_mut_ptr().cast(),
                width,
                height,
                8,
                width,
                Some(&space),
                CGImageAlphaInfo::None.0,
            )
        }
        .context("no bitmap context")?;
        CGContext::draw_image(
            Some(&context),
            CGRect::new(
                CGPoint::new(0.0, 0.0),
                CGSize::new(width as f64, height as f64),
            ),
            Some(&self.0),
        );
        drop(context);
        Thumb::from_gray(width, height, &pixels).context("empty image")
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (width, height) = self.size();
        write!(f, "Frame({width}x{height})")
    }
}

/// The level capture uses. `.accurate` read text better but took 320 ms p50 and about 700 ms p95
/// per window on the fixtures, against a budget of 250 ms; `.fast` took under 60 ms p95.
pub const CHOSEN: Level = Level::Fast;

pub struct Recognizer {
    level: Level,
}

impl Recognizer {
    pub fn new(level: Level) -> Self {
        Self { level }
    }
}

impl Ocr<Frame> for Recognizer {
    fn recognize(&mut self, frame: &Frame) -> Result<Vec<RawLine>> {
        autoreleasepool(|_| recognize(frame, self.level))
    }
}

fn recognize(frame: &Frame, level: Level) -> Result<Vec<RawLine>> {
    let request = VNRecognizeTextRequest::new();
    request.setRecognitionLevel(match level {
        Level::Fast => VNRequestTextRecognitionLevel::Fast,
        Level::Accurate => VNRequestTextRecognitionLevel::Accurate,
    });
    request.setUsesLanguageCorrection(level == Level::Accurate);
    let languages =
        NSArray::from_retained_slice(&[NSString::from_str("en-US"), NSString::from_str("de-DE")]);
    request.setRecognitionLanguages(&languages);
    // SAFETY: the handler only reads the image for the duration of `performRequests`.
    let performed = unsafe {
        let handler = VNImageRequestHandler::initWithCGImage_options(
            VNImageRequestHandler::alloc(),
            &frame.0,
            &NSDictionary::new(),
        );
        let requests = NSArray::from_slice(&[AsRef::<VNRequest>::as_ref(&*request)]);
        handler.performRequests_error(&requests)
    };
    if let Err(err) = performed {
        bail!("Vision failed: {}", err.code());
    }
    let observations: Retained<NSArray<VNRecognizedTextObservation>> = match request.results() {
        Some(results) => results,
        None => return Ok(Vec::new()),
    };
    let mut lines = Vec::with_capacity(observations.len());
    for observation in observations.iter() {
        // SAFETY: read-only getters on a live observation.
        unsafe {
            let candidates = observation.topCandidates(1);
            let Some(best) = candidates.firstObject() else {
                continue;
            };
            let bounds = observation.boundingBox();
            lines.push(RawLine {
                text: best.string().to_string(),
                confidence: best.confidence(),
                x: bounds.origin.x as f32,
                // Vision measures from the bottom left.
                y: (1.0 - bounds.origin.y - bounds.size.height) as f32,
                width: bounds.size.width as f32,
                height: bounds.size.height as f32,
            });
        }
    }
    Ok(lines)
}

/// OCR time per window for one level and size, over every fixture.
#[derive(Debug, Clone)]
pub struct Timing {
    pub width: usize,
    pub height: usize,
    pub level: &'static str,
    pub p50_ms: f64,
    pub p95_ms: f64,
    /// Expected fixture text that the OCR did not find.
    pub missing: Vec<String>,
}

const PASSES: usize = 4;

fn squash(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn measure(level: Level, width: usize, height: usize) -> Result<Timing> {
    let mut ocr = Recognizer::new(level);
    let mut times = Vec::new();
    let mut missing = Vec::new();
    for kind in KINDS {
        for language in LANGUAGES {
            let fixture = render(kind, language, width, height)?;
            // The first call loads the model.
            ocr.recognize(&fixture.frame)?;
            for pass in 0..PASSES {
                let started = Instant::now();
                let raw = ocr.recognize(&fixture.frame)?;
                times.push(started.elapsed().as_secs_f64() * 1000.0);
                if pass == 0 {
                    let text = squash(
                        &raw.iter()
                            .map(|line| line.text.as_str())
                            .collect::<Vec<_>>()
                            .join(" "),
                    );
                    missing.extend(
                        fixture
                            .expected
                            .iter()
                            .filter(|expected| !text.contains(expected.as_str()))
                            .map(|expected| format!("{}: {expected}", fixture.name)),
                    );
                }
            }
        }
    }
    Ok(Timing {
        width,
        height,
        level: level.as_str(),
        p50_ms: percentile(&times, 50.0),
        p95_ms: percentile(&times, 95.0),
        missing,
    })
}

/// Renders the fixtures and times both levels at every size on a background thread, the way
/// capture runs OCR.
pub fn fixture_timings() -> Result<Vec<Timing>> {
    thread::Builder::new()
        .name("sextant-ocr-bench".into())
        .spawn(|| {
            let mut timings = Vec::new();
            for (width, height) in SIZES {
                for level in [Level::Fast, Level::Accurate] {
                    timings.push(measure(level, width, height)?);
                }
            }
            Ok(timings)
        })
        .context("starting the OCR thread")?
        .join()
        .map_err(|_| anyhow::anyhow!("the OCR thread panicked"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUDGET_MS: f64 = 250.0;

    #[test]
    fn ocr_time_per_window_meets_the_budget_for_the_chosen_level() {
        for timing in fixture_timings().expect("measures") {
            eprintln!(
                "ocr {}x{} {:<8} p50 {:7.1} ms  p95 {:7.1} ms  missing text: {}",
                timing.width,
                timing.height,
                timing.level,
                timing.p50_ms,
                timing.p95_ms,
                timing.missing.len()
            );
            if timing.level == CHOSEN.as_str() {
                assert!(
                    timing.p95_ms < BUDGET_MS,
                    "{} OCR p95 {:.0} ms at {}x{} is over {BUDGET_MS} ms",
                    timing.level,
                    timing.p95_ms,
                    timing.width,
                    timing.height
                );
                assert!(timing.missing.is_empty(), "{:?}", timing.missing);
            }
        }
    }
}
