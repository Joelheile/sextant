use std::io::Read;

use sextant_core::fixtures::*;
use sextant_core::{Error, Frame, HitSource, Item, OcrLine, Store, Window, Writer};

fn loaded(label: &str) -> (TempDb, Store) {
    let db = TempDb::new(label);
    let mut store = db.open().expect("test setup");
    load(&mut store, &week(), WEEK_NOW).expect("test setup");
    (db, store)
}

#[test]
fn database_is_encrypted_and_versioned() {
    let (db, store) = loaded("encrypted");
    assert_eq!(store.schema_version().expect("test setup"), 1);
    drop(store);
    let mut header = [0u8; 16];
    std::fs::File::open(&db.path)
        .expect("test setup")
        .read_exact(&mut header)
        .expect("test setup");
    assert_ne!(&header, b"SQLite format 3\0");
    assert!(matches!(
        Store::open(&db.path, &[9; 32]),
        Err(Error::BadKey)
    ));
    assert_eq!(
        Store::open(&db.path, &TEST_KEY)
            .expect("test setup")
            .schema_version()
            .expect("test setup"),
        1
    );
}

#[test]
fn search_finds_english_and_german_text() {
    let (_db, store) = loaded("search");
    let english = store
        .search("pricing before friday", None, None, None, 10, 0)
        .expect("test setup");
    assert!(
        english
            .iter()
            .any(|h| h.app == "Mail" && h.source == HitSource::Screen)
    );
    let german = store
        .search("Rechnungsnummer", None, None, None, 10, 0)
        .expect("test setup");
    assert!(
        german
            .iter()
            .all(|h| h.app == "Google Chrome" && h.url.starts_with("app.lexoffice.de/vouchers/"))
    );
    assert_eq!(german.len(), 5);
    assert_eq!(
        store
            .search("muller", None, None, None, 50, 0)
            .expect("test setup")
            .len(),
        store
            .search("Müller", None, None, None, 50, 0)
            .expect("test setup")
            .len()
    );
    let id = store
        .search("INV-2026-0412", None, None, Some("Microsoft Excel"), 10, 0)
        .expect("test setup");
    assert_eq!(id.len(), 1);
    assert!(id[0].text.contains("INV-2026-0412"));
}

#[test]
fn search_covers_speech_filters_by_time_and_pages() {
    let (_db, store) = loaded("speech");
    let spoken = store
        .search("invoice", None, None, None, 50, 0)
        .expect("test setup");
    assert!(spoken.iter().any(|h| h.source == HitSource::Mic));
    assert!(
        spoken.windows(2).all(|w| w[0].ts >= w[1].ts),
        "newest first"
    );
    let first_day = store
        .search(
            "invoice",
            Some(WEEK_START),
            Some(WEEK_START + 86_400),
            None,
            50,
            0,
        )
        .expect("test setup");
    assert!(first_day.iter().all(|h| h.ts < WEEK_START + 86_400));
    let paged = store
        .search("invoice", None, None, None, 2, 1)
        .expect("test setup");
    assert_eq!(paged, spoken[1..3]);
    assert!(
        store
            .search("\"; DROP TABLE lines; --", None, None, None, 10, 0)
            .expect("test setup")
            .is_empty()
    );
    assert!(
        store
            .search("   ", None, None, None, 10, 0)
            .expect("test setup")
            .is_empty()
    );
}

#[test]
fn secrets_never_reach_the_database() {
    let (_db, store) = loaded("redacted");
    for secret in [
        "lena.fischer",
        "sk-proj",
        "abcdef0123456789",
        "4111",
        "DE89",
        "0532",
        "acme-gmbh.de",
        "j.mueller",
    ] {
        assert!(
            store
                .search(secret, None, None, None, 10, 0)
                .expect("test setup")
                .is_empty(),
            "{secret} was stored"
        );
    }
    let email = store
        .search("lena.fischer@example.com", None, None, None, 10, 0)
        .expect("test setup");
    assert!(!email.is_empty(), "searching for an email finds its hash");
    assert!(email.iter().all(|h| h.text.contains("<email:")));
}

#[test]
fn identical_frames_are_stored_once_and_lines_are_shared() {
    let db = TempDb::new("dedupe");
    let mut store = db.open().expect("test setup");
    let frame = |ts| Frame {
        ts,
        window: Window {
            app: "Mail".into(),
            title: "Inbox".into(),
            ..Window::default()
        },
        lines: vec![
            OcrLine {
                text: "Quarterly numbers".into(),
                bbox: [0, 0, 10, 10],
            },
            OcrLine {
                text: "Open pull requests".into(),
                bbox: [0, 12, 10, 10],
            },
        ],
    };
    let mut writer = Writer::new();
    for ts in [100, 110, 120] {
        writer.push(Item::Frame(frame(ts)));
    }
    let flushed = writer.flush(&mut store, 1000).expect("test setup");
    assert_eq!(
        (flushed.frames, flushed.duplicate_frames, flushed.new_lines),
        (1, 2, 2)
    );
    writer.push(Item::Frame(frame(130)));
    assert_eq!(
        writer
            .flush(&mut store, 1000)
            .expect("test setup")
            .duplicate_frames,
        1
    );
    let mut other = frame(140);
    other.window.title = "Sent".into();
    writer.push(Item::Frame(other));
    let flushed = writer.flush(&mut store, 1000).expect("test setup");
    assert_eq!((flushed.frames, flushed.new_lines), (1, 0));
}

#[test]
fn future_timestamps_are_clamped_to_now() {
    let db = TempDb::new("clamp");
    let mut store = db.open().expect("test setup");
    let mut writer = Writer::new();
    writer.push(Item::Utterance(sextant_core::Utterance {
        ts_start: 5000,
        ts_end: 5010,
        source: sextant_core::Speaker::Mic,
        text: "later".into(),
    }));
    writer.flush(&mut store, 1000).expect("test setup");
    assert_eq!(
        store
            .search("later", None, None, None, 5, 0)
            .expect("test setup")[0]
            .ts,
        1000
    );
}

#[test]
fn retention_removes_old_rows_and_unreferenced_lines() {
    let db = TempDb::new("retention");
    let mut store = db.open().expect("test setup");
    load(
        &mut store,
        &generate(&Spec {
            first_monday: WEEK_START,
            weeks: 1,
            noise_per_day: 5,
            frames: true,
        }),
        WEEK_NOW,
    )
    .expect("test setup");
    let later = WEEK_START + 40 * 86_400;
    let mut writer = Writer::new();
    writer.push(Item::Utterance(sextant_core::Utterance {
        ts_start: later,
        ts_end: later + 5,
        source: sextant_core::Speaker::Mic,
        text: "fresh words".into(),
    }));
    writer.flush(&mut store, later + 10).expect("test setup");
    let gone = store.retention(later + 10, 30).expect("test setup");
    assert!(
        gone.frames > 0 && gone.events > 0 && gone.lines > 0 && gone.windows > 0,
        "{gone:?}"
    );
    assert_eq!(gone.utterances, 6);
    assert!(
        store
            .search("pricing", None, None, None, 10, 0)
            .expect("test setup")
            .is_empty()
    );
    assert_eq!(
        store
            .search("fresh", None, None, None, 10, 0)
            .expect("test setup")
            .len(),
        1
    );
    assert_eq!(
        store.retention(later + 10, 30).expect("test setup"),
        Default::default()
    );
}

#[test]
fn a_reader_sees_writes_while_the_writer_stays_open() {
    let (_db, mut store) = loaded("reader");
    let reader = store.reader().expect("test setup");
    let before = reader
        .search("later", None, None, None, 5, 0)
        .expect("test setup")
        .len();
    let mut writer = Writer::new();
    writer.push(Item::Utterance(sextant_core::Utterance {
        ts_start: WEEK_NOW - 60,
        ts_end: WEEK_NOW - 50,
        source: sextant_core::Speaker::System,
        text: "later again".into(),
    }));
    writer.flush(&mut store, WEEK_NOW).expect("test setup");
    assert_eq!(
        reader
            .search("later", None, None, None, 5, 0)
            .expect("test setup")
            .len(),
        before + 1
    );
}
