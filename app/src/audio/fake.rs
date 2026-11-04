//! Fake devices and models: the pipeline runs against scripted mic users and PCM
//! that the test pushes into the open streams, with no permission, device or model.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use sextant_core::audio::MicUser;
use sextant_core::capture::Browser;

use super::devices::{Chunk, Devices, Inbox, Speech, Vad};

#[derive(Default)]
pub struct World {
    pub users: Mutex<Vec<MicUser>>,
    mic: Mutex<Option<Inbox>>,
    system: Mutex<Option<Inbox>>,
    pub fail_system: AtomicBool,
    /// What the front window of a browser says about itself.
    pub private_window: AtomicBool,
    pub system_pids: Mutex<Vec<i32>>,
    pub opened: AtomicUsize,
    pub open_streams: AtomicUsize,
    /// When the last stream was closed.
    pub closed_at: Mutex<Option<Instant>>,
    pub loads: AtomicU32,
    pub unloads: AtomicU32,
    pub transcribed: AtomicU32,
    /// When the ASR model object was dropped, which happens when its thread exits.
    pub asr_gone_at: Mutex<Option<Instant>>,
    /// A transcription is running.
    pub busy: AtomicBool,
    /// How long one transcription takes.
    pub asr_delay: Mutex<Duration>,
}

impl World {
    pub fn call_app(&self, pid: i32, bundle_id: &str) {
        *self.users.lock().expect("users") = vec![MicUser {
            pid,
            bundle_id: bundle_id.into(),
            name: bundle_id.into(),
        }];
    }

    pub fn hang_up(&self) {
        self.users.lock().expect("users").clear();
    }

    /// Delivers one block to the open mic stream, if there is one.
    pub fn mic(&self, chunk: Chunk) {
        Self::deliver(&self.mic, chunk);
    }

    pub fn system(&self, chunk: Chunk) {
        Self::deliver(&self.system, chunk);
    }

    fn deliver(slot: &Mutex<Option<Inbox>>, chunk: Chunk) {
        if let Some(inbox) = slot.lock().expect("slot").as_ref() {
            inbox.force_push(chunk);
        }
    }

    pub fn streams(&self) -> usize {
        self.open_streams.load(Ordering::Relaxed)
    }
}

pub struct FakeDevices(pub Arc<World>);

pub struct FakeStream {
    world: Arc<World>,
    speaker_is_mic: bool,
}

impl Drop for FakeStream {
    fn drop(&mut self) {
        let slot = if self.speaker_is_mic {
            &self.world.mic
        } else {
            &self.world.system
        };
        *slot.lock().expect("slot") = None;
        self.world.open_streams.fetch_sub(1, Ordering::Relaxed);
        *self.world.closed_at.lock().expect("closed") = Some(Instant::now());
    }
}

impl FakeDevices {
    fn open(&self, slot: &Mutex<Option<Inbox>>, into: Inbox, mic: bool) -> FakeStream {
        *slot.lock().expect("slot") = Some(into);
        self.0.opened.fetch_add(1, Ordering::Relaxed);
        self.0.open_streams.fetch_add(1, Ordering::Relaxed);
        FakeStream {
            world: self.0.clone(),
            speaker_is_mic: mic,
        }
    }
}

impl Devices for FakeDevices {
    type Stream = FakeStream;

    fn mic_users(&mut self) -> Vec<MicUser> {
        self.0.users.lock().expect("users").clone()
    }

    fn front_window_private(&mut self, _: &str, _: Browser) -> Option<bool> {
        Some(self.0.private_window.load(Ordering::Relaxed))
    }

    fn open_mic(&mut self, into: Inbox) -> Result<FakeStream> {
        Ok(self.open(&self.0.mic, into, true))
    }

    fn open_system(&mut self, pids: &[i32], into: Inbox) -> Result<FakeStream> {
        if self.0.fail_system.load(Ordering::Relaxed) {
            bail!("no tap");
        }
        *self.0.system_pids.lock().expect("pids") = pids.to_vec();
        Ok(self.open(&self.0.system, into, false))
    }
}

/// Loud blocks count as speech.
pub struct FakeVad;

impl Vad for FakeVad {
    fn probability(&mut self, frame: &[f32]) -> f32 {
        let energy = frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32;
        if energy > 1e-3 { 0.9 } else { 0.02 }
    }
}

pub struct FakeSpeech(pub Arc<World>);

impl Drop for FakeSpeech {
    fn drop(&mut self) {
        *self.0.asr_gone_at.lock().expect("gone") = Some(Instant::now());
    }
}

impl Speech for FakeSpeech {
    fn load(&mut self) -> Result<()> {
        self.0.loads.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn unload(&mut self) {
        self.0.unloads.fetch_add(1, Ordering::Relaxed);
    }

    fn transcribe(&mut self, _audio: &[f32]) -> Result<String> {
        let delay = *self.0.asr_delay.lock().expect("delay");
        self.0.busy.store(true, Ordering::Relaxed);
        std::thread::sleep(delay);
        self.0.busy.store(false, Ordering::Relaxed);
        let n = self.0.transcribed.fetch_add(1, Ordering::Relaxed) + 1;
        Ok(format!("sentence number {n}"))
    }
}

/// `seconds` of a 220 Hz tone (speech to the fake VAD) or silence, as interleaved blocks of
/// 100 ms at `rate` with `channels` channels.
pub fn blocks(seconds: f32, rate: u32, channels: u16, speech: bool) -> Vec<Chunk> {
    let frames = (rate / 10) as usize;
    (0..(seconds * 10.0).round() as usize)
        .map(|block| {
            let samples = (0..frames * usize::from(channels))
                .map(|i| {
                    let n = (block * frames + i / usize::from(channels)) as f32;
                    if speech {
                        0.3 * (2.0 * std::f32::consts::PI * 220.0 * n / rate as f32).sin()
                    } else {
                        0.0
                    }
                })
                .collect();
            Chunk {
                samples,
                channels,
                rate,
            }
        })
        .collect()
}

/// Speech, then silence.
pub fn talk(speech: f32, silence: f32, rate: u32, channels: u16) -> Vec<Chunk> {
    let mut chunks = blocks(speech, rate, channels, true);
    chunks.extend(blocks(silence, rate, channels, false));
    chunks
}
