//! Done when: corrections change scores on fixtures (plan.md §7.12, §13 M5).

use sextant_core::fixtures::*;
use sextant_core::{Action, Enjoyment, Kind, Store, Target, Workflow};

fn refreshed(label: &str) -> (TempDb, Store) {
    let db = TempDb::new(label);
    let mut store = db.open().expect("test setup");
    load(&mut store, &week(), WEEK_NOW).expect("test setup");
    store.refresh(WEEK_NOW).expect("test setup");
    (db, store)
}

fn invoice(store: &Store) -> Workflow {
    store
        .workflows()
        .expect("test setup")
        .into_iter()
        .find(|w| w.basis == "cases")
        .expect("test setup")
}

/// The invoice workflow after one correction phrase and a refresh, with the workflow before it.
fn corrected(label: &str, phrase: &str) -> (Workflow, Workflow, Store) {
    let (_db, mut store) = refreshed(label);
    let before = invoice(&store);
    store
        .correct(WEEK_NOW, phrase, &[Target::Workflow(before.id)])
        .expect("test setup");
    store.refresh(WEEK_NOW).expect("test setup");
    let after = store
        .workflow(before.id)
        .expect("test setup")
        .expect("workflow stays");
    (before, after, store)
}

#[test]
fn the_baseline_is_an_automatable_friction_workflow() {
    let (_db, store) = refreshed("baseline");
    let w = invoice(&store);
    assert_eq!(
        (w.kind, w.action, w.enjoyment),
        (Kind::Friction, Action::Automate, Enjoyment::Unknown)
    );
    assert_eq!(w.desire, 0.6);
    assert!(w.benefit > 0.0);
}

#[test]
fn hate_and_dislike_raise_the_benefit_in_both_languages() {
    for (label, phrase, enjoyment, desire) in [
        ("hate-en", "I hate this", Enjoyment::Hate, 1.0),
        ("hate-de", "Ich hasse das", Enjoyment::Hate, 1.0),
        ("dislike-en", "I don't like it", Enjoyment::Dislike, 0.9),
        ("dislike-de", "Ich mag das nicht", Enjoyment::Dislike, 0.9),
    ] {
        let (before, after, _store) = corrected(label, phrase);
        assert_eq!(after.enjoyment, enjoyment, "{phrase}");
        assert_eq!(after.desire, desire, "{phrase}");
        assert!(after.benefit > before.benefit, "{phrase}");
        let ratio = after.benefit / before.benefit;
        assert!(
            (ratio - desire / before.desire).abs() < 1e-9,
            "benefit scales with desire: {phrase}"
        );
        assert_eq!(
            (after.kind, after.action, after.potential),
            (before.kind, before.action, before.potential),
            "{phrase}: enjoyment changes the benefit only"
        );
    }
}

#[test]
fn like_lowers_the_benefit_and_love_protects_the_work() {
    let (before, liked, _store) = corrected("like", "I like this");
    assert_eq!(liked.desire, 0.35);
    assert!(liked.benefit < before.benefit);
    assert_eq!(liked.action, Action::Automate);

    for (label, phrase) in [
        ("love-en", "I love this part"),
        ("love-de", "Ich liebe das"),
    ] {
        let (before, loved, store) = corrected(label, phrase);
        assert_eq!(loved.enjoyment, Enjoyment::Love, "{phrase}");
        assert_eq!((loved.desire, loved.benefit), (0.0, 0.0), "{phrase}");
        assert_eq!(loved.action, Action::Protect, "{phrase}");
        assert!(
            store.hours_back().expect("test setup") < before.benefit,
            "{phrase}"
        );
        assert!(
            loved
                .reasons
                .iter()
                .any(|r| r.contains("you love this work")),
            "{:?}",
            loved.reasons
        );
    }
}

#[test]
fn not_automatable_zeroes_the_potential_and_ignores_the_workflow() {
    for (label, phrase) in [
        ("manual-en", "This can't be automated"),
        ("manual-de", "Das lässt sich nicht automatisieren"),
    ] {
        let (before, after, store) = corrected(label, phrase);
        assert!(before.potential > 0.5, "{phrase}");
        assert_eq!(after.potential, 0.0, "{phrase}");
        assert_eq!(after.benefit, 0.0, "{phrase}");
        assert_eq!(after.action, Action::Ignore, "{phrase}");
        assert_eq!(
            after.kind, before.kind,
            "{phrase}: kind does not depend on it"
        );
        assert_eq!(store.hours_back().expect("test setup"), 0.0, "{phrase}");
    }
}

#[test]
fn client_hide_and_rename_change_what_the_list_shows() {
    let (_before, client, _store) = corrected("client", "Das ist für einen Kunden");
    assert!(client.client);

    let (_before, renamed, _store) = corrected("rename", "Rename to Invoice run");
    assert_eq!(renamed.name, "Invoice run");

    let (before, _hidden, store) = corrected("hide", "hide this");
    let listed = store.workflows().expect("test setup");
    assert!(listed.iter().all(|w| w.id != before.id));
}

#[test]
fn the_enjoyment_answer_is_stored_like_a_correction() {
    let (_db, mut store) = refreshed("answer");
    let before = invoice(&store);
    assert!(
        store
            .ask_first()
            .expect("test setup")
            .iter()
            .any(|w| w.id == before.id)
    );
    store
        .answer_enjoyment(WEEK_NOW, Target::Workflow(before.id), Enjoyment::Hate)
        .expect("test setup");
    store.refresh(WEEK_NOW).expect("test setup");
    let after = invoice(&store);
    assert_eq!((after.enjoyment, after.desire), (Enjoyment::Hate, 1.0));
    assert!(
        store
            .ask_first()
            .expect("test setup")
            .iter()
            .all(|w| w.id != before.id),
        "answered workflows are not asked again"
    );
}

#[test]
fn belonging_together_merges_steps_and_not_one_workflow_splits_cases() {
    for (label, phrase) in [
        ("merge-en", "These belong together"),
        ("merge-de", "Das gehört zusammen"),
    ] {
        let (_db, mut store) = refreshed(label);
        let before = invoice(&store);
        assert_eq!(before.steps.len(), 3);
        store
            .correct(
                WEEK_NOW,
                phrase,
                &[
                    Target::Step(before.steps[0].key.clone()),
                    Target::Step(before.steps[1].key.clone()),
                ],
            )
            .expect("test setup");
        store.refresh(WEEK_NOW).expect("test setup");
        let merged = invoice(&store);
        assert_eq!(merged.steps.len(), 2, "{phrase}");
        assert!(
            merged.hours_week > 0.0 && merged.case_count == 5,
            "{phrase}"
        );
    }

    let (before, _split, store) = corrected("split", "Das sind zwei Workflows");
    assert!(
        store
            .workflows()
            .expect("test setup")
            .iter()
            .all(|w| w.basis != "cases" || w.case_count < before.case_count),
        "no workflow keeps all five cases together"
    );
}
