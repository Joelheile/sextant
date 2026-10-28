//! Synthetic fixtures with a fixed date and timezone, shared by core tests, MCP tests and `sextant bench`.
//!
//! The week is Monday 2026-03-02 to Friday 2026-03-06 at UTC+01:00. It holds 5 invoices that each
//! run through Mail, Excel and lexoffice over 2 to 4 days, a daily calendar, Slack and Linear routine,
//! two meetings, promises, a reminder and noise between. No images or audio.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::{Value, json};

use crate::error::Result;
use crate::model::{Event, EventKind, Frame, Item, OcrLine, Source, Speaker, Utterance, Window};
use crate::redact::Secret;
use crate::store::{Key, Store};
use crate::time::DAY;
use crate::writer::{Flushed, Writer};

/// UTC+01:00.
pub const UTC_OFFSET_S: i32 = 3600;
/// Monday 2026-03-02 00:00:00 +01:00.
pub const WEEK_START: i64 = 1_772_406_000;
/// Saturday 2026-03-07 08:00:00 +01:00, the `now` for the week fixture.
pub const WEEK_NOW: i64 = WEEK_START + 5 * DAY + 8 * 3600;
pub const TEST_KEY: Key = [7; 32];
pub const TEST_SECRET: Secret = [5; 16];

pub const INVOICE_IDS: [&str; 5] = [
    "INV-2026-0412",
    "RE-2026-0415",
    "INV-2026-0418",
    "RE-2026-0420",
    "INV-2026-0423",
];

#[derive(Default)]
pub struct Fixture {
    pub events: Vec<Event>,
    pub frames: Vec<Frame>,
    pub utterances: Vec<Utterance>,
}

pub struct Spec {
    /// Local midnight of a Monday.
    pub first_monday: i64,
    pub weeks: u32,
    pub noise_per_day: u32,
    /// One OCR frame per episode.
    pub frames: bool,
}

/// The standard fixture week.
pub fn week() -> Fixture {
    generate(&Spec {
        first_monday: WEEK_START,
        weeks: 1,
        noise_per_day: 20,
        frames: true,
    })
}

/// `weeks` Monday-to-Friday weeks starting at `spec.first_monday`. Invoice numbers advance by 20 a week.
pub fn generate(spec: &Spec) -> Fixture {
    let mut fixture = Fixture::default();
    let mut rng = Rng(0x5e47_a417 ^ u64::from(spec.weeks));
    for week in 0..spec.weeks {
        let monday = spec.first_monday + i64::from(week) * 7 * DAY;
        let mut planned = Vec::new();
        invoices(&mut fixture, monday, week, &mut planned);
        for day in 0..5 {
            routine(&mut fixture, &mut planned, monday + day * DAY, day);
        }
        meetings(&mut fixture, monday, &mut planned);
        promises(&mut fixture, monday, week);
        for day in 0..5 {
            noise(
                &mut fixture,
                monday + day * DAY,
                spec.noise_per_day,
                &planned,
                &mut rng,
            );
        }
    }
    if !spec.frames {
        fixture.frames.clear();
    }
    fixture.events.sort_by_key(|e| e.ts);
    fixture.frames.sort_by_key(|f| f.ts);
    fixture
}

/// Pushes everything through a `Writer`, in the order a live session would, and flushes once.
pub fn load(store: &mut Store, fixture: &Fixture, now: i64) -> Result<Flushed> {
    let mut writer = Writer::new();
    for event in &fixture.events {
        writer.push(Item::Event(event.clone()));
    }
    for frame in &fixture.frames {
        writer.push(Item::Frame(frame.clone()));
    }
    for utterance in &fixture.utterances {
        writer.push(Item::Utterance(utterance.clone()));
    }
    writer.flush(store, now)
}

/// A unique temp database path that cleans up after itself.
pub struct TempDb {
    pub path: PathBuf,
}

impl TempDb {
    pub fn new(label: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let name = format!(
            "sextant-{}-{}-{label}.db",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        Self {
            path: std::env::temp_dir().join(name),
        }
    }

    pub fn open(&self) -> Result<Store> {
        let mut store = Store::open(&self.path, &TEST_KEY, &TEST_SECRET)?;
        store.set_utc_offset(UTC_OFFSET_S);
        Ok(store)
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let mut name = self.path.clone().into_os_string();
            name.push(suffix);
            let _ = std::fs::remove_file(name);
        }
    }
}

/// xorshift64*: deterministic, no dependency.
pub struct Rng(pub u64);

impl Rng {
    pub fn step(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.step() % n.max(1)
    }
}

type Planned = Vec<(i64, i64)>;

pub(crate) fn at(midnight: i64, hour: i64, minute: i64) -> i64 {
    midnight + hour * 3600 + minute * 60
}

fn focus(ts: i64, app: &str, title: &str, doc: &str, url: &str, text: &str) -> Event {
    let mut json = json!({"app": app, "title": title});
    for (key, value) in [("doc", doc), ("url", url), ("text", text)] {
        if !value.is_empty() {
            json[key] = Value::String(value.into());
        }
    }
    Event {
        ts,
        kind: EventKind::Focus,
        source: Source::Live,
        json,
    }
}

fn idle(ts: i64) -> Event {
    Event {
        ts,
        kind: EventKind::Idle,
        source: Source::Live,
        json: json!({}),
    }
}

fn copy(ts: i64, app: &str) -> Event {
    Event {
        ts,
        kind: EventKind::Copy,
        source: Source::Live,
        json: json!({"app": app}),
    }
}

/// One episode: focus, optional copy event 20 s before the end, idle, and one OCR frame.
#[allow(clippy::too_many_arguments)]
fn episode(
    f: &mut Fixture,
    planned: &mut Planned,
    start: i64,
    seconds: i64,
    window: Window,
    text: &str,
    lines: Vec<String>,
    copied: bool,
) {
    f.events.push(focus(
        start,
        &window.app,
        &window.title,
        &window.doc,
        &window.url,
        text,
    ));
    if copied {
        f.events.push(copy(start + seconds - 20, &window.app));
    }
    f.events.push(idle(start + seconds));
    if !lines.is_empty() {
        let lines = lines
            .into_iter()
            .enumerate()
            .map(|(i, text)| OcrLine {
                text,
                bbox: [4, (i as u8).wrapping_mul(12), 200, 10],
            })
            .collect();
        f.frames.push(Frame {
            ts: start + 5,
            window,
            lines,
        });
    }
    planned.push((start, start + seconds));
}

fn window(app: &str, title: &str, doc: &str, url: &str) -> Window {
    Window {
        app: app.into(),
        bundle_id: String::new(),
        title: title.into(),
        doc: doc.into(),
        url: url.into(),
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Part {
    Mail,
    Excel,
    Lex,
}

pub(crate) type Visit = (i64, i64, i64, Part, i64, bool);

/// Per invoice: number offset, German, then (day, hour, minute, part, seconds, copy-event) visits.
pub(crate) const INVOICES: [(u32, bool, &[Visit]); 5] = [
    (
        412,
        false,
        &[
            (0, 9, 40, Part::Mail, 240, true),
            (0, 9, 46, Part::Excel, 360, false),
            (1, 10, 20, Part::Lex, 420, false),
        ],
    ),
    (
        415,
        true,
        &[
            (0, 11, 5, Part::Mail, 200, false),
            (1, 11, 30, Part::Excel, 330, true),
            (1, 11, 38, Part::Lex, 400, false),
            (2, 15, 0, Part::Lex, 180, false),
        ],
    ),
    (
        418,
        false,
        &[
            (1, 15, 45, Part::Mail, 260, true),
            (1, 15, 51, Part::Excel, 300, false),
            (2, 10, 10, Part::Excel, 150, false),
            (3, 9, 30, Part::Lex, 450, false),
        ],
    ),
    (
        420,
        true,
        &[
            (1, 16, 30, Part::Mail, 180, false),
            (2, 16, 0, Part::Excel, 340, false),
            (3, 11, 20, Part::Excel, 200, false),
            (4, 10, 0, Part::Lex, 410, false),
        ],
    ),
    (
        423,
        false,
        &[
            (2, 13, 0, Part::Mail, 220, false),
            (3, 14, 30, Part::Excel, 350, true),
            (3, 14, 38, Part::Lex, 430, false),
        ],
    ),
];

pub(crate) const CLIENTS: [(&str, &str); 5] = [
    ("Acme GmbH", "€1,250.00"),
    ("Müller AG", "980,00 €"),
    ("Beta Ltd", "€2,400.00"),
    ("Schmidt UG", "1.480,00 €"),
    ("Delta Inc", "€620.00"),
];

fn invoices(f: &mut Fixture, monday: i64, week: u32, planned: &mut Planned) {
    for (index, (number, german, visits)) in INVOICES.iter().enumerate() {
        let id = format!(
            "{}-2026-{:04}",
            if *german { "RE" } else { "INV" },
            number + 20 * week
        );
        let (client, amount) = CLIENTS[index];
        for &(day, hour, minute, part, seconds, copied) in *visits {
            let start = at(monday + day * DAY, hour, minute);
            let (win, text, lines) = match part {
                Part::Mail if *german => (
                    window("Mail", &format!("Rechnung {id} angehängt"), "", ""),
                    String::new(),
                    vec![
                        "Von: Jonas Müller <j.mueller@mueller-ag.de>".to_string(),
                        format!("Betreff: Rechnung {id} angehängt"),
                        format!("Gesamtbetrag {amount} fällig am 20. März 2026"),
                        "Bitte bestätigen Sie die Preise bis Freitag.".to_string(),
                        "Zahlung an IBAN DE89 3704 0044 0532 0130 00".to_string(),
                    ],
                ),
                Part::Mail => (
                    window("Mail", &format!("Invoice {id} attached"), "", ""),
                    String::new(),
                    vec![
                        "From: Anna Keller <anna.keller@acme-gmbh.de>".to_string(),
                        format!("Subject: Invoice {id} attached"),
                        format!(
                            "Please find the invoice for March attached. Total {amount}, due 15 March 2026."
                        ),
                        "Can you confirm the pricing before Friday?".to_string(),
                    ],
                ),
                Part::Excel => (
                    window(
                        "Microsoft Excel",
                        "Invoices 2026.xlsx - Excel",
                        "Invoices 2026.xlsx",
                        "",
                    ),
                    format!("Invoice {id} {client} {amount}"),
                    vec![
                        "Invoices 2026".to_string(),
                        format!("{id} | {client} | {amount} | open"),
                        "Status | Customer | Amount | Number".to_string(),
                    ],
                ),
                Part::Lex => (
                    window(
                        "Google Chrome",
                        "Rechnung bearbeiten - lexoffice",
                        "",
                        &format!("https://app.lexoffice.de/vouchers/{id}/edit?tab=1"),
                    ),
                    String::new(),
                    vec![
                        "Rechnung bearbeiten".to_string(),
                        format!("Rechnungsnummer {id}"),
                        format!("Kunde {client}"),
                        format!("Betrag {amount}"),
                    ],
                ),
            };
            episode(f, planned, start, seconds, win, &text, lines, copied);
        }
    }
}

pub(crate) const SLACK_TEXT: [&str; 5] = [
    "Anna Keller: standup notes are in the doc",
    "Anna Keller: Can we revisit the pricing for the annual plan before Friday?",
    "Jonas: Rechnung für Müller AG ist raus, danke!",
    "Anna Keller: pricing sheet updated, please check",
    "Jonas: Preisliste für Q2 bitte bis Montag prüfen",
];

/// Calendar, Slack and Linear every morning at 09:00, 09:03 and 09:07.
fn routine(f: &mut Fixture, planned: &mut Planned, midnight: i64, day: i64) {
    let slack = vec![SLACK_TEXT[day as usize].to_string(), "#standup".to_string()];
    episode(
        f,
        planned,
        at(midnight, 9, 0),
        120,
        window("Calendar", "Calendar - Today", "", ""),
        "",
        vec!["Today".into(), "Weekly sync at 14:00".into()],
        false,
    );
    episode(
        f,
        planned,
        at(midnight, 9, 3),
        150,
        window("Slack", "Slack | #standup", "", ""),
        "",
        slack,
        false,
    );
    episode(
        f,
        planned,
        at(midnight, 9, 7),
        120,
        window("Linear", "Linear | My issues", "", ""),
        "",
        vec!["My issues".into(), "3 in progress".into()],
        false,
    );
}

fn meetings(f: &mut Fixture, monday: i64, planned: &mut Planned) {
    for (day, hour, minute, minutes) in [(2, 14, 0, 45), (4, 15, 0, 30)] {
        let start = at(monday + day * DAY, hour, minute);
        let end = start + minutes * 60;
        f.events.push(Event {
            ts: start,
            kind: EventKind::Calendar,
            source: Source::Backfill,
            json: json!({"title": "Weekly sync", "end_ts": end}),
        });
        planned.push((start, end));
    }
}

fn promises(f: &mut Fixture, monday: i64, week: u32) {
    let id = |n: u32| format!("INV-2026-{:04}", n + 20 * week);
    let said = |day: i64, hour: i64, minute: i64, source: Speaker, text: String| Utterance {
        ts_start: at(monday + day * DAY, hour, minute),
        ts_end: at(monday + day * DAY, hour, minute) + 6,
        source,
        text,
    };
    f.utterances.extend([
        said(
            0,
            9,
            50,
            Speaker::Mic,
            "I'll send the invoice attached to Acme later.".into(),
        ),
        said(
            0,
            14,
            0,
            Speaker::Mic,
            "Let me look at the pricing sheet for Anna.".into(),
        ),
        said(
            1,
            10,
            31,
            Speaker::System,
            "Anna: I'll check the pricing and get back to you.".into(),
        ),
        said(
            2,
            16,
            0,
            Speaker::Mic,
            format!("I'll send the corrected {} tomorrow.", id(412)),
        ),
        said(
            2,
            16,
            5,
            Speaker::Mic,
            "Ich werde die Rechnung für Müller morgen schicken.".into(),
        ),
        said(
            3,
            11,
            0,
            Speaker::System,
            "Jonas: Die Preisliste ist noch nicht fertig.".into(),
        ),
    ]);
    f.events.push(Event {
        ts: at(monday + 2 * DAY, 13, 10),
        kind: EventKind::Reminder,
        source: Source::Backfill,
        json: json!({"title": format!("Send reminder for {}", id(423)), "due_ts": at(monday + 3 * DAY, 17, 0), "done": false}),
    });
}

/// (app, url, title, doc). `{n}` becomes a random number, which the title template removes again.
const NOISE: [(&str, &str, &str, &str); 24] = [
    (
        "Google Chrome",
        "https://news.ycombinator.com/item?id={n}",
        "Show HN: tiny tool {n} | Hacker News",
        "",
    ),
    (
        "Google Chrome",
        "https://en.wikipedia.org/wiki/Process_mining",
        "Process mining - Wikipedia",
        "",
    ),
    (
        "Google Chrome",
        "https://github.com/acme/app/pull/{n}",
        "Fix flaky test {n} by dev · Pull Request #{n} · acme/app",
        "",
    ),
    (
        "Google Chrome",
        "https://stackoverflow.com/questions/{n}/rust-lifetimes",
        "Rust lifetimes explained - Stack Overflow",
        "",
    ),
    (
        "Google Chrome",
        "https://www.youtube.com/watch?v={n}",
        "Rust conference talk - YouTube",
        "",
    ),
    ("Google Chrome", "https://x.com/home", "Home / X", ""),
    (
        "Google Chrome",
        "https://www.linkedin.com/feed/",
        "Feed | LinkedIn",
        "",
    ),
    (
        "Google Chrome",
        "https://www.figma.com/file/{n}/Landing",
        "Landing page – Figma",
        "",
    ),
    (
        "Google Chrome",
        "https://www.notion.so/Roadmap-{n}",
        "Roadmap – Notion",
        "",
    ),
    (
        "Google Chrome",
        "https://docs.google.com/document/d/{n}/edit",
        "Pricing draft - Google Docs",
        "",
    ),
    (
        "Google Chrome",
        "https://docs.google.com/presentation/d/{n}/edit",
        "Q1 review - Google Slides",
        "",
    ),
    (
        "Google Chrome",
        "https://app.hubspot.com/contacts/{n}",
        "Contacts - HubSpot",
        "",
    ),
    (
        "Google Chrome",
        "https://calendar.google.com/calendar/u/0/r/week",
        "Calendar - Week of March",
        "",
    ),
    (
        "Google Chrome",
        "https://www.heise.de/news/Datenschutz-{n}.html",
        "Datenschutz: neue Leitlinien | heise online",
        "",
    ),
    (
        "Google Chrome",
        "https://www.reddit.com/r/rust/comments/{n}",
        "Ask r/rust: async traits",
        "",
    ),
    ("Discord", "", "Discord | #dev | Rustaceans", ""),
    ("Terminal", "", "cargo test — sextant — 80x24", ""),
    (
        "Code",
        "",
        "main.rs — sextant — Visual Studio Code",
        "main.rs",
    ),
    (
        "Code",
        "",
        "lib.rs — sextant — Visual Studio Code",
        "lib.rs",
    ),
    ("Finder", "", "Downloads", ""),
    ("Spotify", "", "Spotify - Focus Flow", ""),
    ("Keynote", "", "Q1 review.key", "Q1 review.key"),
    ("Pages", "", "Offer.docx", "Offer.docx"),
    ("zoom.us", "", "Zoom Meeting", ""),
];

const NOISE_LINES: [&str; 14] = [
    "Release notes for version 4.2 are available",
    "Neue Leitlinien zum Datenschutz veröffentlicht",
    "Build finished in 41 seconds with 0 warnings",
    "Die Preisliste wird am Freitag aktualisiert",
    "Remember to book the train to Hamburg",
    "Heute 14 Uhr Besprechung im Raum Nord",
    "Quarterly numbers look better than expected",
    "Contact: lena.fischer@example.com",
    "export OPENAI_API_KEY=sk-proj-abcdef0123456789abcdef0123456789",
    "Card ending 4111 1111 1111 1111 expires 09/29",
    "Wie funktioniert async in Rust?",
    "Open pull requests: 12",
    "Meeting notes from last Tuesday",
    "Ticket ACME-77 moved to review",
];

fn noise(f: &mut Fixture, midnight: i64, per_day: u32, planned: &Planned, rng: &mut Rng) {
    if per_day == 0 {
        return;
    }
    let (open, close) = (midnight + 8 * 3600, midnight + 19 * 3600);
    let gap_max = ((close - open) as u64 / u64::from(per_day))
        .saturating_sub(130)
        .max(30);
    let mut cursor = open;
    let mut placed = 0;
    while placed < per_day && cursor < close {
        let seconds = 15 + rng.below(226) as i64;
        let start = cursor + 5 + rng.below(gap_max) as i64;
        let end = start + seconds;
        if let Some((_, planned_end)) = planned
            .iter()
            .find(|(s, e)| start < e + 60 && end + 60 > *s && s / DAY == start / DAY)
        {
            cursor = planned_end + 60;
            continue;
        }
        let (app, url, title, doc) = NOISE[rng.below(NOISE.len() as u64) as usize];
        let n = (1000 + rng.below(9000)).to_string();
        let fill = |text: &str| text.replace("{n}", &n);
        let lines = (0..3)
            .map(|_| NOISE_LINES[rng.below(NOISE_LINES.len() as u64) as usize].to_string())
            .collect();
        let mut sink = Vec::new();
        episode(
            f,
            &mut sink,
            start,
            seconds,
            window(app, &fill(title), doc, &fill(url)),
            "",
            lines,
            rng.below(12) == 0,
        );
        cursor = end;
        placed += 1;
    }
}

/// Search material for the budget check: words and two-word phrases taken from lines that exist.
#[derive(Default)]
pub struct Sample {
    pub words: Vec<String>,
    pub phrases: Vec<String>,
}

/// Fills `store` with `days` days of OCR frames and utterances ending at `now`, in one flush per day.
/// Each day has `frames_per_day` frames of 12 lines. Screens persist: a frame changes about 3 lines.
pub fn fill_frames(
    store: &mut Store,
    now: i64,
    days: u32,
    frames_per_day: u32,
    seed: u64,
) -> Result<Sample> {
    let mut rng = Rng(seed);
    let mut writer = Writer::new();
    let mut sample = Sample::default();
    let vocabulary: Vec<String> = (0..20_000u64).map(word).collect();
    let line = |rng: &mut Rng| -> String {
        let length = 4 + rng.below(7);
        let words: Vec<&str> = (0..length)
            .map(|_| {
                let u = rng.below(1_000_000) as f64 / 1_000_000.0;
                vocabulary[(u * u * u * vocabulary.len() as f64) as usize].as_str()
            })
            .collect();
        words.join(" ")
    };
    let apps = [
        "Google Chrome",
        "Mail",
        "Slack",
        "Code",
        "Microsoft Excel",
        "Terminal",
        "Notion",
        "Safari",
    ];
    let mut page: Vec<String> = Vec::new();
    let mut title = String::new();
    let mut app = apps[0];
    for day in 0..days {
        let midnight = now - i64::from(days - day) * DAY;
        for n in 0..frames_per_day {
            if page.is_empty() || rng.below(15) == 0 {
                page = (0..12).map(|_| line(&mut rng)).collect();
                title = line(&mut rng);
                app = apps[rng.below(apps.len() as u64) as usize];
            }
            for _ in 0..3 {
                let at = rng.below(12) as usize;
                page[at] = line(&mut rng);
            }
            if n % 400 == 7 {
                let words: Vec<&str> = page[0].split(' ').collect();
                sample
                    .words
                    .extend(words.iter().take(1).map(|w| w.to_string()));
                if words.len() >= 2 {
                    sample.phrases.push(format!("{} {}", words[0], words[1]));
                }
            }
            let lines = page
                .iter()
                .enumerate()
                .map(|(i, text)| OcrLine {
                    text: text.clone(),
                    bbox: [0, i as u8 * 12, 200, 10],
                })
                .collect();
            let ts = midnight + 8 * 3600 + i64::from(n) * 15;
            writer.push(Item::Frame(Frame {
                ts,
                window: window(app, &title, "", ""),
                lines,
            }));
        }
        for n in 0..40 {
            let ts = midnight + 9 * 3600 + i64::from(n) * 600;
            let speaker = if n % 2 == 0 {
                Speaker::Mic
            } else {
                Speaker::System
            };
            writer.push(Item::Utterance(Utterance {
                ts_start: ts,
                ts_end: ts + 8,
                source: speaker,
                text: line(&mut rng),
            }));
        }
        writer.flush(store, now)?;
    }
    Ok(sample)
}

/// The `count` most frequent words of `fill_frames`: the slowest searches.
pub fn common_words(count: u64) -> Vec<String> {
    (0..count).map(word).collect()
}

/// Pseudo word from an index: stable, distinct, pronounceable enough to look like text.
fn word(index: u64) -> String {
    const SYLLABLES: [&str; 16] = [
        "ka", "lo", "mi", "ne", "ru", "sa", "to", "vi", "be", "da", "fo", "gu", "he", "jo", "pa",
        "ze",
    ];
    let mut value = index + 17;
    let mut out = String::new();
    while value > 0 {
        out.push_str(SYLLABLES[(value % 16) as usize]);
        value /= 16;
    }
    out
}
