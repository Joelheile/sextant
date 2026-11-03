//! The database worker (plan.md §5): one thread owns the write connection and the [`Writer`],
//! flushes every second and refreshes every 5 minutes. Commands read through a separate
//! read-only connection. Live capture runs on its own
//! threads and sends finished records here; no database call happens on those threads.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use sextant_core::capture::{Stall, stall};
use sextant_core::{Item, Key, Secret, Settings, Store};

use crate::capture::{Control, FLUSH_EVERY, Ingest, Pipeline, Sink, Stats, percentile};
use crate::{audio, clock, debug, home, models, platform, prefs};

const REFRESH_EVERY: Duration = Duration::from_secs(300);
const RETENTION_EVERY: Duration = Duration::from_secs(3600);
const PAUSE_STEP_S: i64 = 3600;
const STOP_DOWNLOAD_WAIT: Duration = Duration::from_secs(5);

type Job = Box<dyn FnOnce(&mut Store) + Send>;

enum Msg {
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
    /// On, but nothing reaches the database.
    Stalled(Stall),
}

/// What the tray shows about call audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioState {
    /// Switched off, or the models are not downloaded.
    Off,
    /// On, waiting for a call app to use the mic.
    Waiting,
    /// A call is open and its audio is being transcribed.
    Transcribing,
}

/// The model download, while it runs.
struct Download {
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicU64>,
    total: u64,
}

/// What `sextant audio` shows about the model download.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct DownloadState {
    pub running: bool,
    pub done: u64,
    pub total: u64,
    /// Why the last download stopped, if it failed.
    pub failed: Option<String>,
}

pub struct Engine {
    dir: PathBuf,
    inbox: Sender<Msg>,
    /// `None` once [`Engine::shutdown`] has closed the database.
    reader: Mutex<Option<Store>>,
    control: Arc<Control>,
    capture: Mutex<Option<Pipeline>>,
    audio: Mutex<Option<audio::Pipeline>>,
    download: Arc<Mutex<Option<Download>>>,
    download_failed: Arc<Mutex<Option<String>>>,
    /// Since when flushes fail, 0 while they work.
    writes_failing_since: Arc<AtomicI64>,
}

impl Engine {
    pub fn start(dir: &Path, key: &Key) -> Result<Self> {
        create_private_dir(dir)?;
        let store =
            Store::open(&home::db_path(dir), key, &secret()?).context("opening the database")?;
        let reader = store.reader().context("opening the read connection")?;
        let (inbox, messages) = mpsc::channel();
        let dir = dir.to_path_buf();
        let writes_failing_since = Arc::new(AtomicI64::new(0));
        thread::Builder::new()
            .name("sextant-db".into())
            .spawn({
                let (dir, failing) = (dir.clone(), writes_failing_since.clone());
                move || work(store, &dir, messages, &failing)
            })
            .context("starting the database thread")?;
        Ok(Self {
            dir,
            inbox,
            reader: Mutex::new(Some(reader)),
            control: Arc::new(Control::new(Settings::default())),
            capture: Mutex::new(None),
            audio: Mutex::new(None),
            download: Arc::new(Mutex::new(None)),
            download_failed: Arc::new(Mutex::new(None)),
            writes_failing_since,
        })
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
        while self.download_state().running && Instant::now() < deadline {
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

    /// Downloads the models after the user agreed. [`Engine::download_state`] reports progress.
    /// Does nothing while a download runs.
    pub fn download_models(&self) -> Result<()> {
        let mut running = self
            .download
            .lock()
            .map_err(|_| anyhow::anyhow!("the download state is poisoned"))?;
        if running.is_some() {
            return Ok(());
        }
        let (cancel, done) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicU64::new(0)),
        );
        *running = Some(Download {
            cancel: cancel.clone(),
            done: done.clone(),
            total: models::total_bytes(models::ASSETS),
        });
        if let Ok(mut failed) = self.download_failed.lock() {
            *failed = None;
        }
        let (dir, slot, failed) = (
            self.models_dir(),
            self.download.clone(),
            self.download_failed.clone(),
        );
        thread::Builder::new()
            .name("sextant-models".into())
            .spawn(move || {
                let progress = |progress: models::Progress| {
                    done.store(progress.done, Ordering::Relaxed);
                };
                let result = models::download(
                    &dir,
                    models::ASSETS,
                    &models::UrlSession,
                    &cancel,
                    &progress,
                );
                if let (Err(err), Ok(mut failed)) = (result, failed.lock()) {
                    *failed = Some(format!("{err:#}"));
                }
                if let Ok(mut running) = slot.lock() {
                    *running = None;
                }
            })
            .context("starting the download thread")?;
        Ok(())
    }

    pub fn download_state(&self) -> DownloadState {
        let failed = self
            .download_failed
            .lock()
            .ok()
            .and_then(|failed| failed.clone());
        match self.download.lock().as_deref() {
            Ok(Some(download)) => DownloadState {
                running: true,
                done: download.done.load(Ordering::Relaxed),
                total: download.total,
                failed,
            },
            _ => DownloadState {
                failed,
                ..DownloadState::default()
            },
        }
    }

    fn cancel_download(&self) {
        if let Ok(running) = self.download.lock()
            && let Some(download) = running.as_ref()
        {
            download.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn pause_capture(&self) {
        self.control.pause_until(clock::now() + PAUSE_STEP_S);
    }

    pub fn resume_capture(&self) {
        self.control.resume();
    }

    pub fn capture_state(&self) -> CaptureState {
        let now = clock::now();
        let Ok(capture) = self.capture.lock() else {
            return CaptureState::Off;
        };
        let Some(pipeline) = capture.as_ref() else {
            return CaptureState::Off;
        };
        if let Some(until) = self.control.paused_until(now) {
            return CaptureState::Paused(until);
        }
        let (beat, reads_failing_since) = pipeline.stats.health();
        let writes = self.writes_failing_since.load(Ordering::Relaxed);
        match stall(
            now,
            beat,
            reads_failing_since,
            (writes > 0).then_some(writes),
        ) {
            Some(stalled) => CaptureState::Stalled(stalled),
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

pub(crate) fn create_private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .context("creating the data folder")
}

fn work(mut store: Store, dir: &Path, messages: Receiver<Msg>, writes_failing_since: &AtomicI64) {
    let mut ingest = Ingest::new();
    let mut last_refresh = Instant::now();
    let mut next_retention = Instant::now();
    // The work graph is derived; rebuild it so it always matches this version's finder.
    refresh(&mut store, &mut last_refresh);
    loop {
        match messages.recv_timeout(FLUSH_EVERY) {
            Ok(Msg::Job(job)) => {
                // A job sees everything captured before it, so a delete leaves nothing behind.
                flush(&mut store, &mut ingest, writes_failing_since);
                job(&mut store);
            }
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
            flush(&mut store, &mut ingest, writes_failing_since);
        }
        if last_refresh.elapsed() >= REFRESH_EVERY {
            refresh(&mut store, &mut last_refresh);
        }
        if Instant::now() >= next_retention {
            enforce_retention(&mut store, dir);
            enforce_storage_cap(&mut store, dir);
            if let Err(err) = store.maintain(clock::now()) {
                eprintln!("maintenance failed: {err}");
                debug::record("maintenance");
            }
            next_retention = Instant::now() + RETENTION_EVERY;
        }
    }
}

fn flush(store: &mut Store, ingest: &mut Ingest, writes_failing_since: &AtomicI64) {
    let now = clock::now();
    match ingest.flush(store, now) {
        Ok((_, latencies)) => {
            writes_failing_since.store(0, Ordering::Relaxed);
            debug::flushed(&latencies);
        }
        Err(err) => {
            let _ =
                writes_failing_since.compare_exchange(0, now, Ordering::Relaxed, Ordering::Relaxed);
            eprintln!("flush failed: {err}");
            debug::record("flush");
        }
    }
}

/// Deletes the oldest raw days while the database is over the storage cap the user chose.
fn enforce_storage_cap(store: &mut Store, dir: &Path) {
    let Ok(Some(cap_mb)) = prefs::load(dir).map(|settings| settings.storage_cap_mb) else {
        return;
    };
    if let Err(err) = store.cap_storage(clock::local(), u64::from(cap_mb) * 1_000_000) {
        eprintln!("storage cap failed: {err}");
        debug::record("storage cap");
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
