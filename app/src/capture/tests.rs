//! Gate tests (plan.md §13 M3): a scripted session on the fake platform. Every gated source must
//! write zero rows and never reach the capture call. Each case has a twin with the gate open,
//! which has to write rows, so a gate test cannot pass by capturing nothing at all.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sextant_core::fixtures::TempDb;
use sextant_core::{Counts, EventKind, Item, Settings, Store};

use super::fake::{Calls, Fake, FakeOcr, Scene};
use super::ingest::Ingest;
use super::pipeline::{Capturer, Control, Queue, Sink, recognize_next};
use super::sensors::{App, Tab, WindowText};
use super::stats::Stats;

const START_S: i64 = 1_700_000_000;
const STEP_MS: i64 = 250;

fn app(name: &str, bundle_id: &str, pid: i32) -> App {
    App {
        name: name.into(),
        bundle_id: bundle_id.into(),
        pid,
    }
}

fn mail() -> App {
    app("Mail", "com.apple.mail", 100)
}

fn chrome() -> App {
    app("Google Chrome", "com.google.Chrome", 200)
}

impl Scene {
    fn in_app(mut self, app: App, title: &str) -> Self {
        self.app = Some(app);
        self.text = WindowText {
            title: title.into(),
            doc: String::new(),
        };
        self
    }

    fn showing(mut self, picture: u64, lines: &[&str]) -> Self {
        self.picture = picture;
        self.lines = lines.iter().map(|line| line.to_string()).collect();
        self
    }

    fn tab(mut self, url: &str, private: bool) -> Self {
        self.tab = Some(Tab {
            url: url.into(),
            private,
        });
        self
    }
}

/// A user working in Mail with nothing special going on.
fn working(at_ms: i64) -> Scene {
    Scene::new(at_ms)
        .in_app(mail(), "Inbox")
        .showing(1, &["Quarterly report", "Invoice 4711 attached"])
}

struct Session {
    clock: Arc<AtomicI64>,
    calls: Arc<Mutex<Calls>>,
    capturer: Capturer<Fake>,
    queue: Arc<Queue<Vec<String>>>,
    control: Arc<Control>,
    stats: Arc<Stats>,
    ingest: Arc<Mutex<Ingest>>,
    sink: Sink,
    ocr: FakeOcr,
    now_ms: i64,
}

impl Session {
    fn new(script: Vec<Scene>, settings: Settings) -> Self {
        let clock = Arc::new(AtomicI64::new(0));
        let (fake, calls) = Fake::new(script, clock.clone());
        let control = Arc::new(Control::new(settings));
        let stats = Arc::new(Stats::default());
        let queue = Arc::new(Queue::new(2));
        let ingest = Arc::new(Mutex::new(Ingest::new()));
        let sink: Sink = {
            let ingest = ingest.clone();
            Arc::new(move |item: Item, taken: Instant| {
                if let Ok(mut ingest) = ingest.lock() {
                    ingest.push(item, taken);
                }
            })
        };
        let capturer = Capturer::new(
            fake,
            control.clone(),
            stats.clone(),
            queue.clone(),
            sink.clone(),
        );
        Self {
            clock,
            calls,
            capturer,
            queue,
            control,
            stats,
            ingest,
            sink,
            ocr: FakeOcr,
            now_ms: 0,
        }
    }

    fn now_s(&self) -> i64 {
        START_S + self.now_ms / 1000
    }

    /// Polls every 250 ms until `until_ms` of script time, running OCR after each poll.
    fn run(&mut self, until_ms: i64) {
        while self.now_ms < until_ms {
            self.clock.store(self.now_ms, Ordering::Relaxed);
            self.capturer.step(self.now_ms, self.now_s());
            let now_s = self.now_s();
            while recognize_next(
                &self.queue,
                &mut self.ocr,
                &self.control,
                &self.stats,
                &self.sink,
                now_s,
            ) {}
            self.now_ms += STEP_MS;
        }
    }

    fn calls(&self) -> Calls {
        self.calls.lock().expect("calls").clone()
    }

    /// Writes what was captured into a fresh database.
    fn database(&self) -> (Store, TempDb) {
        let db = TempDb::new("capture");
        let mut store = db.open().expect("opens");
        self.ingest
            .lock()
            .expect("ingest")
            .flush(&mut store, START_S + 86_400)
            .expect("flushes");
        (store, db)
    }
}

fn session(script: Vec<Scene>) -> Session {
    Session::new(script, Settings::default())
}

fn nothing_stored(counts: &Counts) -> bool {
    counts.frames == 0 && counts.lines == 0 && counts.windows == 0 && counts.utterances == 0
}

struct Gated {
    name: &'static str,
    script: Vec<Scene>,
    settings: Settings,
    /// Pause capture from the start.
    paused: bool,
}

fn gated_cases() -> Vec<Gated> {
    let open = Settings::default();
    let tweak = |change: fn(&mut Settings)| {
        let mut settings = Settings::default();
        change(&mut settings);
        settings
    };
    let case = |name, script: Vec<Scene>, settings: Settings, paused| Gated {
        name,
        script,
        settings,
        paused,
    };
    vec![
        case(
            "excluded password manager",
            vec![working(0).in_app(app("1Password 8", "com.1password.1password", 300), "Vault")],
            open.clone(),
            false,
        ),
        case(
            "excluded messenger",
            vec![working(0).in_app(app("Messages", "com.apple.MobileSMS", 301), "Anna")],
            open.clone(),
            false,
        ),
        case(
            "excluded app added by the user",
            vec![working(0).in_app(mail(), "Inbox")],
            tweak(|s| s.excluded_apps.push("Mail".into())),
            false,
        ),
        case(
            "private Chromium window",
            vec![
                working(0)
                    .in_app(chrome(), "News")
                    .tab("https://example.com/a", true),
            ],
            open.clone(),
            false,
        ),
        case(
            "private Arc window",
            vec![
                working(0)
                    .in_app(app("Arc", "company.thebrowser.Browser", 201), "News")
                    .tab("https://example.com/a", true),
            ],
            open.clone(),
            false,
        ),
        case(
            "browser that does not say whether the window is private",
            vec![{
                let mut scene = working(0).in_app(chrome(), "News");
                scene.tab = None;
                scene
            }],
            open.clone(),
            false,
        ),
        case(
            "Safari without the opt-in",
            vec![working(0).in_app(app("Safari", "com.apple.Safari", 202), "News")],
            open.clone(),
            false,
        ),
        case(
            "secure input",
            vec![{
                let mut scene = working(0);
                scene.secure_input = true;
                scene
            }],
            open.clone(),
            false,
        ),
        case("paused", vec![working(0)], open.clone(), true),
        case(
            "OCR source off",
            vec![working(0)],
            tweak(|s| s.live.ocr = false),
            false,
        ),
        case(
            "every live source off",
            vec![working(0)],
            tweak(|s| {
                s.live.metadata = false;
                s.live.ocr = false;
                s.live.copy = false;
            }),
            false,
        ),
        case(
            "idle for two minutes",
            vec![{
                let mut scene = working(0);
                scene.idle_s = 120.0;
                scene
            }],
            open.clone(),
            false,
        ),
        case(
            "screen locked",
            vec![{
                let mut scene = working(0);
                scene.locked = true;
                scene
            }],
            open.clone(),
            false,
        ),
        case(
            "display asleep",
            vec![{
                let mut scene = working(0);
                scene.asleep = true;
                scene
            }],
            open,
            false,
        ),
    ]
}

#[test]
fn gated_sources_write_zero_rows_and_never_reach_the_capture_call() {
    for case in gated_cases() {
        let mut run = Session::new(case.script, case.settings);
        if case.paused {
            run.control.pause_until(START_S + 3600);
        }
        run.run(35_000);

        let calls = run.calls();
        assert!(
            calls.captured.is_empty(),
            "{}: capture was called for {:?}",
            case.name,
            calls.captured
        );
        let (store, _db) = run.database();
        let counts = store.counts().expect("counts");
        assert!(nothing_stored(&counts), "{}: {counts:?}", case.name);
        assert_eq!(
            store.event_count(EventKind::Copy).expect("count"),
            0,
            "{}",
            case.name
        );
        assert_eq!(run.queue.len(), 0, "{}", case.name);
    }
}

#[test]
fn gated_sources_leave_no_trace_in_events_either() {
    // These gates close before any window is read, so not even a focus event exists.
    for name in [
        "excluded password manager",
        "excluded messenger",
        "private Chromium window",
        "browser that does not say whether the window is private",
        "Safari without the opt-in",
        "secure input",
        "paused",
        "every live source off",
        "idle for two minutes",
        "screen locked",
        "display asleep",
    ] {
        let case = gated_cases()
            .into_iter()
            .find(|case| case.name == name)
            .expect("case exists");
        let mut run = Session::new(case.script, case.settings);
        if case.paused {
            run.control.pause_until(START_S + 3600);
        }
        run.run(35_000);
        let (store, _db) = run.database();
        assert_eq!(
            store.event_count(EventKind::Focus).expect("count"),
            0,
            "{name}"
        );
        assert_eq!(store.counts().expect("counts").events, 0, "{name}");
    }
}

#[test]
fn the_os_is_not_read_beyond_what_the_gate_needs() {
    let calls_of = |name: &str| {
        let case = gated_cases()
            .into_iter()
            .find(|case| case.name == name)
            .expect("case exists");
        let mut run = Session::new(case.script, case.settings);
        if case.paused {
            run.control.pause_until(START_S + 3600);
        }
        run.run(35_000);
        run.calls()
    };

    let paused = calls_of("paused");
    assert_eq!(
        paused.presence + paused.focused_app,
        0,
        "paused means no OS call at all"
    );
    let off = calls_of("every live source off");
    assert_eq!(
        off.presence + off.focused_app + off.reads_beyond_triggers(),
        0
    );

    for name in ["idle for two minutes", "screen locked", "display asleep"] {
        assert_eq!(calls_of(name).reads_beyond_triggers(), 0, "{name}");
    }
    for name in ["excluded password manager", "excluded messenger"] {
        let calls = calls_of(name);
        assert_eq!(
            calls.window_text + calls.browser_tab + calls.secure_input,
            0,
            "{name}"
        );
    }
    let private = calls_of("private Chromium window");
    assert_eq!(
        private.window_text, 0,
        "the title of a private window is never read"
    );
    let secure = calls_of("secure input");
    assert_eq!(secure.window_text + secure.browser_tab, 0);
}

#[test]
fn ungated_windows_write_rows() {
    let mut run = session(vec![working(0)]);
    run.run(35_000);
    assert_eq!(
        run.calls().captured.first().map(String::as_str),
        Some("Mail")
    );
    let (store, _db) = run.database();
    let counts = store.counts().expect("counts");
    assert_eq!((counts.frames, counts.windows), (1, 1), "{counts:?}");
    assert_eq!(counts.lines, 2);
    assert_eq!(store.event_count(EventKind::Focus).expect("count"), 1);
    let hits = store
        .search("invoice 4711", None, None, None, 10, 0)
        .expect("search");
    assert_eq!(hits.len(), 1);
}

#[test]
fn each_open_gate_twin_writes_rows() {
    // The same scenes as the gated cases with the gate open: a private flag of false, an app
    // that is not excluded, input not secure, not paused, sources on.
    let chrome_open = working(0)
        .in_app(chrome(), "News")
        .tab("https://example.com/a?x=1", false);
    let mut opted_in = Settings::default();
    opted_in.live.opaque_browsers = true;
    let cases = [
        (vec![chrome_open], Settings::default()),
        (
            vec![working(0).in_app(app("Safari", "com.apple.Safari", 202), "News")],
            opted_in,
        ),
        (vec![working(0)], Settings::default()),
    ];
    for (script, settings) in cases {
        let mut run = Session::new(script, settings);
        run.run(35_000);
        let (store, _db) = run.database();
        assert_eq!(store.counts().expect("counts").frames, 1);
    }
}

#[test]
fn a_scripted_session_keeps_only_what_the_gates_allow() {
    let secure = {
        let mut scene = working(40_000).showing(5, &["Zulu secure field"]);
        scene.secure_input = true;
        scene
    };
    let locked = {
        let mut scene = working(55_000).showing(6, &["Yankee lock screen"]);
        scene.locked = true;
        scene
    };
    let script = vec![
        working(0).showing(1, &["Alpha invoice"]),
        working(10_000)
            .in_app(app("1Password 8", "com.1password.1password", 300), "Vault")
            .showing(2, &["Bravo vault secret"]),
        working(20_000)
            .in_app(chrome(), "Pricing")
            .tab("https://example.com/pricing?token=abc", false)
            .showing(3, &["Charlie pricing"]),
        working(30_000)
            .in_app(chrome(), "Private")
            .tab("https://example.com/private", true)
            .showing(4, &["Delta private browsing"]),
        secure,
        locked,
        working(70_000).showing(7, &["Xray back at work"]),
    ];
    let mut run = session(script);
    run.run(90_000);

    let (store, _db) = run.database();
    let found = |word: &str| {
        store
            .search(word, None, None, None, 10, 0)
            .expect("search")
            .len()
    };
    for kept in ["alpha", "charlie", "xray"] {
        assert_eq!(found(kept), 1, "{kept} was on an allowed window");
    }
    for dropped in ["bravo", "delta", "zulu", "yankee"] {
        assert_eq!(found(dropped), 0, "{dropped} was on a gated window");
    }
    let counts = store.counts().expect("counts");
    assert_eq!((counts.frames, counts.windows), (3, 2), "{counts:?}");

    let captured = run.calls().captured;
    assert!(
        !captured.iter().any(|name| name == "1Password 8"),
        "{captured:?}"
    );
    assert_eq!(
        captured
            .iter()
            .filter(|name| *name == "Google Chrome")
            .count(),
        1,
        "only the normal Chrome window was captured"
    );
    // Each gate that closed after an open window ended its episode, with no content.
    assert_eq!(store.event_count(EventKind::Idle).expect("count"), 2);
    assert_eq!(store.event_count(EventKind::Focus).expect("count"), 3);
}

#[test]
fn pausing_stops_capture_and_resuming_starts_it_again() {
    let mut run = session(vec![
        working(0).showing(1, &["Alpha before"]),
        working(20_000).showing(2, &["Bravo while paused"]),
        working(30_000).showing(3, &["Charlie after"]),
    ]);
    run.run(15_000);
    run.control.pause_until(START_S + 35);
    run.run(34_000);
    assert!(run.control.paused_until(run.now_s()).is_some());
    run.run(60_000);

    let (store, _db) = run.database();
    let found = |word: &str| {
        store
            .search(word, None, None, None, 10, 0)
            .expect("search")
            .len()
    };
    assert_eq!(
        (found("alpha"), found("bravo"), found("charlie")),
        (1, 0, 1)
    );
    assert_eq!(store.event_count(EventKind::Pause).expect("count"), 1);
}

#[test]
fn copy_events_name_the_app_and_respect_the_gates() {
    let mut copied_in_mail = working(0);
    copied_in_mail.clipboard = 1;
    let mut copied_again = working(5_000);
    copied_again.clipboard = 2;
    let mut copied_in_vault =
        working(10_000).in_app(app("1Password 8", "com.1password.1password", 300), "Vault");
    copied_in_vault.clipboard = 3;
    let mut copied_in_private = working(15_000)
        .in_app(chrome(), "News")
        .tab("https://example.com", true);
    copied_in_private.clipboard = 4;

    let mut run = session(vec![
        working(0),
        copied_in_mail,
        copied_again,
        copied_in_vault,
        copied_in_private,
    ]);
    run.run(25_000);
    let (store, _db) = run.database();
    // The first poll only reads the count. Changes 1 to 2 happen in Mail; the vault and the
    // private window copy nothing into the database.
    assert_eq!(store.event_count(EventKind::Copy).expect("count"), 1);
}

#[test]
fn the_queue_keeps_the_newest_frames_when_ocr_falls_behind() {
    let script: Vec<Scene> = (0..5)
        .map(|i| {
            working(i * 11_000)
                .in_app(
                    app(
                        &format!("App{i}"),
                        &format!("com.example.app{i}"),
                        400 + i as i32,
                    ),
                    "Window",
                )
                .showing(10 + i as u64, &[&format!("line {i}")])
        })
        .collect();
    let mut run = session(script);
    // OCR never runs: poll without draining.
    while run.now_ms < 55_000 {
        run.clock.store(run.now_ms, Ordering::Relaxed);
        let now_s = run.now_s();
        run.capturer.step(run.now_ms, now_s);
        run.now_ms += STEP_MS;
    }
    assert_eq!(run.queue.len(), 2);
    assert_eq!(Stats::get(&run.stats.dropped), 3);
}

#[test]
fn nearly_equal_frames_skip_ocr() {
    let mut run = session(vec![working(0)]);
    run.run(65_000);
    assert_eq!(Stats::get(&run.stats.ocr), 1);
    assert!(Stats::get(&run.stats.skipped) >= 5);
    let (store, _db) = run.database();
    assert_eq!(store.counts().expect("counts").frames, 1);
}

#[test]
fn low_confidence_lines_never_reach_the_database() {
    use sextant_core::capture::RawLine;
    struct Unsure;
    impl super::sensors::Ocr<Vec<String>> for Unsure {
        fn recognize(&mut self, image: &Vec<String>) -> anyhow::Result<Vec<RawLine>> {
            Ok(image
                .iter()
                .enumerate()
                .map(|(i, text)| RawLine {
                    text: text.clone(),
                    confidence: if i == 0 { 0.9 } else { 0.2 },
                    x: 0.1,
                    y: 0.1 * i as f32,
                    width: 0.5,
                    height: 0.02,
                })
                .collect())
        }
    }
    let mut run = session(vec![
        working(0).showing(9, &["Kept line", "Guess one", "Guess two"]),
    ]);
    let mut unsure = Unsure;
    while run.now_ms < 2_000 {
        run.clock.store(run.now_ms, Ordering::Relaxed);
        let now_s = run.now_s();
        run.capturer.step(run.now_ms, now_s);
        while recognize_next(
            &run.queue,
            &mut unsure,
            &run.control,
            &run.stats,
            &run.sink,
            now_s,
        ) {}
        run.now_ms += STEP_MS;
    }
    let (store, _db) = run.database();
    assert_eq!(store.counts().expect("counts").lines, 1);
}

#[test]
fn a_failed_capture_is_counted_and_nothing_is_written() {
    use super::sensors::{Sensors, Shot};
    struct Broken(Fake);
    impl Sensors for Broken {
        type Image = Vec<String>;
        fn presence(&mut self) -> super::sensors::Presence {
            self.0.presence()
        }
        fn clipboard_count(&mut self) -> i64 {
            self.0.clipboard_count()
        }
        fn focused_app(&mut self) -> Option<App> {
            self.0.focused_app()
        }
        fn secure_input(&mut self) -> bool {
            self.0.secure_input()
        }
        fn window_text(&mut self, app: &App) -> WindowText {
            self.0.window_text(app)
        }
        fn browser_tab(
            &mut self,
            app: &App,
            browser: sextant_core::capture::Browser,
        ) -> anyhow::Result<Tab> {
            self.0.browser_tab(app, browser)
        }
        fn capture(&mut self, _app: &App) -> anyhow::Result<Shot<Vec<String>>> {
            anyhow::bail!("no screen recording permission")
        }
    }
    let clock = Arc::new(AtomicI64::new(0));
    let (fake, _calls) = Fake::new(vec![working(0)], clock);
    let stats = Arc::new(Stats::default());
    let sink: Sink = Arc::new(|_, _| {});
    let mut capturer = Capturer::new(
        Broken(fake),
        Arc::new(Control::new(Settings::default())),
        stats.clone(),
        Arc::new(Queue::new(2)),
        sink,
    );
    for t in (0..2_000).step_by(250) {
        capturer.step(t, START_S + t / 1000);
    }
    assert_eq!(Stats::get(&stats.capture_errors), 1);
    assert_eq!(Stats::get(&stats.captured), 0);
}

#[test]
fn the_threads_deliver_a_frame_and_stop() {
    use super::pipeline::Pipeline;
    let clock = Arc::new(AtomicI64::new(0));
    let (fake, _calls) = Fake::new(vec![working(0)], clock);
    let delivered = Arc::new(Mutex::new(Vec::new()));
    let sink: Sink = {
        let delivered = delivered.clone();
        Arc::new(move |item: Item, _| {
            if let (Item::Frame(frame), Ok(mut delivered)) = (item, delivered.lock()) {
                delivered.push(frame.lines.len());
            }
        })
    };
    let pipeline = Pipeline::start(
        fake,
        FakeOcr,
        Arc::new(Control::new(Settings::default())),
        sink,
    )
    .expect("starts");
    let deadline = Instant::now() + Duration::from_secs(5);
    while delivered.lock().expect("lock").is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    pipeline.stop();
    assert_eq!(*delivered.lock().expect("lock"), [2]);
}
