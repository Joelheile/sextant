//! The database worker (plan.md §5): one thread owns the write connection and the [`Writer`],
//! flushes every second, refreshes every 5 minutes and after a backfill, and reports progress as
//! events. Commands read through a separate read-only connection. Live capture runs on its own
//! threads and sends finished records here; no database call happens on those threads.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use sextant_core::backfill;
use sextant_core::{Item, Key, Secret, Settings, Store};

use crate::capture::{Control, FLUSH_EVERY, Ingest, Pipeline, Sink, Stats, percentile};
use crate::{audio, clock, debug, home, models, platform, prefs};

const REFRESH_EVERY: Duration = Duration::from_secs(300);
const RETENTION_EVERY: Duration = Duration::from_secs(3600);
const PAUSE_STEP_S: i64 = 3600;
const STOP_DOWNLOAD_WAIT: Duration = Duration::from_secs(5);

type Emit = Arc<dyn Fn(&str, Value) + Send + Sync>;

type Job = Box<dyn FnOnce(&mut Store) + Send>;

enum Msg {
    Backfill,
    /// Runs on the write connection, then the sender of the job gets its answer.
    Job(Job),
    /// Closes the database and ends the thread; the sender waits for the acknowledgement.
    Close(Sender<()>),
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

/// What the tray shows about call audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioState {
    /// Switched off, or the models are not downloaded.
    Off,
    /// On, waiting for a call app to use the mic.
    Waiting,
    /// A call is open and its audio is being transcribed.
    Transcribing,
}

pub struct Engine {
    dir: PathBuf,
    emit: Emit,
    inbox: Sender<Msg>,
    /// `None` once [`Engine::shutdown`] has closed the database.
    reader: Mutex<Option<Store>>,
    control: Arc<Control>,
    capture: Mutex<Option<Pipeline>>,
    audio: Mutex<Option<audio::Pipeline>>,
    /// The cancel flag of the model download that runs, if one does.
    download: Arc<Mutex<Option<Arc<AtomicBool>>>>,
}

impl Engine {
    pub fn start(
        dir: &Path,
        key: &Key,
        emit: impl Fn(&str, Value) + Send + Sync + 'static,
    ) -> Result<Self> {
        create_private_dir(dir)?;
        let store =
            Store::open(&home::db_path(dir), key, &secret()?).context("opening the database")?;
        let reader = store.reader().context("opening the read connection")?;
        let (inbox, messages) = mpsc::channel();
        let (dir, emit) = (dir.to_path_buf(), Arc::new(emit) as Emit);
        thread::Builder::new()
            .name("sextant-db".into())
            .spawn({
                let (dir, emit) = (dir.clone(), emit.clone());
                move || work(store, &dir, messages, &emit)
            })
            .context("starting the database thread")?;
        Ok(Self {
            dir,
            emit,
            inbox,
            reader: Mutex::new(Some(reader)),
            control: Arc::new(Control::new(Settings::default())),
            capture: Mutex::new(None),
            audio: Mutex::new(None),
            download: Arc::new(Mutex::new(None)),
        })
    }

    /// Replaces the backfill events with a fresh read of the ticked sources, then refreshes.
    pub fn start_backfill(&self) -> Result<()> {
        self.inbox
            .send(Msg::Backfill)
            .map_err(|_| anyhow::anyhow!("the database thread has stopped"))
    }

    /// Runs `job` on the database thread, which owns the only write connection, and waits for it.
    pub fn write<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut Store) -> sextant_core::Result<T> + Send + 'static,
    ) -> Result<T> {
        let (answer, wait) = mpsc::channel();
        let job: Job = Box::new(move |store| {
            let _ = answer.send(job(store));
        });
        self.inbox
            .send(Msg::Job(job))
            .map_err(|_| anyhow::anyhow!("the database thread has stopped"))?;
        let result = wait
            .recv()
            .map_err(|_| anyhow::anyhow!("the database thread has stopped"))?;
        Ok(result?)
    }

    /// Like [`Engine::write`], then refreshes, so the next read shows the change.
    pub fn change<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut Store) -> sextant_core::Result<T> + Send + 'static,
    ) -> Result<T> {
        self.write(move |store| {
            let out = job(store)?;
            store.refresh(clock::local())?;
            Ok(out)
        })
    }

    /// Saves a settings change and hands it to the running capture.
    pub fn change_settings(&self, change: impl Fn(&mut Settings)) -> Result<Settings> {
        self.control.update(&change);
        prefs::update(&self.dir, change)
    }

    /// Stops capture, audio and downloads and closes the database, so the data folder can go.
    /// Reads fail after this. Start the app again to use it.
    pub fn shutdown(&self) -> Result<()> {
        self.cancel_download();
        let deadline = Instant::now() + STOP_DOWNLOAD_WAIT;
        while self.downloading() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
        let audio = self
            .audio
            .lock()
            .map_err(|_| anyhow::anyhow!("the audio state is poisoned"))?
            .take();
        if let Some(pipeline) = audio {
            pipeline.stop();
        }
        let capture = self
            .capture
            .lock()
            .map_err(|_| anyhow::anyhow!("the capture state is poisoned"))?
            .take();
        if let Some(pipeline) = capture {
            pipeline.stop();
        }
        let (closed, wait) = mpsc::channel();
        if self.inbox.send(Msg::Close(closed)).is_ok() {
            let _ = wait.recv();
        }
        self.reader
            .lock()
            .map_err(|_| anyhow::anyhow!("the read connection is poisoned"))?
            .take();
        Ok(())
    }

    /// Makes [`Engine::shutdown`] fail, as a panic on another thread would.
    #[cfg(test)]
    pub(crate) fn poison_capture_state(&self) {
        let _ = thread::scope(|scope| {
            scope
                .spawn(|| {
                    let _held = self.audio.lock();
                    panic!("poisoning the audio state for a test");
                })
                .join()
        });
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
        *capture = Some(platform::start_live(self.control.clone(), self.sink())?);
        drop(capture);
        if let Err(err) = self.sync_audio() {
            eprintln!("audio did not start: {err:#}");
            debug::record("starting audio");
        }
        Ok(())
    }

    fn sink(&self) -> Sink {
        let inbox = self.inbox.clone();
        Arc::new(move |item, taken| {
            let _ = inbox.send(Msg::Capture(item, taken));
        })
    }

    fn models_dir(&self) -> PathBuf {
        models::dir(&self.dir)
    }

    pub fn models_installed(&self) -> bool {
        models::installed(&self.models_dir(), models::ASSETS)
    }

    /// Switches call audio on or off. Off closes the streams at once: the audio thread sees the
    /// setting within a quarter of a second, and the threads are joined off this one.
    pub fn set_audio(&self, on: bool) -> Result<()> {
        self.change_settings(|settings| settings.live.audio = on)?;
        self.sync_audio()
    }

    /// Starts or stops the audio pipeline to match the settings, once capture has started.
    fn sync_audio(&self) -> Result<()> {
        let started = self.capture.lock().is_ok_and(|capture| capture.is_some());
        let wanted = started && self.control.settings().live.audio && self.models_installed();
        let mut audio = self
            .audio
            .lock()
            .map_err(|_| anyhow::anyhow!("the audio state is poisoned"))?;
        match (wanted, audio.is_some()) {
            (true, false) => {
                *audio = Some(platform::start_audio(
                    self.control.clone(),
                    self.sink(),
                    &self.models_dir(),
                )?);
            }
            (false, true) => {
                if let Some(pipeline) = audio.take() {
                    thread::spawn(move || pipeline.stop());
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub fn audio_state(&self) -> AudioState {
        match self.audio.lock() {
            Ok(audio) => match audio.as_ref() {
                None => AudioState::Off,
                Some(pipeline) if pipeline.stats.listening() => AudioState::Transcribing,
                Some(_) => AudioState::Waiting,
            },
            Err(_) => AudioState::Off,
        }
    }

    /// Downloads the models after the user agreed. Progress arrives as `models-progress` events,
    /// then `models-done` or `models-failed`. Does nothing while a download runs.
    pub fn download_models(&self) -> Result<()> {
        let mut running = self
            .download
            .lock()
            .map_err(|_| anyhow::anyhow!("the download state is poisoned"))?;
        if running.is_some() {
            return Ok(());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *running = Some(cancel.clone());
        let (dir, emit, slot) = (self.models_dir(), self.emit.clone(), self.download.clone());
        thread::Builder::new()
            .name("sextant-models".into())
            .spawn(move || {
                let progress = |progress: models::Progress| {
                    emit("models-progress", json!({"done": progress.done, "total": progress.total}));
                };
                match models::download(&dir, models::ASSETS, &models::UrlSession, &cancel, &progress) {
                    Ok(()) => emit("models-done", json!({})),
                    Err(err) => emit(
                        "models-failed",
                        json!({"message": format!("{err:#}"), "cancelled": cancel.load(Ordering::Relaxed)}),
                    ),
                }
                if let Ok(mut running) = slot.lock() {
                    *running = None;
                }
            })
            .context("starting the download thread")?;
        Ok(())
    }

    pub fn cancel_download(&self) {
        if let Ok(running) = self.download.lock()
            && let Some(cancel) = running.as_ref()
        {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn downloading(&self) -> bool {
        self.download.lock().is_ok_and(|running| running.is_some())
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

    /// Live capture counters and OCR timings for the debug report. Empty while capture is off.
    pub fn capture_numbers(&self) -> Vec<(&'static str, f64)> {
        let Ok(capture) = self.capture.lock() else {
            return Vec::new();
        };
        let Some(pipeline) = capture.as_ref() else {
            return Vec::new();
        };
        let stats = &pipeline.stats;
        let ocr_ms = stats.ocr_ms();
        let mut numbers = vec![
            ("ocr p50 ms", percentile(&ocr_ms, 50.0)),
            ("ocr p95 ms", percentile(&ocr_ms, 95.0)),
        ];
        let counters = [
            ("looks", &stats.looks),
            ("frames captured", &stats.captured),
            ("frames skipped", &stats.skipped),
            ("frames read by OCR", &stats.ocr),
            ("frames read from the tree", &stats.tree),
            ("frames dropped", &stats.dropped),
            ("capture errors", &stats.capture_errors),
            ("OCR errors", &stats.ocr_errors),
            ("copies", &stats.copies),
            ("presses", &stats.presses),
            ("edits", &stats.edits),
        ];
        numbers.extend(counters.map(|(name, counter)| (name, Stats::get(counter) as f64)));
        numbers.extend(
            stats
                .blocked()
                .into_iter()
                .map(|(reason, count)| (reason, count as f64)),
        );
        numbers
    }

    /// Runs `read` on the read-only connection.
    pub fn read<T>(&self, read: impl FnOnce(&Store) -> sextant_core::Result<T>) -> Result<T> {
        let reader = self
            .reader
            .lock()
            .map_err(|_| anyhow::anyhow!("the read connection is poisoned"))?;
        let store = reader
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Sextant is closed"))?;
        Ok(read(store)?)
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
    let mut next_retention = Instant::now();
    // The work graph is derived; rebuild it so it always matches this version's finder.
    refresh(&mut store, &mut last_refresh);
    loop {
        match messages.recv_timeout(FLUSH_EVERY) {
            Ok(Msg::Backfill) => {
                match backfill_and_refresh(&mut store, &mut ingest, dir, emit) {
                    Ok(summary) => emit("backfill-done", summary),
                    Err(err) => {
                        debug::record("backfill");
                        emit("backfill-failed", json!({"message": err.to_string()}));
                    }
                }
                last_refresh = Instant::now();
            }
            Ok(Msg::Job(job)) => job(&mut store),
            Ok(Msg::Close(closed)) => {
                drop(store);
                let _ = closed.send(());
                return;
            }
            Ok(Msg::Capture(item, taken)) => ingest.push(item, taken),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if ingest.due() {
            match ingest.flush(&mut store, clock::now()) {
                Ok((_, latencies)) => debug::flushed(&latencies),
                Err(err) => {
                    eprintln!("flush failed: {err}");
                    debug::record("flush");
                }
            }
        }
        if last_refresh.elapsed() >= REFRESH_EVERY {
            refresh(&mut store, &mut last_refresh);
        }
        if Instant::now() >= next_retention {
            enforce_retention(&mut store, dir);
            if let Err(err) = store.maintain(clock::now()) {
                eprintln!("maintenance failed: {err}");
                debug::record("maintenance");
            }
            next_retention = Instant::now() + RETENTION_EVERY;
        }
    }
}

/// Deletes raw data older than the retention the user chose. Forever deletes nothing. Settings
/// that cannot be read fall back to the default, so raw rows never outlive it.
fn enforce_retention(store: &mut Store, dir: &Path) {
    let days = prefs::load(dir)
        .unwrap_or_else(|err| {
            eprintln!("retention uses the default: {err:#}");
            debug::record("reading settings");
            Settings::default()
        })
        .retention_days;
    if let Some(days) = days
        && let Err(err) = store.retention(clock::local(), days)
    {
        eprintln!("retention failed: {err}");
        debug::record("retention");
    }
}

fn refresh(store: &mut Store, last_refresh: &mut Instant) {
    let started = Instant::now();
    match store.refresh(clock::local()) {
        Ok(_) => debug::refreshed(started.elapsed()),
        Err(err) => {
            eprintln!("refresh failed: {err}");
            debug::record("refresh");
        }
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
    let when = clock::local();
    let now = when.now;

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
    store.refresh(when)?;
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

#[cfg(test)]
mod tests {
    use sextant_core::fixtures::{TempDb, WEEK_NOW, load, week};

    use super::*;

    fn temp(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sextant-engine-{}-{label}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("test setup");
        dir
    }

    fn with_old_rows() -> (Store, TempDb) {
        let db = TempDb::new("retention");
        let mut store = db.open().expect("test setup");
        load(&mut store, &week(), WEEK_NOW).expect("test setup");
        assert!(store.counts().expect("counts").events > 0, "test setup");
        (store, db)
    }

    #[test]
    fn retention_runs_with_the_default_when_settings_json_is_broken() {
        let dir = temp("broken");
        fs::write(dir.join("settings.json"), "{not json").expect("test setup");
        let (mut store, _db) = with_old_rows();

        enforce_retention(&mut store, &dir);

        let counts = store.counts().expect("counts");
        assert_eq!((counts.events, counts.frames), (0, 0));
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn forever_still_deletes_nothing() {
        let dir = temp("forever");
        prefs::update(&dir, |s| s.retention_days = None).expect("test setup");
        let (mut store, _db) = with_old_rows();
        let before = store.counts().expect("counts").events;

        enforce_retention(&mut store, &dir);

        assert_eq!(store.counts().expect("counts").events, before);
        fs::remove_dir_all(dir).expect("cleanup");
    }
}
