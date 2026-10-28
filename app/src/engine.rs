//! The database worker (plan.md §5): one thread owns the write connection and the [`Writer`],
//! flushes every second, refreshes every 5 minutes and after a backfill, and reports progress as
//! events. Commands read through a separate read-only connection. Live capture runs on its own
//! threads and sends finished records here; no database call happens on those threads.

use std::fs;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use sextant_core::backfill;
use sextant_core::{Item, Key, Secret, Settings, Store};

use crate::capture::{Control, Ingest, Pipeline, Sink};
use crate::{clock, home, platform, prefs};

const FLUSH_EVERY: Duration = Duration::from_secs(1);
const REFRESH_EVERY: Duration = Duration::from_secs(300);
const PAUSE_STEP_S: i64 = 3600;

type Emit = Box<dyn Fn(&str, Value) + Send>;

enum Msg {
    Backfill,
    /// A finished live record and when its screen was captured.
    Capture(Item, Instant),
}

/// What the tray shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureState {
    Off,
    Capturing,
    /// Until this Unix time.
    Paused(i64),
}

pub struct Engine {
    inbox: Sender<Msg>,
    reader: Mutex<Store>,
    control: Arc<Control>,
    capture: Mutex<Option<Pipeline>>,
}

impl Engine {
    pub fn start(
        dir: &Path,
        key: &Key,
        emit: impl Fn(&str, Value) + Send + 'static,
    ) -> Result<Self> {
        create_private_dir(dir)?;
        let mut store =
            Store::open(&home::db_path(dir), key, &secret()?).context("opening the database")?;
        store.set_utc_offset(clock::utc_offset(clock::now()));
        let reader = store.reader().context("opening the read connection")?;
        let (inbox, messages) = mpsc::channel();
        let (dir, emit) = (dir.to_path_buf(), Box::new(emit) as Emit);
        thread::Builder::new()
            .name("sextant-db".into())
            .spawn(move || work(store, &dir, messages, &emit))
            .context("starting the database thread")?;
        Ok(Self {
            inbox,
            reader: Mutex::new(reader),
            control: Arc::new(Control::new(Settings::default())),
            capture: Mutex::new(None),
        })
    }

    /// Replaces the backfill events with a fresh read of the ticked sources, then refreshes.
    pub fn start_backfill(&self) -> Result<()> {
        self.inbox
            .send(Msg::Backfill)
            .map_err(|_| anyhow::anyhow!("the database thread has stopped"))
    }

    /// Starts live capture with `settings`. Does nothing when it already runs.
    pub fn start_capture(&self, settings: Settings) -> Result<()> {
        let mut capture = self
            .capture
            .lock()
            .map_err(|_| anyhow::anyhow!("the capture state is poisoned"))?;
        if capture.is_some() {
            return Ok(());
        }
        self.control.set_settings(settings);
        let inbox = self.inbox.clone();
        let sink: Sink = Arc::new(move |item, taken| {
            let _ = inbox.send(Msg::Capture(item, taken));
        });
        *capture = Some(platform::start_live(self.control.clone(), sink)?);
        Ok(())
    }

    pub fn pause_capture(&self) {
        self.control.pause_until(clock::now() + PAUSE_STEP_S);
    }

    pub fn resume_capture(&self) {
        self.control.resume();
    }

    pub fn capture_state(&self) -> CaptureState {
        let running = self.capture.lock().is_ok_and(|capture| capture.is_some());
        match self.control.paused_until(clock::now()) {
            _ if !running => CaptureState::Off,
            Some(until) => CaptureState::Paused(until),
            None => CaptureState::Capturing,
        }
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

/// Random bytes for the email hashes. The database keeps the first ones it gets.
fn secret() -> Result<Secret> {
    let mut secret = [0u8; 16];
    getrandom::fill(&mut secret).map_err(|err| anyhow::anyhow!("no random numbers: {err}"))?;
    Ok(secret)
}

fn create_private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .context("creating the data folder")
}

fn work(mut store: Store, dir: &Path, messages: Receiver<Msg>, emit: &Emit) {
    let mut ingest = Ingest::new();
    let mut last_refresh = Instant::now();
    loop {
        match messages.recv_timeout(FLUSH_EVERY) {
            Ok(Msg::Backfill) => {
                match backfill_and_refresh(&mut store, &mut ingest, dir, emit) {
                    Ok(summary) => emit("backfill-done", summary),
                    Err(err) => emit("backfill-failed", json!({"message": err.to_string()})),
                }
                last_refresh = Instant::now();
            }
            Ok(Msg::Capture(item, taken)) => ingest.push(item, taken),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if ingest.due()
            && let Err(err) = ingest.flush(&mut store, clock::now())
        {
            eprintln!("flush failed: {err}");
        }
        if last_refresh.elapsed() >= REFRESH_EVERY {
            refresh(&mut store, &mut last_refresh);
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
    ingest: &mut Ingest,
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
        ingest.push(Item::Event(event), Instant::now());
    }
    ingest.flush(store, now)?;

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
