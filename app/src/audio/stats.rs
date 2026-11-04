//! Counts of the audio pipeline. Numbers only, never audio or text.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Default)]
pub struct AudioStats {
    /// Calls that opened the streams.
    pub calls: AtomicU64,
    /// Speech segments cut by the VAD.
    pub segments: AtomicU64,
    /// Segments that gave text.
    pub utterances: AtomicU64,
    /// Segments whose audio was already overwritten in the ring.
    pub lost: AtomicU64,
    /// Segments dropped because the model was not loaded.
    pub unheard: AtomicU64,
    pub stream_errors: AtomicU64,
    pub load_errors: AtomicU64,
    pub asr_errors: AtomicU64,
    listening: AtomicBool,
}

impl AudioStats {
    /// Whether the streams are open, so the tray shows the dot.
    pub fn listening(&self) -> bool {
        self.listening.load(Ordering::Relaxed)
    }

    pub(super) fn set_listening(&self, on: bool) {
        self.listening.store(on, Ordering::Relaxed);
    }
}
