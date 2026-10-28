//! The audio pipeline against the fake devices and models (plan.md §12): who counts as a call,
//! what each side of the call becomes, and the done-when checks of M4 that need no model:
//! nothing audio reaches the data folder, and switching audio off closes the streams within 1 s.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::Result;
use sextant_core::fixtures::TempDb;
use sextant_core::{Item, Settings, Speaker, Utterance};

use super::fake::{FakeDevices, FakeSpeech, FakeVad, World, blocks, talk};
use super::pipeline::{Listener, Pipeline};
use super::stats::AudioStats;
use super::transcriber::Transcriber;
use crate::capture::{Control, Ingest, Sink};

type Rig = Listener<FakeDevices, FakeVad, fn() -> Result<FakeVad>>;

fn new_vad() -> Result<FakeVad> {
    Ok(FakeVad)
}

fn settings() -> Settings {
    let mut settings = Settings::default();
    settings.live.audio = true;
    settings
}

/// A listener stepped by hand on a made-up clock, with the ASR thread behind it.
struct Session {
    world: Arc<World>,
    control: Arc<Control>,
    stats: Arc<AudioStats>,
    listener: Rig,
    asr: JoinHandle<()>,
    heard: Arc<Mutex<Vec<Utterance>>>,
    now_ms: i64,
}

impl Session {
    fn new() -> Self {
        Self::with_sink(|_| {})
    }

    /// `also` sees every utterance, besides the list the session keeps.
    fn with_sink(also: impl Fn(&Utterance) + Send + Sync + 'static) -> Self {
        let world = Arc::new(World::default());
        let control = Arc::new(Control::new(settings()));
        let stats = Arc::new(AudioStats::default());
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink: Sink = {
            let heard = heard.clone();
            Arc::new(move |item, _| {
                if let Item::Utterance(utterance) = item {
                    also(&utterance);
                    heard.lock().expect("heard").push(utterance);
                }
            })
        };
        let (jobs, inbox) = mpsc::channel();
        let transcriber = Transcriber {
            speech: FakeSpeech(world.clone()),
            control: control.clone(),
            stats: stats.clone(),
            sink,
        };
        let asr = thread::spawn(move || {
            transcriber.run(inbox, &std::sync::atomic::AtomicBool::new(false))
        });
        let listener = Listener::new(
            FakeDevices(world.clone()),
            new_vad as fn() -> Result<FakeVad>,
            control.clone(),
            stats.clone(),
            jobs,
        );
        Self {
            world,
            control,
            stats,
            listener,
            asr,
            heard,
            now_ms: 0,
        }
    }

    fn step(&mut self) {
        let now_s = 1_000 + self.now_ms / 1000;
        self.listener.step(self.now_ms, now_s);
    }

    /// Steps once a second for `seconds`.
    fn run(&mut self, seconds: i64) {
        for _ in 0..seconds {
            self.step();
            self.now_ms += 1000;
        }
    }

    /// The app uses the mic from now; returns once the streams are open (at 1002 s).
    fn start_call(&mut self, bundle_id: &str) {
        self.world.call_app(4242, bundle_id);
        self.run(2);
        self.step();
    }

    fn hang_up(&mut self) {
        self.world.hang_up();
        self.run(7);
    }

    /// Ends the session and returns what was heard, once the ASR thread has caught up.
    fn finish(self) -> Vec<Utterance> {
        drop(self.listener);
        self.asr.join().expect("asr thread");
        let mut heard = self.heard.lock().expect("heard").clone();
        heard.sort_by_key(|u| (u.source.as_str(), u.ts_start));
        heard
    }
}

fn feed(session: &Session, mic: Vec<super::Chunk>, system: Vec<super::Chunk>) {
    for chunk in mic {
        session.world.mic(chunk);
    }
    for chunk in system {
        session.world.system(chunk);
    }
}

#[test]
fn a_call_app_opens_both_streams_after_two_seconds_and_nothing_loads_before_speech() {
    let mut session = Session::new();
    session.world.call_app(4242, "us.zoom.xos");
    session.step();
    assert_eq!(session.world.streams(), 0, "not yet: a beep is not a call");
    session.now_ms = 1000;
    session.step();
    assert_eq!(session.world.streams(), 0);
    session.now_ms = 2000;
    session.step();
    assert_eq!(session.world.streams(), 2);
    assert!(session.stats.listening());
    assert_eq!(*session.world.system_pids.lock().expect("pids"), [4242]);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(
        session.world.loads.load(Ordering::Relaxed),
        0,
        "no speech, no model"
    );
}

#[test]
fn apps_that_are_not_calls_and_sextant_itself_open_nothing() {
    let mut session = Session::new();
    session.world.call_app(4242, "com.example.Dictation");
    session.run(10);
    assert_eq!(session.world.opened.load(Ordering::Relaxed), 0);
    session
        .world
        .call_app(std::process::id() as i32, "us.zoom.xos");
    session.run(10);
    assert_eq!(session.world.opened.load(Ordering::Relaxed), 0);
    session
        .control
        .update(|s| s.call_apps = vec!["com.example.Dictation".into()]);
    session.world.call_app(4243, "com.example.Dictation");
    session.run(3);
    assert_eq!(session.world.streams(), 2, "marked by the user");
}

#[test]
fn the_mic_is_me_and_the_system_sound_is_them() {
    let mut session = Session::new();
    session.start_call("us.zoom.xos");
    let mut mic = blocks(3.0, 48_000, 2, true);
    mic.extend(blocks(1.5, 48_000, 2, false));
    let mut system = blocks(2.0, 44_100, 2, true);
    system.extend(blocks(1.0, 44_100, 2, false));
    feed(&session, mic, system);
    session.now_ms += 20;
    session.step();
    session.hang_up();
    assert_eq!(session.world.streams(), 0, "the call ended");
    assert!(!session.stats.listening());
    let heard = session.finish();
    assert_eq!(heard.len(), 2, "{heard:?}");
    let (mic, system) = (&heard[0], &heard[1]);
    assert_eq!((mic.source, system.source), (Speaker::Mic, Speaker::System));
    assert_eq!(mic.ts_start, 1002);
    assert_eq!(mic.ts_end, 1005);
    assert_eq!((system.ts_start, system.ts_end), (1002, 1004));
}

#[test]
fn speech_still_running_when_the_call_ends_is_kept() {
    let mut session = Session::new();
    session.start_call("com.microsoft.teams2");
    feed(&session, blocks(4.0, 16_000, 1, true), Vec::new());
    session.now_ms += 20;
    session.step();
    session.hang_up();
    let heard = session.finish();
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].source, Speaker::Mic);
}

#[test]
fn the_model_loads_once_for_the_first_speech() {
    let mut session = Session::new();
    session.start_call("us.zoom.xos");
    for _ in 0..3 {
        feed(&session, blocks(2.0, 16_000, 1, true), Vec::new());
        feed(&session, blocks(1.0, 16_000, 1, false), Vec::new());
        session.now_ms += 20;
        session.step();
    }
    let world = session.world.clone();
    let heard = session.finish();
    assert_eq!(heard.len(), 3);
    assert_eq!(world.loads.load(Ordering::Relaxed), 1);
}

#[test]
fn switching_audio_off_closes_the_streams_at_once_and_drops_what_was_not_heard() {
    let mut session = Session::new();
    session.start_call("us.zoom.xos");
    feed(
        &session,
        blocks(3.0, 16_000, 1, true),
        blocks(3.0, 16_000, 1, true),
    );
    session.control.update(|s| s.live.audio = false);
    session.now_ms += 20;
    session.step();
    assert_eq!(session.world.streams(), 0);
    assert!(!session.stats.listening());
    session.now_ms += 5000;
    session.step();
    assert_eq!(session.world.streams(), 0, "and they stay closed");
    assert!(session.finish().is_empty());
}

#[test]
fn a_pause_closes_the_streams_and_the_end_of_it_lets_the_call_start_again() {
    let mut session = Session::new();
    session.start_call("us.zoom.xos");
    session.control.pause_until(1_000 + 60);
    session.now_ms += 1000;
    session.step();
    assert_eq!(session.world.streams(), 0);
    session.control.resume();
    session.run(4);
    assert_eq!(session.world.streams(), 2);
}

#[test]
fn a_tap_that_cannot_open_still_leaves_the_mic() {
    let mut session = Session::new();
    session.world.fail_system.store(true, Ordering::Relaxed);
    session.start_call("us.zoom.xos");
    assert_eq!(session.world.streams(), 1);
    assert_eq!(session.stats.stream_errors.load(Ordering::Relaxed), 1);
    feed(&session, blocks(3.0, 16_000, 1, true), Vec::new());
    session.now_ms += 20;
    session.step();
    session.hang_up();
    let heard = session.finish();
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].source, Speaker::Mic);
}

fn data_dir() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "sextant-audio-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("data dir");
    dir
}

/// Done when: no audio in the data folder after a session. A call of a minute and a half, both
/// sides talking, goes through the real pipeline into the real store; afterwards the folder holds
/// only the database, which holds the sentences and is far smaller than the audio.
#[test]
fn no_audio_is_in_the_data_folder_after_a_session() {
    let dir = data_dir();
    let db = TempDb {
        path: dir.join("sextant.db"),
    };
    let mut store = db.open().expect("store");
    let ingest = Arc::new(Mutex::new(Ingest::new()));
    let mut session = Session::with_sink({
        let ingest = ingest.clone();
        move |utterance| {
            ingest
                .lock()
                .expect("ingest")
                .push(Item::Utterance(utterance.clone()), Instant::now());
        }
    });
    session.start_call("us.zoom.xos");
    let mut fed_seconds = 0.0;
    for _ in 0..9 {
        feed(
            &session,
            talk(5.0, 2.0, 48_000, 2),
            talk(3.0, 4.0, 44_100, 2),
        );
        fed_seconds += 14.0;
        session.now_ms += 20;
        session.step();
        thread::sleep(Duration::from_millis(50));
    }
    session.hang_up();
    let heard = session.finish();
    assert!(fed_seconds > 100.0);
    assert_eq!(heard.len(), 18);
    ingest
        .lock()
        .expect("ingest")
        .flush(&mut store, 2_000)
        .expect("flush");
    assert_eq!(store.counts().expect("counts").utterances, 18);
    let found = store
        .search("sentence", None, None, None, 100, 0)
        .expect("search");
    assert_eq!(found.len(), 18);
    drop(store);

    let mut total = 0;
    for entry in fs::read_dir(&dir).expect("read dir") {
        let entry = entry.expect("entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(name.starts_with("sextant.db"), "unexpected file {name}");
        let bytes = fs::read(entry.path()).expect("read");
        for magic in [&b"RIFF"[..], b"fLaC", b"OggS", b"caff", b"ID3"] {
            assert!(!bytes.starts_with(magic), "{name} starts like audio");
        }
        total += bytes.len();
    }
    println!("data folder: {total} bytes after {fed_seconds} s of call audio");
    // 100 s of 16 kHz audio is 3.2 MB even as 16-bit; the sentences are a few kilobytes.
    assert!(total < 1 << 20, "{total} bytes in the data folder");
    let _ = fs::remove_dir_all(dir);
}

fn wait_for(what: &str, within: Duration, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < within, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

fn start_pipeline(world: &Arc<World>, control: &Arc<Control>) -> Pipeline {
    let sink: Sink = Arc::new(|_, _| {});
    Pipeline::start(
        FakeDevices(world.clone()),
        new_vad,
        FakeSpeech(world.clone()),
        control.clone(),
        sink,
    )
    .expect("pipeline")
}

fn closed_after(world: &World, since: Instant) -> Duration {
    world
        .closed_at
        .lock()
        .expect("closed")
        .expect("a stream was closed")
        .duration_since(since)
}

/// Done when: switching audio off stops streams within 1 s.
#[test]
fn switching_audio_off_in_the_settings_stops_the_streams_within_a_second() {
    let world = Arc::new(World::default());
    let control = Arc::new(Control::new(settings()));
    world.call_app(4242, "us.zoom.xos");
    let pipeline = start_pipeline(&world, &control);
    wait_for("the call to open", Duration::from_secs(6), || {
        world.streams() == 2
    });

    let switched = Instant::now();
    control.update(|s| s.live.audio = false);
    wait_for("the streams to close", Duration::from_secs(5), || {
        world.streams() == 0
    });
    let took = closed_after(&world, switched);
    println!("audio off -> streams closed in {took:?}");
    assert!(took < Duration::from_secs(1), "{took:?}");
    pipeline.stop();
}

/// Done when: stopping stops the streams, the taps and the ASR thread within 1 s, even while a
/// segment is being transcribed. The thread finishes that segment and drops the rest.
#[test]
fn stopping_closes_the_streams_within_a_second_even_while_the_model_is_busy() {
    let world = Arc::new(World::default());
    *world.asr_delay.lock().expect("delay") = Duration::from_millis(1500);
    let control = Arc::new(Control::new(settings()));
    world.call_app(4242, "us.zoom.xos");
    let pipeline = start_pipeline(&world, &control);
    wait_for("the call to open", Duration::from_secs(6), || {
        world.streams() == 2
    });
    for chunk in talk(2.0, 1.0, 16_000, 1) {
        world.mic(chunk);
    }
    wait_for("the model to be busy", Duration::from_secs(5), || {
        world.busy.load(Ordering::Relaxed)
    });

    let stopped = Instant::now();
    let stopper = thread::spawn(move || pipeline.stop());
    wait_for("the streams to close", Duration::from_secs(5), || {
        world.streams() == 0
    });
    let took = closed_after(&world, stopped);
    println!("stop -> streams closed in {took:?} while the model was busy");
    assert!(took < Duration::from_secs(1), "{took:?}");
    stopper.join().expect("stop");
    assert!(
        world.asr_gone_at.lock().expect("gone").is_some(),
        "the ASR thread ended"
    );
}

#[test]
fn stopping_an_idle_pipeline_ends_both_threads_within_a_second() {
    let world = Arc::new(World::default());
    let control = Arc::new(Control::new(settings()));
    world.call_app(4242, "us.zoom.xos");
    let pipeline = start_pipeline(&world, &control);
    wait_for("the call to open", Duration::from_secs(6), || {
        world.streams() == 2
    });

    let stopped = Instant::now();
    pipeline.stop();
    let took = stopped.elapsed();
    println!("stop -> both threads joined in {took:?}");
    assert!(took < Duration::from_secs(1), "{took:?}");
    assert_eq!(world.streams(), 0);
    assert!(world.asr_gone_at.lock().expect("gone").is_some());
}
