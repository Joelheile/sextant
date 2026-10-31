use serde_json::json;
use sextant_core::fixtures::*;
use sextant_core::{Enjoyment, Event, EventKind, Source, Speaker, Store, Utterance, Workflow};

const DAY: i64 = 86_400;
/// Same lexoffice step as the invoice workflow, but no entity: the flinch joins no case.
const LEX: &str = "https://app.lexoffice.de/vouchers/88231/edit?tab=1";

fn at(day: i64, hour: i64, minute: i64) -> i64 {
    WEEK_START + day * DAY + hour * 3600 + minute * 60
}

fn event(ts: i64, kind: EventKind, json: serde_json::Value) -> Event {
    Event {
        ts,
        kind,
        source: Source::Live,
        json,
    }
}

fn focus(ts: i64, app: &str, title: &str, url: &str) -> Event {
    event(
        ts,
        EventKind::Focus,
        json!({"app": app, "title": title, "url": url}),
    )
}

fn idle(ts: i64) -> Event {
    event(ts, EventKind::Idle, json!({}))
}

fn refreshed_with(label: &str, extra: impl FnOnce(&mut Fixture)) -> (TempDb, Store) {
    let mut fixture = week();
    extra(&mut fixture);
    fixture.events.sort_by_key(|e| e.ts);
    let db = TempDb::new(label);
    let mut store = db.open().expect("test setup");
    load(&mut store, &fixture, WEEK_NOW).expect("test setup");
    store.refresh(local(WEEK_NOW)).expect("test setup");
    (db, store)
}

fn reason<'a>(w: &'a Workflow, start: &str) -> Option<&'a str> {
    w.reasons
        .iter()
        .map(String::as_str)
        .find(|r| r.starts_with(start))
}

/// Opens lexoffice for 15 s, then YouTube for 4 min, at 20:00 on `days`.
fn flinch(fixture: &mut Fixture, days: &[i64]) {
    for &day in days {
        let start = at(day, 20, 0);
        fixture.events.extend([
            focus(
                start,
                "Google Chrome",
                "Rechnung bearbeiten - lexoffice",
                LEX,
            ),
            focus(
                start + 15,
                "Google Chrome",
                "Cat videos - YouTube",
                "https://www.youtube.com/watch?v=abc",
            ),
            idle(start + 255),
        ]);
    }
}

#[test]
fn a_complaint_in_my_own_speech_raises_desire_for_its_workflow() {
    let (_a, before) = refreshed_with("complaint-before", |_| {});
    let (_b, after) = refreshed_with("complaint-after", |f| {
        f.utterances.push(Utterance {
            ts_start: at(1, 11, 0),
            ts_end: at(1, 11, 0) + 8,
            source: Speaker::Mic,
            text: "Every week I have to type INV-2026-0412 into lexoffice by hand, so annoying"
                .into(),
        });
    });
    let (before, after) = (invoice(&before), invoice(&after));
    assert_eq!(before.desire, Enjoyment::Unknown.desire());
    assert_eq!(after.desire, Enjoyment::Dislike.desire());
    assert_eq!(
        after.enjoyment,
        Enjoyment::Unknown,
        "still asked, not assumed"
    );
    assert!(after.benefit > before.benefit);
    let line = reason(&after, "desire").expect("desire reason");
    assert!(line.contains("complained about it 1 times"), "{line}");
}

#[test]
fn the_other_speaker_complaining_does_not_count() {
    let (_db, store) = refreshed_with("complaint-system", |f| {
        f.utterances.push(Utterance {
            ts_start: at(1, 11, 0),
            ts_end: at(1, 11, 0) + 8,
            source: Speaker::System,
            text: "Typing INV-2026-0412 by hand is so annoying".into(),
        });
    });
    assert_eq!(invoice(&store).desire, Enjoyment::Unknown.desire());
}

#[test]
fn fleeing_to_media_right_after_opening_work_counts_as_procrastination() {
    let (_a, before) = refreshed_with("flinch-before", |_| {});
    let (_b, after) = refreshed_with("flinch-after", |f| flinch(f, &[1, 3]));
    let (before, after) = (invoice(&before), invoice(&after));
    assert!(
        (after.procrastination - before.procrastination - 0.2).abs() < 1e-9,
        "0.1 per flinch day: {} then {}",
        before.procrastination,
        after.procrastination
    );
    let line = reason(&after, "procrastination").expect("procrastination reason");
    assert!(line.contains("2 flinches on 2 days"), "{line}");
}

#[test]
fn one_flinch_day_is_counted_once_and_a_long_open_is_not_a_flinch() {
    let (_a, once) = refreshed_with("flinch-once", |f| flinch(f, &[2]));
    let (_b, long) = refreshed_with("flinch-long", |f| {
        let start = at(2, 20, 0);
        f.events.extend([
            focus(
                start,
                "Google Chrome",
                "Rechnung bearbeiten - lexoffice",
                LEX,
            ),
            focus(
                start + 90,
                "Google Chrome",
                "Cat videos - YouTube",
                "https://www.youtube.com/watch?v=abc",
            ),
            idle(start + 300),
        ]);
    });
    let line = |s: &Store| reason(&invoice(s), "procrastination").map(str::to_string);
    assert!(line(&once).is_some_and(|l| l.contains("1 flinches on 1 days")));
    assert!(line(&long).is_some_and(|l| l.contains("0 flinches on 0 days")));
}

#[test]
fn chat_cutting_into_focused_work_is_reported_with_time_away() {
    let (_db, store) = refreshed_with("interrupt", |f| {
        let start = at(4, 16, 0);
        f.events.extend([
            focus(
                start,
                "Google Chrome",
                "Rechnung bearbeiten - lexoffice",
                LEX,
            ),
            focus(start + 300, "Slack", "Slack | #general", ""),
            focus(start + 360, "Linear", "Linear - Inbox", ""),
            focus(
                start + 600,
                "Google Chrome",
                "Rechnung bearbeiten - lexoffice",
                LEX,
            ),
            idle(start + 900),
        ]);
    });
    let w = invoice(&store);
    let line = reason(&w, "interrupted").expect("interruption reason");
    assert!(line.contains("1 times by Slack"), "{line}");
    assert!(line.contains("median 5 min away"), "{line}");
    assert!(line.contains("0 not picked up"), "{line}");
}

#[test]
fn a_value_that_shows_up_in_the_next_app_with_nothing_copied_is_retyped() {
    let (_db, store) = refreshed_with("retype", |f| {
        let start = at(3, 17, 0);
        let id = "INV-2026-0499";
        f.events.extend([
            focus(start, "Mail", &format!("Invoice {id} attached"), ""),
            event(
                start + 120,
                EventKind::Focus,
                json!({"app": "Microsoft Excel", "title": "Invoices 2026.xlsx - Excel",
                       "doc": "Invoices 2026.xlsx", "text": format!("Invoice {id} Acme GmbH")}),
            ),
            focus(
                start + 300,
                "Google Chrome",
                "Rechnung bearbeiten - lexoffice",
                &format!("https://app.lexoffice.de/vouchers/{id}/edit?tab=1"),
            ),
            idle(start + 480),
        ]);
    });
    let detail = store
        .workflow_detail(invoice(&store).id)
        .expect("test setup")
        .expect("test setup");
    let retyped: usize = detail.copy_points.iter().map(|p| p.retypes).sum();
    assert_eq!(retyped, 2, "{:#?}", detail.copy_points);
    let lines: Vec<_> = detail
        .workflow
        .reasons
        .iter()
        .filter(|r| r.starts_with("retyped: 1 times"))
        .collect();
    assert_eq!(lines.len(), 2, "{:#?}", detail.workflow.reasons);
    let skill = store
        .skill_md(detail.workflow.id, UTC_OFFSET_S)
        .expect("test setup");
    assert!(skill.contains("typed again 1 times"), "{skill}");
}
