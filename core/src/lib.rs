//! Sextant logic without OS APIs: storage, work graph, finder. See plan.md §7.

pub mod backfill;
pub mod capture;
pub mod connect;
mod corrections;
mod error;
mod finder;
pub mod fixtures;
pub mod mcp;
mod model;
pub mod profile_fixture;
mod redact;
mod search;
pub mod settings;
pub mod skill;
mod store;
mod text;
mod time;
mod writer;

pub use corrections::{Rule, Target, parse as parse_correction};
pub use error::{Error, Result};
pub use finder::{
    Action, CaseExample, Category, CopyPoint, Enjoyment, Kind, OpenCommitment, RefreshStats,
    StepInfo, Workflow, WorkflowDetail,
};
pub use model::{Event, EventKind, Frame, Item, OcrLine, Source, Speaker, Utterance, Window};
pub use redact::{Secret, redact, redact_json};
pub use search::{Hit, HitSource};
pub use settings::{Live, Settings, Sources};
pub use store::{Counts, Key, Retained, Store};
pub use writer::{Flushed, Writer};
