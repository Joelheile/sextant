//! The 30 s ring buffer (plan.md §4): the last seconds of 16 kHz mono audio, addressed by absolute
//! sample position, so the segmenter can ask for a span it saw and get it back, or learn that it
//! was overwritten.

use super::RATE;

pub struct Ring {
    samples: Vec<f32>,
    /// Total samples ever pushed.
    end: u64,
}

impl Ring {
    pub fn seconds(seconds: usize) -> Self {
        Self {
            samples: vec![0.0; seconds * RATE as usize],
            end: 0,
        }
    }

    pub fn push(&mut self, input: &[f32]) {
        let capacity = self.samples.len();
        // Only the last `capacity` samples can survive a push larger than the ring.
        let mut rest = &input[input.len().saturating_sub(capacity)..];
        let skipped = (input.len() - rest.len()) as u64;
        let mut at = ((self.end + skipped) % capacity as u64) as usize;
        while !rest.is_empty() {
            let run = rest.len().min(capacity - at);
            self.samples[at..at + run].copy_from_slice(&rest[..run]);
            at = (at + run) % capacity;
            rest = &rest[run..];
        }
        self.end += input.len() as u64;
    }

    /// Position after the newest sample.
    pub fn end(&self) -> u64 {
        self.end
    }

    /// Position of the oldest sample still held.
    pub fn start(&self) -> u64 {
        self.end.saturating_sub(self.samples.len() as u64)
    }

    /// The samples in `from..to`, or `None` if any of them is gone or not pushed yet.
    pub fn slice(&self, from: u64, to: u64) -> Option<Vec<f32>> {
        if from > to || from < self.start() || to > self.end {
            return None;
        }
        let capacity = self.samples.len() as u64;
        Some(
            (from..to)
                .map(|at| self.samples[(at % capacity) as usize])
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(from: u32, to: u32) -> Vec<f32> {
        (from..to).map(|n| n as f32).collect()
    }

    #[test]
    fn returns_what_was_pushed_by_position() {
        let mut ring = Ring::seconds(30);
        ring.push(&ramp(0, 1000));
        ring.push(&ramp(1000, 1500));
        assert_eq!(ring.slice(900, 1100), Some(ramp(900, 1100)));
        assert_eq!(ring.end(), 1500);
        assert_eq!(ring.start(), 0);
    }

    #[test]
    fn old_audio_is_overwritten_after_30_seconds() {
        let mut ring = Ring::seconds(30);
        let total = 30 * RATE + 1234;
        for from in (0..total).step_by(4000) {
            ring.push(&ramp(from, (from + 4000).min(total)));
        }
        assert_eq!(ring.start(), 1234);
        assert_eq!(ring.slice(0, 100), None);
        assert_eq!(ring.slice(1233, 1240), None);
        assert_eq!(ring.slice(1234, 1240), Some(ramp(1234, 1240)));
        assert_eq!(
            ring.slice(total as u64 - 5, total as u64),
            Some(ramp(total - 5, total))
        );
        assert_eq!(ring.slice(total as u64 - 5, total as u64 + 1), None);
    }

    #[test]
    fn a_push_larger_than_the_ring_keeps_the_newest_samples() {
        let mut ring = Ring::seconds(1);
        ring.push(&ramp(0, 40_000));
        assert_eq!(ring.start(), 24_000);
        assert_eq!(ring.slice(24_000, 24_003), Some(ramp(24_000, 24_003)));
        assert_eq!(ring.slice(39_990, 40_000), Some(ramp(39_990, 40_000)));
    }
}
