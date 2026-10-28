//! One audio stream on its way to speech segments: resample to 16 kHz mono, keep the ring, run
//! the VAD over it in frames, and cut the segments the ASR thread will transcribe.

use std::sync::mpsc::Sender;

use sextant_core::Speaker;
use sextant_core::audio::{FRAME, Ring, Segment, Segmenter, To16k};

use super::devices::{Inbox, Vad};
use super::stats::AudioStats;
use super::transcriber::{Job, Msg};
use crate::capture::Stats;

const RING_SECONDS: usize = 30;

pub struct Lane<St, V> {
    speaker: Speaker,
    inbox: Inbox,
    stream: Option<St>,
    to16k: To16k,
    scratch: Vec<f32>,
    ring: Ring,
    vad: V,
    segmenter: Segmenter,
    /// Position of the next frame the VAD has not seen.
    next_frame: u64,
    speaking: bool,
    /// Unix time of the stream's first sample.
    started_s: i64,
}

impl<St, V: Vad> Lane<St, V> {
    pub fn new(speaker: Speaker, inbox: Inbox, stream: St, vad: V, started_s: i64) -> Self {
        Self {
            speaker,
            inbox,
            stream: Some(stream),
            to16k: To16k::new(),
            scratch: Vec::new(),
            ring: Ring::seconds(RING_SECONDS),
            vad,
            segmenter: Segmenter::new(),
            next_frame: 0,
            speaking: false,
            started_s,
        }
    }

    /// Takes in everything the stream delivered since the last call.
    pub fn drain(&mut self, out: &Sender<Msg>, stats: &AudioStats) {
        while let Some(chunk) = self.inbox.pop() {
            self.scratch.clear();
            self.to16k.push(
                &chunk.samples,
                chunk.channels,
                chunk.rate,
                &mut self.scratch,
            );
            self.ring.push(&self.scratch);
            self.run_vad(out, stats);
        }
    }

    fn run_vad(&mut self, out: &Sender<Msg>, stats: &AudioStats) {
        let frame = FRAME as u64;
        while self.ring.end() - self.next_frame >= frame {
            let samples = self.ring.slice(self.next_frame, self.next_frame + frame);
            let prob = samples.map_or(0.0, |samples| self.vad.probability(&samples));
            self.next_frame += frame;
            if let Some(segment) = self.segmenter.push(prob) {
                self.cut(segment, out, stats);
            }
        }
        if self.segmenter.speaking() && !self.speaking {
            let _ = out.send(Msg::Speaking);
        }
        self.speaking = self.segmenter.speaking();
    }

    fn cut(&mut self, segment: Segment, out: &Sender<Msg>, stats: &AudioStats) {
        Stats::add(&stats.segments);
        match self.ring.slice(segment.start, segment.end) {
            Some(audio) => {
                let _ = out.send(Msg::Segment(Job {
                    speaker: self.speaker,
                    stream_start_s: self.started_s,
                    segment,
                    audio,
                }));
            }
            None => Stats::add(&stats.lost),
        }
    }

    /// Stops the stream. With `keep`, the audio already delivered is still cut and sent on;
    /// without, it is dropped unheard.
    pub fn end(mut self, out: &Sender<Msg>, stats: &AudioStats, keep: bool) {
        drop(self.stream.take());
        if keep {
            self.drain(out, stats);
            if let Some(segment) = self.segmenter.finish() {
                self.cut(segment, out, stats);
            }
        }
    }
}
