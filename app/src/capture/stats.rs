//! Counts and timings of the live pipeline. Numbers only, never text (plan.md §10).

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use sextant_core::capture::Blocked;

/// Timings kept for percentiles. Older ones fall out, so a long session stays small.
const KEPT: usize = 4096;

const BLOCKED: [Blocked; 7] = [
    Blocked::Paused,
    Blocked::SourceOff,
    Blocked::ExcludedApp,
    Blocked::OpaqueBrowser,
    Blocked::PrivateWindow,
    Blocked::TabUnknown,
    Blocked::SecureInput,
];

#[derive(Default)]
pub struct Stats {
    /// Triggers that reached the gates.
    pub looks: AtomicU64,
    /// Frames the OS handed over.
    pub captured: AtomicU64,
    /// Frames the diff gate skipped.
    pub skipped: AtomicU64,
    /// Frames that went through OCR.
    pub ocr: AtomicU64,
    /// Frames read from the Accessibility tree, without a screenshot.
    pub tree: AtomicU64,
    /// Frames pushed out of the queue by newer ones.
    pub dropped: AtomicU64,
    pub capture_errors: AtomicU64,
    pub ocr_errors: AtomicU64,
    pub copies: AtomicU64,
    pub presses: AtomicU64,
    pub edits: AtomicU64,
    blocked: [AtomicU64; BLOCKED.len()],
    ocr_ms: Mutex<VecDeque<f64>>,
}

impl Stats {
    pub fn add(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get(counter: &AtomicU64) -> u64 {
        counter.load(Ordering::Relaxed)
    }

    pub fn block(&self, reason: Blocked) {
        if let Some(i) = BLOCKED.iter().position(|b| *b == reason) {
            Self::add(&self.blocked[i]);
        }
    }

    /// How often each gate closed.
    pub fn blocked(&self) -> Vec<(&'static str, u64)> {
        BLOCKED
            .iter()
            .zip(&self.blocked)
            .map(|(reason, count)| (reason.as_str(), Self::get(count)))
            .collect()
    }

    pub fn record_ocr(&self, took: Duration) {
        if let Ok(mut times) = self.ocr_ms.lock() {
            if times.len() == KEPT {
                times.pop_front();
            }
            times.push_back(took.as_secs_f64() * 1000.0);
        }
    }

    pub fn ocr_ms(&self) -> Vec<f64> {
        self.ocr_ms
            .lock()
            .map(|times| times.iter().copied().collect())
            .unwrap_or_default()
    }
}

/// Nearest-rank percentile of `samples` (0 for none), `p` in 0..=100.
pub fn percentile(samples: &[f64], p: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_use_the_nearest_rank() {
        let samples: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&samples, 50.0), 50.0);
        assert_eq!(percentile(&samples, 95.0), 95.0);
        assert_eq!(percentile(&samples, 100.0), 100.0);
        assert_eq!(percentile(&[], 95.0), 0.0);
        assert_eq!(percentile(&[7.0], 95.0), 7.0);
    }
}
