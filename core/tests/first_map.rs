//! Plan.md §13 M2 budget: the first map of a 90-day profile takes under 60 s.
//! Run with `cargo test -p sextant-core --release --test first_map -- --nocapture`.

use std::time::{Duration, Instant};

use sextant_core::backfill::{self, CalendarRecord, ReminderRecord, Stage};
use sextant_core::fixtures::*;
use sextant_core::profile_fixture::{FakePlatform, ProfileSpec, profile};
use sextant_core::{Item, Sources, Store, Writer};

const FIRST_MAP: Duration = Duration::from_secs(60);
const DAY: i64 = 86_400;

struct Map {
    tasks: usize,
    events: usize,
    took: Duration,
}

fn first_map(store: &mut Store, weeks: u32, sources: &Sources, calendar_denied: bool) -> Map {
    let profile = profile(&ProfileSpec {
        first_monday: WEEK_START,
        weeks,
        noise_visits_per_day: 150,
    })
    .expect("test setup");
    let platform = FakePlatform {
        profile,
        calendar_denied,
    };
    let now = WEEK_START + (i64::from(weeks) * 7 - 1) * DAY;

    let started = Instant::now();
    let collected = backfill::collect(&platform, sources, &["/code".to_string()], now, |_| {});
    let events = collected.events.len();
    let mut writer = Writer::new();
    for event in collected.events {
        writer.push(Item::Event(event));
    }
    writer.flush(store, now).expect("test setup");
    let stats = store.refresh(local(now)).expect("test setup");
    Map {
        tasks: stats.tasks,
        events,
        took: started.elapsed(),
    }
}

#[test]
fn a_backfill_only_map_finds_the_invoice_tasks() {
    let db = TempDb::new("first-map");
    let mut store = db.open().expect("test setup");
    let map = first_map(&mut store, 4, &Sources::default(), false);

    assert!(
        map.tasks >= 15,
        "{} tasks from {} events",
        map.tasks,
        map.events
    );
}

#[test]
fn a_denied_calendar_does_not_stop_the_other_sources() {
    let profile = profile(&ProfileSpec {
        first_monday: WEEK_START,
        weeks: 2,
        noise_visits_per_day: 20,
    })
    .expect("test setup");
    let platform = FakePlatform {
        profile,
        calendar_denied: true,
    };
    let mut stages = Vec::new();
    let collected = backfill::collect(
        &platform,
        &Sources::default(),
        &["/code".to_string()],
        WEEK_START + 13 * DAY,
        |stage| stages.push(stage),
    );

    assert_eq!(
        stages,
        [Stage::Browsers, Stage::Calendar, Stage::Git, Stage::Files]
    );
    let calendar = &collected.reports[1];
    assert_eq!(
        (calendar.stage, calendar.events, calendar.errors.len()),
        (Stage::Calendar, 0, 2)
    );
    assert!(collected.reports[0].events > 0 && collected.reports[0].errors.is_empty());
    assert!(collected.reports[2].events > 0 && collected.reports[3].events > 0);
}

#[test]
fn unticked_sources_are_not_read() {
    let profile = profile(&ProfileSpec {
        first_monday: WEEK_START,
        weeks: 1,
        noise_visits_per_day: 10,
    })
    .expect("test setup");
    let platform = FakePlatform {
        profile,
        calendar_denied: false,
    };
    let sources = Sources {
        browsers: false,
        safari: false,
        calendar: false,
        git: false,
        files: true,
    };
    let collected = backfill::collect(
        &platform,
        &sources,
        &["/code".to_string()],
        WEEK_START + 6 * DAY,
        |_| {},
    );
    assert_eq!(collected.reports.len(), 1);
    assert_eq!(collected.reports[0].stage, Stage::Files);
}

#[test]
fn a_second_backfill_replaces_the_first() {
    let db = TempDb::new("first-map-twice");
    let mut store = db.open().expect("test setup");
    let first = first_map(&mut store, 2, &Sources::default(), false);
    let removed = store.clear_backfill().expect("test setup");
    assert_eq!(removed, first.events);
}

#[test]
#[cfg_attr(debug_assertions, ignore = "budget tests run in release mode")]
fn first_map_of_a_90_day_profile_takes_under_60_s() {
    let db = TempDb::new("budget-first-map");
    let mut store = db.open().expect("test setup");
    let map = first_map(&mut store, 13, &Sources::default(), false);
    println!(
        "first map over 90 days: {:?} for {} backfill events, {} tasks (budget {FIRST_MAP:?})",
        map.took, map.events, map.tasks
    );
    assert!(map.tasks > 0);
    assert!(
        map.took < FIRST_MAP,
        "first map {:?} is over the {FIRST_MAP:?} budget",
        map.took
    );
}

/// A 2-week profile whose calendar and reminders also hold things after `now`.
fn upcoming_profile(now: i64) -> sextant_core::profile_fixture::Profile {
    let mut profile = profile(&ProfileSpec {
        first_monday: WEEK_START,
        weeks: 2,
        noise_visits_per_day: 20,
    })
    .expect("test setup");
    let meeting = |title: &str, start: i64| CalendarRecord {
        title: title.into(),
        start,
        end: start + 3600,
        all_day: false,
    };
    profile
        .calendar
        .push(meeting("Planning soon", now + 3 * DAY));
    profile
        .calendar
        .push(meeting("Offsite later", now + 13 * DAY));
    profile
        .calendar
        .push(meeting("Beyond the horizon", now + 20 * DAY));
    profile.reminders.push(ReminderRecord {
        title: "Call Anna about the offer".into(),
        created: None,
        due: Some(now + 5 * DAY),
        done: false,
    });
    profile
}

#[test]
fn the_next_14_days_of_calendar_and_reminders_are_kept() {
    let now = WEEK_START + 13 * DAY;
    let platform = FakePlatform {
        profile: upcoming_profile(now),
        calendar_denied: false,
    };
    let collected = backfill::collect(&platform, &Sources::default(), &[], now, |_| {});
    let mut upcoming: Vec<(i64, &str)> = collected
        .events
        .iter()
        .filter(|e| e.ts > now)
        .map(|e| (e.ts, e.kind.as_str()))
        .collect();
    upcoming.sort();
    assert_eq!(
        upcoming,
        [
            (now + 3 * DAY, "calendar"),
            (now + 5 * DAY, "reminder"),
            (now + 13 * DAY, "calendar"),
        ]
    );
}

#[test]
fn upcoming_events_are_not_worked_time() {
    let now = WEEK_START + 13 * DAY;
    let run = |with_upcoming: bool| {
        let db = TempDb::new("upcoming");
        let mut store = db.open().expect("test setup");
        let mut profile = upcoming_profile(now);
        if !with_upcoming {
            profile.calendar.retain(|r| r.start <= now);
            profile.reminders.retain(|r| r.due.is_none_or(|d| d <= now));
        }
        let platform = FakePlatform {
            profile,
            calendar_denied: false,
        };
        let collected = backfill::collect(&platform, &Sources::default(), &[], now, |_| {});
        let mut writer = Writer::new();
        for event in collected.events {
            writer.push(Item::Event(event));
        }
        writer.flush(&mut store, now).expect("test setup");
        store.refresh(local(now)).expect("test setup")
    };
    let (with, without) = (run(true), run(false));

    assert_eq!(
        with.episodes, without.episodes,
        "no episode from the future"
    );
    assert_eq!(with.tasks, without.tasks);
}
