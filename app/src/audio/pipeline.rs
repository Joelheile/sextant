//! The audio pipeline: the audio thread watches who uses the mic, opens the streams
//! for a call, and cuts speech out of them; the ASR thread transcribes it. Neither is the Tauri
//! main thread. Switching audio off, or pausing, closes the streams within one wait of the audio
//! thread, which is never longer than [`IDLE_WAIT`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use sextant_core::Speaker;
use sextant_core::audio::{Call, CallTracker, Change, detect};
use sextant_core::capture;

use super::devices::{Devices, Inbox, Speech, Vad, inbox};
use super::lane::Lane;
use super::stats::AudioStats;
use super::transcriber::{Msg, Transcriber};
use crate::capture::{Control, Sink, Stats};
use crate::clock;

/// Wait between steps while a call is open: short enough to drain the streams promptly.
const BUSY_WAIT: Duration = Duration::from_millis(20);
/// Wait between steps with no call: the longest it takes to notice the switch.
const IDLE_WAIT: Duration = Duration::from_millis(250);
/// How often the processes using the mic are listed.
const POLL_MS: i64 = 1000;

/// Whether audio should run: switched on in the settings and not paused.
pub fn wanted(control: &Control, now_s: i64) -> bool {
    control.settings().live.audio && control.paused_until(now_s).is_none()
}

pub struct Listener<D: Devices, V, F> {
    devices: D,
    new_vad: F,
    control: Arc<Control>,
    stats: Arc<AudioStats>,
    jobs: Sender<Msg>,
    own_pid: i32,
    tracker: CallTracker,
    lanes: Vec<Lane<D::Stream, V>>,
    next_poll_ms: i64,
}

impl<D, V, F> Listener<D, V, F>
where
    D: Devices,
    V: Vad,
    F: FnMut() -> Result<V>,
{
    pub fn new(
        devices: D,
        new_vad: F,
        control: Arc<Control>,
        stats: Arc<AudioStats>,
        jobs: Sender<Msg>,
    ) -> Self {
        Self {
            devices,
            new_vad,
            control,
            stats,
            jobs,
            own_pid: std::process::id() as i32,
            tracker: CallTracker::new(),
            lanes: Vec::new(),
            next_poll_ms: 0,
        }
    }

    /// One step. Returns how long to wait before the next. `now_ms` is a steady clock for the
    /// call timing, `now_s` is Unix time for the records.
    pub fn step(&mut self, now_ms: i64, now_s: i64) -> Duration {
        if !wanted(&self.control, now_s) {
            self.tracker.reset();
            self.close(false);
            return IDLE_WAIT;
        }
        if now_ms >= self.next_poll_ms {
            self.next_poll_ms = now_ms + POLL_MS;
            let seen = detect(&self.devices.mic_users(), self.own_pid);
            match self.tracker.step(now_ms, seen) {
                Some(Change::Started(call)) => self.start(&call, now_s),
                Some(Change::Ended) => self.close(true),
                None => {}
            }
        }
        for lane in &mut self.lanes {
            lane.drain(&self.jobs, &self.stats);
        }
        if self.lanes.is_empty() {
            IDLE_WAIT
        } else {
            BUSY_WAIT
        }
    }

    /// Opens the streams unless a screen gate would have closed: an excluded app, or a browser
    /// whose front window is private. The call is looked at again after the next start delay.
    fn start(&mut self, call: &Call, now_s: i64) {
        let settings = self.control.settings();
        let devices = &mut self.devices;
        let gate = capture::call(&settings, &call.name, &call.app_id, |browser| {
            devices.front_window_private(&call.app_id, browser)
        });
        if gate.is_err() {
            self.tracker.reset();
            return;
        }
        self.open(call, now_s);
    }

    fn open(&mut self, call: &Call, now_s: i64) {
        let mic = self.lane(Speaker::Mic, now_s, |devices, into| devices.open_mic(into));
        let system = self.lane(Speaker::System, now_s, |devices, into| {
            devices.open_system(&call.pids, into)
        });
        self.lanes.extend(mic.into_iter().chain(system));
        if self.lanes.is_empty() {
            self.tracker.reset();
            return;
        }
        Stats::add(&self.stats.calls);
        self.stats.set_listening(true);
    }

    fn lane(
        &mut self,
        speaker: Speaker,
        now_s: i64,
        open: impl FnOnce(&mut D, Inbox) -> Result<D::Stream>,
    ) -> Option<Lane<D::Stream, V>> {
        let into = inbox();
        let opened =
            (self.new_vad)().and_then(|vad| Ok((vad, open(&mut self.devices, into.clone())?)));
        match opened {
            Ok((vad, stream)) => Some(Lane::new(speaker, into, stream, vad, now_s)),
            Err(err) => {
                eprintln!("opening the {speaker:?} stream failed: {err:#}");
                Stats::add(&self.stats.stream_errors);
                None
            }
        }
    }

    /// Stops the streams. With `keep`, what they already delivered is still transcribed.
    pub fn close(&mut self, keep: bool) {
        for lane in self.lanes.drain(..) {
            lane.end(&self.jobs, &self.stats, keep);
        }
        self.stats.set_listening(false);
    }
}

pub struct Pipeline {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    pub stats: Arc<AudioStats>,
}

impl Pipeline {
    pub fn start<D, V, F, S>(
        devices: D,
        new_vad: F,
        speech: S,
        control: Arc<Control>,
        sink: Sink,
    ) -> Result<Self>
    where
        D: Devices,
        V: Vad + 'static,
        F: FnMut() -> Result<V> + Send + 'static,
        S: Speech,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(AudioStats::default());
        let (jobs, inbox) = mpsc::channel();

        let asr_thread = thread::Builder::new()
            .name("sextant-asr".into())
            .spawn({
                let stop = stop.clone();
                let transcriber = Transcriber {
                    speech,
                    control: control.clone(),
                    stats: stats.clone(),
                    sink,
                };
                move || transcriber.run(inbox, &stop)
            })
            .context("starting the ASR thread")?;

        let mut listener = Listener::new(devices, new_vad, control, stats.clone(), jobs);
        let audio_thread = thread::Builder::new()
            .name("sextant-audio".into())
            .spawn({
                let stop = stop.clone();
                move || {
                    let start = Instant::now();
                    while !stop.load(Ordering::Relaxed) {
                        let now_ms = start.elapsed().as_millis() as i64;
                        thread::park_timeout(listener.step(now_ms, clock::now()));
                    }
                    listener.close(false);
                }
            })
            .context("starting the audio thread")?;

        Ok(Self {
            stop,
            threads: vec![audio_thread, asr_thread],
            stats,
        })
    }

    /// Closes the streams and waits for both threads. The audio thread is joined first, so the
    /// streams are closed at once; the ASR thread finishes the segment it is on and drops the
    /// rest unheard.
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
