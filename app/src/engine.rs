//! The database worker (plan.md §5): one thread owns the write connection and the [`Writer`],
//! flushes every second, refreshes every 5 minutes and after a backfill, and reports progress as
//! events. Commands read through a separate read-only connection.

use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use sextant_core::backfill;
use sextant_core::{Item, Key, Store, Writer};

use crate::{clock, home, platform, prefs};

const FLUSH_EVERY: Duration = Duration::from_secs(1);
const REFRESH_EVERY: Duration = Duration::from_secs(300);

type Emit = Box<dyn Fn(&str, Value) + Send>;

pub struct Engine {
    backfill: Sender<()>,
    reader: Mutex<Store>,
}

impl Engine {
    pub fn start(
        dir: &Path,
        key: &Key,
        emit: impl Fn(&str, Value) + Send + 'static,
    ) -> Result<Self> {
        create_private_dir(dir)?;
        let mut store = Store::open(&home::db_path(dir), key).context("opening the database")?;
        store.set_utc_offset(clock::utc_offset(clock::now()));
        let reader = store.reader().context("opening the read connection")?;
        let (backfill, inbox) = mpsc::channel();
        let (dir, emit) = (dir.to_path_buf(), Box::new(emit) as Emit);
        thread::Builder::new()
            .name("sextant-db".into())
            .spawn(move || work(store, &dir, inbox, &emit))
            .context("starting the database thread")?;
        Ok(Self {
            backfill,
            reader: Mutex::new(reader),
        })
    }

    /// Replaces the backfill events with a fresh read of the ticked sources, then refreshes.
    pub fn start_backfill(&self) -> Result<()> {
        self.backfill
            .send(())
            .map_err(|_| anyhow::anyhow!("the database thread has stopped"))
    }

    /// Runs `read` on the read-only connection.
    pub fn read<T>(&self, read: impl FnOnce(&Store) -> sextant_core::Result<T>) -> Result<T> {
        let store = self
            .reader
            .lock()
            .map_err(|_| anyhow::anyhow!("the read connection is poisoned"))?;
        Ok(read(&store)?)
    }
}

fn create_private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .context("creating the data folder")
}

fn work(mut store: Store, dir: &Path, inbox: Receiver<()>, emit: &Emit) {
    let mut writer = Writer::new();
    let mut last_refresh = Instant::now();
    loop {
        match inbox.recv_timeout(FLUSH_EVERY) {
            Ok(()) => {
                match backfill_and_refresh(&mut store, &mut writer, dir, emit) {
                    Ok(summary) => emit("backfill-done", summary),
                    Err(err) => emit("backfill-failed", json!({"message": err.to_string()})),
                }
                last_refresh = Instant::now();
            }
            Err(RecvTimeoutError::Timeout) => {
                if writer.pending() > 0
                    && let Err(err) = writer.flush(&mut store, clock::now())
                {
                    eprintln!("flush failed: {err}");
                }
                if last_refresh.elapsed() >= REFRESH_EVERY {
                    refresh(&mut store, &mut last_refresh);
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn refresh(store: &mut Store, last_refresh: &mut Instant) {
    let now = clock::now();
    store.set_utc_offset(clock::utc_offset(now));
    if let Err(err) = store.refresh(now) {
        eprintln!("refresh failed: {err}");
    }
    *last_refresh = Instant::now();
}

fn backfill_and_refresh(
    store: &mut Store,
    writer: &mut Writer,
    dir: &Path,
    emit: &Emit,
) -> Result<Value> {
    let settings = prefs::load(dir)?;
    let now = clock::now();
    store.set_utc_offset(clock::utc_offset(now));

    let collected = backfill::collect(
        &platform::System,
        &settings.sources,
        &settings.git_folders,
        now,
        |stage| emit("backfill-progress", json!({"stage": stage.as_str()})),
    );
    emit("backfill-progress", json!({"stage": "saving"}));
    store.clear_backfill()?;
    let events = collected.events.len();
    for event in collected.events {
        writer.push(Item::Event(event));
    }
    writer.flush(store, now)?;

    emit("backfill-progress", json!({"stage": "analyzing"}));
    store.refresh(now)?;
    let reports: Vec<Value> = collected
        .reports
        .iter()
        .map(|r| json!({"stage": r.stage.as_str(), "events": r.events, "errors": r.errors}))
        .collect();
    Ok(json!({
        "events": events,
        "workflows": store.workflows()?.len(),
        "hours_back": store.hours_back()?,
        "sources": reports,
    }))
}
