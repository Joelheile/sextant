use sextant_core::fixtures::*;
use sextant_core::{Store, ValueType};

fn refreshed_actions(label: &str) -> (TempDb, Store) {
    let (fixture, _) = actions_week();
    let db = TempDb::new(label);
    let mut store = db.open().expect("test setup");
    load(&mut store, &fixture, WEEK_NOW).expect("test setup");
    store.refresh(local(WEEK_NOW)).expect("test setup");
    (db, store)
}

/// The earliest task with this templated label. The first invoice task is INV-2026-0412.
fn task_id(store: &Store, label: &str) -> i64 {
    store
        .tasks()
        .expect("test setup")
        .into_iter()
        .find(|t| t.label == label)
        .expect("test setup")
        .id
}

#[test]
fn an_invoice_task_lists_its_actions_in_time_order() {
    let (_db, store) = refreshed_actions("task-actions");
    let actions = store
        .task_actions(task_id(&store, "inv #"))
        .expect("test setup");
    let symbols: Vec<_> = actions.iter().map(|a| a.symbol.as_str()).collect();
    assert_eq!(
        symbols,
        [
            "Mail · AXTextField · to",
            "Mail · AXTextField · subject",
            "Mail · AXButton · send",
            "Microsoft Excel · AXTextField · amount",
            "Microsoft Excel · AXButton · save",
            "Google Chrome · AXTextField · betrag",
            "Google Chrome · AXButton · speichern",
        ]
    );
    assert_eq!(actions[0].value_type, Some(ValueType::Email));
    assert!(
        actions[0]
            .value
            .as_deref()
            .is_some_and(|v| v.starts_with("<email:")),
        "the address is stored redacted: {:?}",
        actions[0]
    );
    assert_eq!(actions[3].value_type, Some(ValueType::Amount));
    assert_eq!(actions[3].value.as_deref(), Some("€1,250.00"));
    assert!(actions.windows(2).all(|pair| pair[0].ts <= pair[1].ts));
}

#[test]
fn where_time_goes_adds_up_each_task_per_day() {
    let (_db, store) = refreshed_actions("where-time");
    let minutes = store.where_time_goes(UTC_OFFSET_S).expect("test setup");

    let report: Vec<_> = minutes.iter().filter(|m| m.label == "ops #").collect();
    assert_eq!(report.len(), 2, "{report:?}");
    let (tuesday, thursday) = (report[0], report[1]);
    assert!((tuesday.minutes - 15.0).abs() < 1e-9, "{tuesday:?}");
    assert!((thursday.minutes - 10.0).abs() < 1e-9, "{thursday:?}");

    let invoice: f64 = minutes
        .iter()
        .filter(|m| m.task_id == task_id(&store, "inv #"))
        .map(|m| m.minutes)
        .sum();
    assert!(
        (invoice - 17.0).abs() < 1e-9,
        "240 + 360 + 420 seconds: {invoice}"
    );
}

#[test]
fn the_unrelated_chat_stays_out_of_every_task() {
    let (_db, store) = refreshed_actions("task-noise");
    for task in store.tasks().expect("test setup") {
        let actions = store.task_actions(task.id).expect("test setup");
        assert!(
            actions.iter().all(|a| !a.symbol.contains("lunch")),
            "{} holds the chat: {actions:?}",
            task.label
        );
    }
}
