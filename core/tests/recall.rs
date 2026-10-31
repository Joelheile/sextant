use sextant_core::fixtures::*;
use sextant_core::{Frame, Item, OcrLine, Settings, Store, Window, Writer, keywords};

fn frame(ts: i64, app: &str, title: &str, lines: &[&str]) -> Item {
    Item::Frame(Frame {
        ts,
        window: Window {
            app: app.into(),
            bundle_id: String::new(),
            title: title.into(),
            doc: String::new(),
            url: String::new(),
        },
        lines: lines
            .iter()
            .enumerate()
            .map(|(row, text)| OcrLine {
                text: (*text).into(),
                bbox: [0, row as u8 * 10, 200, 8],
            })
            .collect(),
    })
}

fn store_with(label: &str, items: Vec<Item>) -> (TempDb, Store) {
    let db = TempDb::new(label);
    let mut store = db.open().expect("test setup");
    let mut writer = Writer::new();
    for item in items {
        writer.push(item);
    }
    writer.flush(&mut store, WEEK_NOW).expect("test setup");
    (db, store)
}

#[test]
fn keywords_drop_question_words_and_keep_names_topics_and_numbers() {
    assert_eq!(
        keywords("What was I writing to Anna about the pricing?"),
        ["anna", "pricing"]
    );
    assert_eq!(
        keywords("Was habe ich gestern Jonas zu Rechnung 4711 geschrieben"),
        ["jonas", "rechnung", "4711"]
    );
}

#[test]
fn the_draft_whose_title_names_the_person_ranks_first() {
    let (_db, store) = store_with(
        "recall-draft",
        vec![
            frame(
                WEEK_NOW - 7200,
                "Mail",
                "Re: Offer - Anna Berg",
                &[
                    "Hi Anna,",
                    "the new pricing starts at 40 per seat",
                    "Best, Joel",
                ],
            ),
            frame(
                WEEK_NOW - 3600,
                "Safari",
                "Pricing pages",
                &["pricing for teams"],
            ),
            frame(WEEK_NOW - 1800, "Notes", "Lunch", &["lunch with anna"]),
        ],
    );
    let found = store
        .recall(
            &Settings::default(),
            &keywords("what was I writing to Anna about pricing"),
            (None, None),
            5,
        )
        .expect("recall");
    assert_eq!(found.len(), 3);
    assert_eq!(found[0].app, "Mail");
    assert_eq!(found[0].lines.len(), 2, "{:?}", found[0].lines);
}

#[test]
fn recall_skips_excluded_apps_and_respects_the_time_window() {
    let (_db, store) = store_with(
        "recall-gates",
        vec![
            frame(WEEK_NOW - 9 * 86_400, "Mail", "Anna", &["pricing draft"]),
            frame(WEEK_NOW - 3600, "1Password", "Anna", &["pricing vault"]),
        ],
    );
    let found = store
        .recall(
            &Settings::default(),
            &["pricing".to_string()],
            (Some(WEEK_NOW - 86_400), None),
            5,
        )
        .expect("recall");
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn hits_five_minutes_apart_form_one_session_in_search_and_recall() {
    let (_db, store) = store_with(
        "recall-gap",
        vec![
            frame(WEEK_NOW - 3600, "Mail", "Offer", &["pricing draft"]),
            frame(WEEK_NOW - 3300, "Mail", "Offer", &["anna draft"]),
        ],
    );
    let settings = Settings::default();
    let searched = store
        .search_sessions(&settings, "draft", (None, None), None, 10, 0)
        .expect("search");
    assert_eq!(searched.sessions.len(), 1, "{:?}", searched.sessions);
    let recalled = store
        .recall(
            &settings,
            &["pricing".to_string(), "anna".to_string()],
            (None, None),
            5,
        )
        .expect("recall");
    assert_eq!(recalled.len(), 1, "{recalled:?}");
    assert_eq!(recalled[0].lines.len(), 2);
}
