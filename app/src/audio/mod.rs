//! Call audio: when another app uses the mic and it is a call app, listen to the mic
//! ("me") and to that app's sound ("them"), cut speech out with a VAD, and transcribe it. Audio
//! lives in memory only, in rings of 30 s, and is gone when the call ends. The logic is generic
//! over [`Devices`], [`Vad`] and [`Speech`], so it runs against the macOS platform and against the fake.
//! `cfg` appears only in `platform`.

mod devices;
#[cfg(test)]
mod fake;
mod lane;
mod pipeline;
mod stats;
#[cfg(test)]
mod tests;
mod transcriber;

pub use devices::{Chunk, Devices, Inbox, Runs, Speech, Vad};
pub use pipeline::Pipeline;
pub use sextant_core::audio::RATE;
