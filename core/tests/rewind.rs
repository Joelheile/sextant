use sextant_core::fixtures::*;

const DAY: i64 = 86_400;

#[test]
fn turns_come_back_newest_first_one_page_at_a_time() {
    let db = TempDb::new("rewind-pages");
    let mut store = db.open().expect("test setup");
    for (index, question) in ["first", "second", "third"].iter().enumerate() {
        store
            .add_turn(WEEK_NOW + index as i64, question, "")
            .expect("test setup");
    }
    let newest = store.turns(None, 2).expect("test setup");
    let questions: Vec<&str> = newest.iter().map(|turn| turn.question.as_str()).collect();
    assert_eq!(questions, ["third", "second"]);
    let older = store.turns(Some(newest[1].id), 2).expect("test setup");
    assert_eq!(older.len(), 1);
    assert_eq!(older[0].question, "first");
}

#[test]
fn a_turn_is_redacted_before_it_is_stored() {
    let db = TempDb::new("rewind-redact");
    let mut store = db.open().expect("test setup");
    store
        .add_turn(
            WEEK_NOW,
            "mail from anna@example.com",
            "card 4111 1111 1111 1111",
        )
        .expect("test setup");
    let turn = &store.turns(None, 1).expect("test setup")[0];
    assert!(
        !turn.question.contains("anna@example.com"),
        "{}",
        turn.question
    );
    assert!(
        !turn.answer.contains("4111 1111 1111 1111"),
        "{}",
        turn.answer
    );
}

#[test]
fn retention_deletes_old_turns() {
    let db = TempDb::new("rewind-retention");
    let mut store = db.open().expect("test setup");
    store
        .add_turn(WEEK_NOW - 40 * DAY, "old", "")
        .expect("test setup");
    store.add_turn(WEEK_NOW, "new", "").expect("test setup");
    let gone = store.retention(local(WEEK_NOW), 30).expect("test setup");
    assert_eq!(gone.turns, 1);
    let left: Vec<String> = store
        .turns(None, 10)
        .expect("test setup")
        .into_iter()
        .map(|turn| turn.question)
        .collect();
    assert_eq!(left, ["new"]);
}
