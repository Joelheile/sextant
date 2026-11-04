//! Batches finished records for the writer: the database thread pushes what the
//! capture and OCR threads send and flushes about once a second, so a flush is one transaction
//! and nothing waits on the database during capture or OCR.

use std::time::{Duration, Instant};

use sextant_core::{Flushed, Item, Result, Store, Writer};

pub const FLUSH_EVERY: Duration = Duration::from_secs(1);

pub struct Ingest {
    writer: Writer,
    /// When each pending live record was captured.
    taken: Vec<Instant>,
    last_flush: Instant,
}

impl Ingest {
    pub fn new() -> Self {
        Self {
            writer: Writer::new(),
            taken: Vec::new(),
            last_flush: Instant::now(),
        }
    }

    pub fn push(&mut self, item: Item, taken: Instant) {
        self.writer.push(item);
        self.taken.push(taken);
    }

    pub fn due(&self) -> bool {
        self.writer.pending() > 0 && self.last_flush.elapsed() >= FLUSH_EVERY
    }

    /// Writes everything pushed so far. Returns the counts and, for each live record, the time
    /// from capture until it is searchable.
    pub fn flush(&mut self, store: &mut Store, now_s: i64) -> Result<(Flushed, Vec<Duration>)> {
        let flushed = self.writer.flush(store, now_s)?;
        self.last_flush = Instant::now();
        let latencies = self.taken.drain(..).map(|taken| taken.elapsed()).collect();
        Ok((flushed, latencies))
    }
}
