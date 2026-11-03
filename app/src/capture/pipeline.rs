//! The live pipeline (plan.md §4): a capture thread that runs triggers, gates and the diff gate,
//! a queue that drops the oldest frame under load, and an OCR thread. Frames live in memory only
//! and are dropped right after OCR. Neither thread is the Tauri main thread, and no database
//! transaction is open while they work: finished records go to the writer through a sink.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle, Thread};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossbeam_queue::ArrayQueue;
use sextant_core::capture::{
    self, Blocked, DiffGate, Edits, IDLE_LIMIT_S, Sample, Signal, Triggers, classify, focus_event,
    idle_event, pause_event, press_event,
};
use sextant_core::{Event, Frame, Item, OcrLine, Settings, Window};

use super::sensors::{App, Ocr, Sensors};
use super::stats::Stats;

const ACTIVE_POLL: Duration = Duration::from_millis(250);
const AWAY_POLL: Duration = Duration::from_secs(1);
/// Frames waiting for OCR. OCR is much faster than the trigger rate, so more would only be stale.
const QUEUE: usize = 2;

/// Where finished records go, with the moment the screen was captured.
pub type Sink = Arc<dyn Fn(Item, Instant) + Send + Sync>;

/// What the UI and tray change while capture runs.
pub struct Control {
    settings: Mutex<Settings>,
    paused_until: AtomicI64,
}

impl Control {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings: Mutex::new(settings),
            paused_until: AtomicI64::new(0),
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings
            .lock()
            .map(|settings| settings.clone())
            .unwrap_or_default()
    }

    pub fn set_settings(&self, settings: Settings) {
        if let Ok(mut current) = self.settings.lock() {
            *current = settings;
        }
    }

    /// Changes the settings in place, for example one switch at a time.
    pub fn update(&self, change: impl FnOnce(&mut Settings)) {
        if let Ok(mut current) = self.settings.lock() {
            change(&mut current);
        }
    }

    pub fn pause_until(&self, ts: i64) {
        self.paused_until.store(ts, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.pause_until(0);
    }

    /// When the pause ends, or `None` when capture is running.
    pub fn paused_until(&self, now_s: i64) -> Option<i64> {
        let until = self.paused_until.load(Ordering::Relaxed);
        (now_s < until).then_some(until)
    }
}

/// A frame waiting for OCR.
pub struct Job<I> {
    image: I,
    window: Window,
    ts: i64,
    taken: Instant,
}

pub type Queue<I> = ArrayQueue<Job<I>>;

fn window_key(window: &Window) -> u64 {
    let mut hasher = DefaultHasher::new();
    window.hash(&mut hasher);
    hasher.finish()
}

/// Identifies the focused app and window, so switching windows inside one app counts as a focus
/// change and passes the gates again.
fn focus_key(pid: i32, window: u64) -> u64 {
    let mut hasher = DefaultHasher::new();
    (pid, window).hash(&mut hasher);
    hasher.finish()
}

fn text_key(lines: &[OcrLine]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for line in lines {
        line.text.hash(&mut hasher);
    }
    hasher.finish()
}

pub struct Capturer<S: Sensors> {
    sensors: S,
    control: Arc<Control>,
    stats: Arc<Stats>,
    sink: Sink,
    queue: Arc<Queue<S::Image>>,
    /// Wakes the OCR thread when a frame is queued.
    waker: Option<Thread>,
    triggers: Triggers,
    diff: DiffGate,
    last_window: Option<Window>,
    /// Window and text hash of the last frame read from the Accessibility tree.
    last_tree: Option<(u64, u64)>,
    /// A `focus` event is open: the next gate or absence has to close it.
    open: bool,
    /// The focused app and window of this poll.
    focus: u64,
    /// The focus that last passed every gate. Presses and fields are read only while it holds.
    allowed: Option<u64>,
    edits: Edits,
}

impl<S: Sensors> Capturer<S> {
    pub fn new(
        sensors: S,
        control: Arc<Control>,
        stats: Arc<Stats>,
        queue: Arc<Queue<S::Image>>,
        sink: Sink,
    ) -> Self {
        Self {
            sensors,
            control,
            stats,
            sink,
            queue,
            waker: None,
            triggers: Triggers::new(),
            diff: DiffGate::new(),
            last_window: None,
            last_tree: None,
            open: false,
            focus: 0,
            allowed: None,
            edits: Edits::new(),
        }
    }

    /// One poll. Returns how long to wait before the next. `now_ms` is a steady clock for the
    /// trigger timing, `now_s` is Unix time for the records.
    pub fn step(&mut self, now_ms: i64, now_s: i64) -> Duration {
        self.stats.beat(now_s);
        let settings = self.control.settings();
        let paused = self.control.paused_until(now_s).is_some();
        if let Err(blocked) = capture::session(&settings, paused) {
            // No OS call at all while paused or with every source off.
            self.close(now_s, blocked == Blocked::Paused);
            return AWAY_POLL;
        }

        let presence = self.sensors.presence();
        let app = self.sensors.focused_app();
        let present = !presence.locked && !presence.asleep && presence.idle_s < IDLE_LIMIT_S;
        self.focus = match &app {
            Some(app) if present => focus_key(app.pid, self.sensors.window_id(app)),
            Some(app) => focus_key(app.pid, 0),
            None => 0,
        };
        let signal = self.triggers.step(Sample {
            now_ms,
            focus: self.focus,
            idle_s: presence.idle_s,
            locked: presence.locked,
            asleep: presence.asleep,
        });
        match (signal, &app) {
            (Some(Signal::Left(_)), _) => self.close(now_s, false),
            (Some(Signal::Look(_)), Some(app)) => self.look(app, now_s, &settings),
            _ => {}
        }
        self.act(app.as_ref(), now_s, &settings);

        if self.triggers.away().is_some() {
            AWAY_POLL
        } else {
            ACTIVE_POLL
        }
    }

    /// Ends the open activity without saying anything about what came next.
    fn close(&mut self, now_s: i64, paused: bool) {
        if std::mem::take(&mut self.open) {
            let event = if paused {
                pause_event(now_s)
            } else {
                idle_event(now_s)
            };
            (self.sink)(Item::Event(event), Instant::now());
        }
        self.last_window = None;
        self.allowed = None;
    }

    fn look(&mut self, app: &App, now_s: i64, settings: &Settings) {
        Stats::add(&self.stats.looks);
        let window = match self.gates(app, settings, true) {
            Ok(window) => window,
            Err(blocked) => {
                self.stats.block(blocked);
                self.close(now_s, false);
                return;
            }
        };
        self.allowed = Some(self.focus);
        if settings.live.metadata && self.last_window.as_ref() != Some(&window) {
            (self.sink)(Item::Event(focus_event(now_s, &window)), Instant::now());
            self.last_window = Some(window.clone());
            self.open = true;
        }
        if settings.live.ocr {
            self.capture(app, window, now_s);
        }
    }

    /// The gates in order, each before the OS call that would read more. `Ok` carries the window
    /// metadata the settings allow.
    fn gates(
        &mut self,
        app: &App,
        settings: &Settings,
        read_title: bool,
    ) -> Result<Window, Blocked> {
        let browser = classify(&app.bundle_id);
        capture::app(settings, &app.name, browser)?;
        capture::secure_input(self.sensors.secure_input())?;
        let mut url = String::new();
        if let Some(browser) = browser {
            let tab = self.sensors.browser_tab(app, browser).ok();
            capture::tab(tab.as_ref().map(|tab| tab.private))?;
            url = tab.map(|tab| tab.url).unwrap_or_default();
            capture::site(settings, &url)?;
        }
        let mut window = Window {
            app: app.name.clone(),
            bundle_id: app.bundle_id.clone(),
            ..Window::default()
        };
        if settings.live.metadata {
            window.url = url;
            if read_title {
                let text = self.sensors.window_text(app);
                window.title = text.title;
                window.doc = text.doc;
            }
        }
        Ok(window)
    }

    /// The Accessibility tree first; a screenshot and OCR only when it says too little.
    fn capture(&mut self, app: &App, window: Window, now_s: i64) {
        let taken = Instant::now();
        let lines = self.sensors.window_lines(app);
        if capture::enough(&lines) {
            let key = (window_key(&window), text_key(&lines));
            if self.last_tree.replace(key) == Some(key) {
                self.stats.read_ok();
                Stats::add(&self.stats.skipped);
                return;
            }
            Stats::add(&self.stats.tree);
            self.stats.read_ok();
            let frame = Frame {
                ts: now_s,
                window,
                lines,
            };
            (self.sink)(Item::Frame(frame), taken);
            return;
        }
        if self.sensors.low_power() {
            Stats::add(&self.stats.low_power);
            return;
        }
        let shot = match self.sensors.capture(app) {
            Ok(shot) => shot,
            Err(_) => {
                Stats::add(&self.stats.capture_errors);
                self.stats.read_failed(now_s);
                return;
            }
        };
        self.stats.read_ok();
        Stats::add(&self.stats.captured);
        if !self.diff.changed(window_key(&window), shot.thumb) {
            Stats::add(&self.stats.skipped);
            return;
        }
        let job = Job {
            image: shot.image,
            window,
            ts: now_s,
            taken,
        };
        if self.queue.force_push(job).is_some() {
            Stats::add(&self.stats.dropped);
        }
        if let Some(ocr) = &self.waker {
            ocr.unpark();
        }
    }

    /// Presses and the focused text field, only while the window that last passed the gates is
    /// still in front. Anything else drops the queued presses unread.
    fn act(&mut self, app: Option<&App>, now_s: i64, settings: &Settings) {
        let clicks = self.sensors.clicks();
        let readable = settings.live.actions
            && self.triggers.away().is_none()
            && self.allowed == Some(self.focus);
        let Some(app) = app
            .filter(|_| readable)
            .filter(|_| !self.sensors.secure_input())
        else {
            let left = self.edits.observe(now_s, "", None, |_| None);
            self.emit(left, &self.stats.edits);
            return;
        };
        if !clicks.is_empty() {
            let typed = self.edits.checkpoint(now_s, |id| self.sensors.reread(id));
            self.emit(typed, &self.stats.edits);
        }
        for point in clicks {
            let chain = self.sensors.element_chain(app, point);
            self.emit(press_event(now_s, &app.name, &chain), &self.stats.presses);
        }
        let field = self.sensors.focused_field(app);
        let left = self
            .edits
            .observe(now_s, &app.name, field, |id| self.sensors.reread(id));
        self.emit(left, &self.stats.edits);
    }

    fn emit(&self, event: Option<Event>, counter: &AtomicU64) {
        if let Some(event) = event {
            Stats::add(counter);
            (self.sink)(Item::Event(event), Instant::now());
        }
    }
}

/// Takes the oldest queued frame through OCR and hands the lines to the sink. Returns whether
/// there was a frame. A frame that waited through a pause is dropped unread.
pub fn recognize_next<I, O: Ocr<I>>(
    queue: &Queue<I>,
    ocr: &mut O,
    control: &Control,
    stats: &Stats,
    sink: &Sink,
    now_s: i64,
) -> bool {
    let Some(job) = queue.pop() else {
        return false;
    };
    let settings = control.settings();
    if control.paused_until(now_s).is_some() || !settings.live.ocr {
        return true;
    }
    let started = Instant::now();
    let raw = ocr.recognize(&job.image);
    stats.record_ocr(started.elapsed());
    drop(job.image);
    match raw {
        Ok(raw) => {
            Stats::add(&stats.ocr);
            let lines = capture::lines(raw);
            if !lines.is_empty() {
                let frame = Frame {
                    ts: job.ts,
                    window: job.window,
                    lines,
                };
                sink(Item::Frame(frame), job.taken);
            }
        }
        Err(_) => Stats::add(&stats.ocr_errors),
    }
    true
}

pub struct Pipeline {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    pub stats: Arc<Stats>,
}

impl Pipeline {
    pub fn start<S, O>(sensors: S, ocr: O, control: Arc<Control>, sink: Sink) -> Result<Self>
    where
        S: Sensors,
        O: Ocr<S::Image>,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(Stats::default());
        stats.beat(crate::clock::now());
        let queue = Arc::new(ArrayQueue::new(QUEUE));

        let ocr_thread = thread::Builder::new()
            .name("sextant-ocr".into())
            .spawn({
                let (queue, control, stats, stop) =
                    (queue.clone(), control.clone(), stats.clone(), stop.clone());
                let sink = sink.clone();
                let mut ocr = ocr;
                move || {
                    while !stop.load(Ordering::Relaxed) {
                        let busy = recognize_next(
                            &queue,
                            &mut ocr,
                            &control,
                            &stats,
                            &sink,
                            crate::clock::now(),
                        );
                        if !busy {
                            thread::park_timeout(Duration::from_millis(200));
                        }
                    }
                }
            })
            .context("starting the OCR thread")?;

        let mut capturer = Capturer::new(sensors, control, stats.clone(), queue, sink);
        capturer.waker = Some(ocr_thread.thread().clone());
        let capture_thread = thread::Builder::new()
            .name("sextant-capture".into())
            .spawn({
                let stop = stop.clone();
                move || {
                    capturer.sensors.wake_on_click(thread::current());
                    let start = Instant::now();
                    while !stop.load(Ordering::Relaxed) {
                        let now_ms = start.elapsed().as_millis() as i64;
                        let wait = capturer.step(now_ms, crate::clock::now());
                        thread::park_timeout(wait);
                    }
                }
            })
            .context("starting the capture thread")?;

        Ok(Self {
            stop,
            threads: vec![capture_thread, ocr_thread],
            stats,
        })
    }

    /// Stops both threads and waits for them. Queued frames are dropped.
    pub fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in &self.threads {
            thread.thread().unpark();
        }
        for thread in self.threads {
            let _ = thread.join();
        }
    }
}
