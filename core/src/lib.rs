//! Sextant logic without OS APIs: storage, work graph, routines. See plan.md §7.

pub mod agent;
pub mod audio;
pub mod capture;
pub mod connect;
mod debug;
mod error;
mod finder;
pub mod fixtures;
pub mod mcp;
mod model;
mod pages;
mod redact;
mod routines;
mod search;
mod sent;
mod sessions;
pub mod settings;
mod store;
mod text;
mod time;
mod writer;

pub use debug::Facts;
pub use error::{Error, Result};
pub use finder::ValueType;
pub use model::{Event, EventKind, Frame, Item, OcrLine, Source, Speaker, Utterance, Window};
pub use redact::{Secret, redact};
pub use routines::Routine;
pub use search::HitSource;
pub use sent::Sent;
pub use sessions::Session;
pub use settings::{Live, Settings};
pub use store::{Counts, Key, Store};
pub use time::Local;
pub use writer::{Flushed, Writer};
