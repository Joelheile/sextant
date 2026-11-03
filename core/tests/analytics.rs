use serde_json::json;
use sextant_core::fixtures::*;
use sextant_core::{Event, EventKind, Item, Local, Source, Speaker, Store, Utterance, Writer};

const HOUR: i64 = 3600;
const DAY: i64 = 86_400;
/// Local 09:00 on the Tuesday of the fixture week.
const MORNING: i64 = WEEK_START + DAY + 9 * HOUR;

fn focus(ts: i64, app: &str, title: &str) -> Item {
    Item::Event(Event {
        ts,
        kind: EventKind::Focus,
        source: Source::Live,
        json: json!({"app": app, "title": title}),
    })
}

fn idle(ts: i64) -> Item {
    Item::Event(Event {
        ts,
        kind: EventKind::Idle,
        source: Source::Live,
        json: json!({}),
    })
}

fn call(start: i64, end: i64, source: Speaker) -> Item {
    Item::Utterance(Utterance {
        ts_start: start,
        ts_end: end,
        source,
        text: "hello".into(),
    })
}

/// Monday: 30 minutes of Mail. Tuesday morning: Mail 10 min, Numbers 30 min, Mail 20 min, a
/// break, then Slack 15 min. A 2 minute call with both sides talking over each other.
fn day_of_work(label: &str) -> (TempDb, Store) {
    let db = TempDb::new(label);
    let mut store = db.open().expect("test setup");
    let m = |minutes: i64| MORNING + minutes * 60;
    let items = [
        focus(MORNING - DAY, "Mail", "Inbox"),
        idle(MORNING - DAY + 30 * 60),
        focus(m(0), "Mail", "Inbox"),
        focus(m(10), "Numbers", "Budget"),
        focus(m(40), "Mail", "Inbox"),
        idle(m(60)),
        focus(m(120), "Slack", "General"),
        idle(m(135)),
        call(m(120), m(121), Speaker::Mic),
        call(m(120) + 30, m(122), Speaker::System),
    ];
    let mut writer = Writer::new();
    for item in items {
        writer.push(item);
    }
    writer.flush(&mut store, m(140)).expect("test setup");
    store.refresh(local(m(140))).expect("test setup");
    (db, store)
}

#[test]
fn today_counts_active_time_apps_and_focus() {
    let (_db, store) = day_of_work("today");
    let now = MORNING + 140 * 60;
    let insights = store.insights(local(now)).expect("insights");

    assert_eq!(insights.active_minutes.round(), 75.0);
    let apps: Vec<(&str, i64)> = insights
        .apps
        .iter()
        .map(|a| (a.app.as_str(), a.minutes.round() as i64))
        .collect();
    assert_eq!(apps, [("Mail", 30), ("Numbers", 30), ("Slack", 15)]);
    assert_eq!(insights.longest_focus_minutes.round(), 30.0);
    assert_eq!(
        insights.switches, 2,
        "Mail to Numbers to Mail; the break ends the run"
    );
    assert_eq!(
        insights.call_minutes, 2.0,
        "overlapping speakers count once"
    );
}

#[test]
fn week_has_one_value_per_day_ending_today() {
    let (_db, store) = day_of_work("week");
    let insights = store.insights(local(MORNING + 140 * 60)).expect("insights");
    assert_eq!(insights.days.len(), 7);
    let rounded: Vec<i64> = insights.days.iter().map(|d| d.round() as i64).collect();
    assert_eq!(rounded[5..], [30, 75]);
    assert!(rounded[..5].iter().all(|&d| d == 0));
}

#[test]
fn insights_splits_days_at_the_offset_it_is_given() {
    let (_db, store) = day_of_work("offset");
    let now = MORNING + 140 * 60;
    let days = |utc_offset_s| {
        let insights = store
            .insights(Local { now, utc_offset_s })
            .expect("insights");
        insights.days[4..]
            .iter()
            .map(|d| d.round() as i64)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        days(UTC_OFFSET_S),
        [0, 30, 75],
        "the work starts at 09:00 local"
    );
    assert_eq!(
        days(-9 * HOUR as i32),
        [30, 60, 15],
        "at 23:00 local the work starts the evening before and midnight falls in the middle of it"
    );
}

#[test]
fn an_empty_day_has_no_switch_rate() {
    let db = TempDb::new("empty");
    let store = db.open().expect("test setup");
    let insights = store.insights(local(MORNING)).expect("insights");
    assert_eq!(insights.active_minutes, 0.0);
    assert!(insights.apps.is_empty());
    assert_eq!(insights.switches_per_hour, None);
    assert_eq!(insights.last_capture, None);
}

#[test]
fn a_morning_before_any_capture_counts_no_lines_today() {
    let db = TempDb::new("morning");
    let mut store = db.open().expect("test setup");
    load(&mut store, &week(), WEEK_NOW).expect("test setup");
    store.refresh(local(WEEK_NOW)).expect("test setup");
    let insights = store.insights(local(WEEK_NOW)).expect("insights");
    assert_eq!(
        insights.lines, 0,
        "Saturday morning: nothing captured yet today"
    );
    assert!(insights.last_capture.is_some_and(|ts| ts < WEEK_NOW));
}
