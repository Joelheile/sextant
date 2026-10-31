//! Live capture (plan.md §4): triggers, gates, window capture, diff gate, OCR and the writer.
//! The logic is generic over [`Sensors`] and [`Ocr`], so it runs against the macOS platform
//! and against the scripted fake. `cfg` appears only in `platform`.

#[cfg(test)]
mod fake;
mod ingest;
mod pipeline;
mod sensors;
mod stats;
#[cfg(test)]
mod tests;

pub use ingest::{FLUSH_EVERY, Ingest};
pub use pipeline::{Control, Pipeline, Sink};
pub use sensors::{App, Ocr, Presence, Sensors, Shot, Tab, WindowText};
pub use stats::{Stats, percentile};
