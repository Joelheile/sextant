//! The ASR thread: loads the model when speech starts, transcribes the segments the lanes cut,
//! and unloads the model after five quiet minutes.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use sextant_core::audio::{Action, Lifecycle, Segment, utterance};
use sextant_core::{Item, Speaker};

use super::devices::Speech;
use super::pipeline::wanted;
use super::stats::AudioStats;
use crate::capture::{Control, Sink, Stats};
use crate::clock;

/// How often the thread looks at the stop flag and the unload timer when nothing arrives.
const TICK: Duration = Duration::from_millis(200);

/// A speech segment and where it came from. Never printed.
pub struct Job {
    pub speaker: Speaker,
    pub stream_start_s: i64,
    pub segment: Segment,
    pub audio: Vec<f32>,
}

pub enum Msg {
    /// Someone started talking: have the model ready by the time the segment ends.
    Speaking,
    Segment(Job),
}

pub struct Transcriber<S: Speech> {
    pub speech: S,
    pub control: Arc<Control>,
    pub stats: Arc<AudioStats>,
    pub sink: Sink,
}

impl<S: Speech> Transcriber<S> {
    pub fn run(mut self, jobs: Receiver<Msg>, stop: &AtomicBool) {
        let start = Instant::now();
        let mut lifecycle = Lifecycle::new();
        while !stop.load(Ordering::Relaxed) {
            let now_ms = start.elapsed().as_millis() as i64;
            match jobs.recv_timeout(TICK) {
                Ok(Msg::Speaking) => {
                    self.ready(&mut lifecycle, now_ms);
                }
                Ok(Msg::Segment(job)) => self.transcribe(job, &mut lifecycle, now_ms),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if lifecycle.tick(start.elapsed().as_millis() as i64) == Action::Unload {
                self.speech.unload();
            }
        }
    }

    /// Loads the model when speech asks for it. Returns whether it is in memory.
    fn ready(&mut self, lifecycle: &mut Lifecycle, now_ms: i64) -> bool {
        if lifecycle.speech(now_ms) == Action::Load {
            match self.speech.load() {
                Ok(()) => lifecycle.loaded(now_ms),
                Err(err) => {
                    eprintln!("loading the speech model failed: {err:#}");
                    Stats::add(&self.stats.load_errors);
                    lifecycle.load_failed(now_ms);
                }
            }
        }
        lifecycle.is_loaded()
    }

    fn transcribe(&mut self, job: Job, lifecycle: &mut Lifecycle, now_ms: i64) {
        if !wanted(&self.control, clock::now()) {
            return;
        }
        if !self.ready(lifecycle, now_ms) {
            Stats::add(&self.stats.unheard);
            return;
        }
        let text = match self.speech.transcribe(&job.audio) {
            Ok(text) => text,
            Err(err) => {
                eprintln!("transcribing failed: {err:#}");
                Stats::add(&self.stats.asr_errors);
                return;
            }
        };
        if let Some(utterance) = utterance(job.speaker, job.stream_start_s, job.segment, &text) {
            Stats::add(&self.stats.utterances);
            (self.sink)(Item::Utterance(utterance), Instant::now());
        }
    }
}
