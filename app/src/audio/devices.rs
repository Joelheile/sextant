//! What the pipeline needs from the OS and from the models.

use std::sync::Arc;

use anyhow::Result;
use crossbeam_queue::ArrayQueue;
use sextant_core::audio::MicUser;

/// A block of interleaved samples as the OS delivered it. Never printed: only its length is.
pub struct Chunk {
    pub samples: Vec<f32>,
    pub channels: u16,
    pub rate: u32,
}

/// Where a stream drops its chunks. Bounded, the oldest go first, so a stalled reader costs a
/// little audio and no memory.
pub type Inbox = Arc<ArrayQueue<Chunk>>;

const INBOX_CHUNKS: usize = 512;

pub fn inbox() -> Inbox {
    Arc::new(ArrayQueue::new(INBOX_CHUNKS))
}

pub trait Devices: Send + 'static {
    /// Dropping it stops the stream.
    type Stream: Send;

    /// The processes that have mic input running right now.
    fn mic_users(&mut self) -> Vec<MicUser>;
    fn open_mic(&mut self, into: Inbox) -> Result<Self::Stream>;
    /// The sound the given processes play.
    fn open_system(&mut self, pids: &[i32], into: Inbox) -> Result<Self::Stream>;
}

/// Voice activity for one stream.
pub trait Vad: Send {
    /// How likely the next 512 samples (16 kHz) are speech.
    fn probability(&mut self, frame: &[f32]) -> f32;
}

/// The ASR model.
pub trait Speech: Send + 'static {
    /// Brings the ASR model into memory.
    fn load(&mut self) -> Result<()>;
    fn unload(&mut self);
    fn transcribe(&mut self, audio: &[f32]) -> Result<String>;
}

/// Where the ASR model runs. `sextant bench --asr` measures both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runs {
    Cpu,
    CoreMl,
}
