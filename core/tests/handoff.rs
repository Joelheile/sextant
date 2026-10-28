//! Done when: a reported handoff shows hours before and after (plan.md §9, §13 M5).

use sextant_core::fixtures::*;
use sextant_core::{Event, EventKind, MIN_DAYS, Store, Workflow};

const DAY: i64 = 86_400;
const INVOICE_APPS: [&str; 2] = ["Mail", "Microsoft Excel"];
const SECOND_MONDAY: i64 = WEEK_START + 7 * DAY;
const HANDOFF_TS: i64 = SECOND_MONDAY + 8 * 3600;

fn is_invoice_work(event: &Event) -> bool {
    let text = |key: &str| event.json[key].as_str().unwrap_or_default().to_owned();
    matches!(event.kind, EventKind::Focus | EventKind::Copy)
        && (INVOICE_APPS.contains(&text("app").as_str()) || text("url").contains("lexoffice"))
}

/// Two weeks of work where the invoices stop at the second Monday. The routine goes on.
fn handed_off_fixture() -> (Vec<Event>, Vec<Event>) {
    let fixture = generate(&Spec {
        first_monday: WEEK_START,
        weeks: 2,
        noise_per_day: 20,
        frames: false,
    });
    fixture
        .events
        .into_iter()
        .filter(|e| e.ts < SECOND_MONDAY || !is_invoice_work(e))
        .partition(|e| e.ts < HANDOFF_TS)
}

fn invoice(store: &Store) -> Workflow {
    store
        .workflows()
        .expect("test setup")
        .into_iter()
        .find(|w| w.basis == "cases")
        .expect("test setup")
}

fn handed_off(label: &str) -> (TempDb, Store, i64, Vec<Event>) {
    let db = TempDb::new(label);
    let mut store = db.open().expect("test setup");
    let (before, after) = handed_off_fixture();
    load(
        &mut store,
        &Fixture {
            events: before,
            ..Fixture::default()
        },
        HANDOFF_TS,
    )
    .expect("test setup");
    store.refresh(HANDOFF_TS).expect("test setup");
    let id = invoice(&store).id;
    store
        .record_handoff(id, HANDOFF_TS, "started", "Claude takes over the invoices")
        .expect("test setup");
    (db, store, id, after)
}

fn measured(store: &mut Store, after: &[Event], now: i64) -> sextant_core::Measurement {
    let fixture = Fixture {
        events: after.iter().filter(|e| e.ts < now).cloned().collect(),
        ..Fixture::default()
    };
    load(store, &fixture, now).expect("test setup");
    store.refresh(now).expect("test setup");
    let mut results = store.results(now).expect("test setup");
    assert_eq!(results.len(), 1);
    results.remove(0)
}

#[test]
fn a_handoff_shows_hours_before_and_after_once_three_days_exist_on_each_side() {
    let (_db, mut store, id, after) = handed_off("handoff");

    let wednesday = SECOND_MONDAY + 2 * DAY + 8 * 3600;
    let measuring = measured(&mut store, &after, wednesday);
    assert_eq!(measuring.workflow_id, id);
    assert_eq!(measuring.name, "Mail > Microsoft Excel > app.lexoffice.de");
    assert!(measuring.days_before >= MIN_DAYS && measuring.hours_before.is_some());
    assert_eq!((measuring.days_after, measuring.hours_after), (1, None));
    assert_eq!((measuring.progress(), measuring.saved()), (1, None));

    let friday = SECOND_MONDAY + 4 * DAY + 8 * 3600;
    let done = measured(&mut store, &after, friday);
    assert_eq!(done.days_after, MIN_DAYS);
    let (before, after) = (
        done.hours_before.expect("measured"),
        done.hours_after.expect("measured"),
    );
    assert!(before > 0.2, "the invoice week took time: {before}");
    assert_eq!(after, 0.0, "no invoice work after the handoff");
    assert_eq!(done.saved(), Some(before));
    assert_eq!(
        (done.status.as_str(), done.progress()),
        ("started", MIN_DAYS)
    );
}

#[test]
fn reporting_twice_counts_one_handoff_with_the_latest_status() {
    let (_db, store, id, _after) = handed_off("twice");
    store
        .record_handoff(id, HANDOFF_TS + 3600, "done", "All five ran")
        .expect("test setup");
    let results = store.results(HANDOFF_TS + 2 * DAY).expect("test setup");
    assert_eq!(results.len(), 1);
    assert_eq!(
        (
            results[0].ts,
            results[0].status.as_str(),
            results[0].note.as_str()
        ),
        (HANDOFF_TS, "done", "All five ran")
    );
    assert!(store.is_handed_off(id).expect("test setup"));
}

#[test]
fn the_report_holds_no_entity_title_or_url() {
    let db = TempDb::new("report");
    let mut store = db.open().expect("test setup");
    let fixture = week();
    load(&mut store, &fixture, WEEK_NOW).expect("test setup");
    store.refresh(WEEK_NOW).expect("test setup");

    let report = store.weekly_report(WEEK_NOW).expect("test setup");
    assert!(!report.top.is_empty() && report.hours_back > 0.0);
    let shown = format!("{report:?}");

    let mut private: Vec<String> = INVOICE_IDS.iter().map(|id| id.to_string()).collect();
    for event in &fixture.events {
        for key in ["title", "doc", "url"] {
            private.extend(event.json[key].as_str().map(str::to_owned));
        }
    }
    for frame in &fixture.frames {
        private.push(frame.window.title.clone());
        private.extend(frame.lines.iter().map(|l| l.text.clone()));
    }
    private.retain(|value| value.len() >= 4);
    assert!(private.len() > 20);
    for value in private {
        assert!(!shown.contains(&value), "report leaks {value:?}: {shown}");
    }
}
