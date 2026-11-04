//! Audio logic without OS APIs: when a call is on, the 30 s ring buffer, 16 kHz
//! mono conversion, speech segmentation around a VAD probability stream, the ASR model lifecycle
//! and utterance assembly. Samples live in memory only. Time and OS state are passed in.

mod calls;
mod lifecycle;
mod resample;
mod ring;
mod segmenter;
mod utterance;

pub use calls::{CALL_APPS, Call, CallTracker, Change, MicUser, belongs, detect};
pub use lifecycle::{Action, Lifecycle, UNLOAD_AFTER_MS};
pub use resample::To16k;
pub use ring::Ring;
pub use segmenter::{FRAME, Segment, Segmenter};
pub use utterance::utterance;

/// Sample rate the VAD and the ASR model take.
pub const RATE: u32 = 16_000;
