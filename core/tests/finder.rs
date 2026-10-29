use sextant_core::fixtures::*;
use sextant_core::{Action, Category, Enjoyment, Error, Kind, Rule, Store, Target, Workflow};

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

#[test]
fn finder_recovers_the_invoice_workflow() {
    let (_db, store) = refreshed("invoice");
    let workflows = store.workflows().expect("test setup");
    assert_eq!(
        workflows.len(),
        2,
        "invoice workflow and the morning routine, no noise: {workflows:#?}"
    );
    let top = &workflows[0];
    assert_eq!(top.basis, "cases");
    assert_eq!(
        top.case_count, 5,
        "five invoices, English and German mails together"
    );
    let categories: Vec<_> = top.steps.iter().map(|s| s.category).collect();
    assert_eq!(
        categories,
        [Category::Email, Category::Spreadsheet, Category::Finance]
    );
    assert_eq!(top.kind, Kind::Friction);
    assert_eq!(top.action, Action::Automate);
    assert!(top.hours_week > 1.0 && top.potential > 0.5);
    assert!(top.reasons.len() >= 3 && top.reasons.iter().all(|r| !r.is_empty()));

    let routine = &workflows[1];
    assert_eq!(routine.basis, "chain");
    let apps: Vec<_> = routine
        .steps
        .iter()
        .map(|s| s.name.split(':').next().unwrap_or_default())
        .collect();
    assert_eq!(apps, ["Calendar", "Slack", "Linear"]);
    assert!(routine.benefit < top.benefit);
}

#[test]
fn detail_has_examples_copy_points_and_commitments() {
    let (_db, store) = refreshed("detail");
    let id = invoice(&store).id;
    let detail = store
        .workflow_detail(id)
        .expect("test setup")
        .expect("test setup");
    assert_eq!(detail.examples.len(), 3);
    assert!(
        detail
            .examples
            .iter()
            .all(|e| INVOICE_IDS.contains(&e.entity.as_str())
                || e.entity.starts_with("INV-")
                || e.entity.starts_with("RE-"))
    );
    assert!(
        detail
            .examples
            .iter()
            .all(|e| e.path.len() >= 3 && e.last_ts > e.first_ts)
    );
    assert!(!detail.copy_points.is_empty());
    assert!(!detail.commitments.is_empty());
    assert!(store.workflow_detail(1).expect("test setup").is_none());
    assert!(matches!(store.skill_md(1), Err(Error::NoWorkflow(1))));
}

#[test]
fn refresh_is_deterministic_and_repeatable() {
    let (_a, mut first) = refreshed("determinism-a");
    let (_b, second) = refreshed("determinism-b");
    let before = first.workflows().expect("test setup");
    assert_eq!(before, second.workflows().expect("test setup"));
    first.refresh(WEEK_NOW).expect("test setup");
    assert_eq!(before, first.workflows().expect("test setup"));
    let id = before[0].id;
    assert_eq!(
        first.skill_md(id).expect("test setup"),
        second.skill_md(id).expect("test setup")
    );
}

#[test]
fn skill_md_lists_steps_and_reasons() {
    let (_db, store) = refreshed("skill");
    let w = invoice(&store);
    let skill = store.skill_md(w.id).expect("test setup");
    assert!(skill.starts_with("---\nname: "));
    for part in [
        "## Steps",
        "## Data copied by hand",
        "## Example cases",
        "## Rules",
        "## Why Sextant picked it",
        "Start with:",
    ] {
        assert!(skill.contains(part), "missing {part}");
    }
    assert!(skill.contains("Mon 2026-03-02"));
    assert!(w.reasons.iter().all(|r| skill.contains(r.as_str())));
}

#[test]
fn corrections_apply_at_the_next_refresh() {
    let (_db, mut store) = refreshed("corrections");
    let w = invoice(&store);
    assert_eq!(w.enjoyment, Enjoyment::Unknown);
    assert!(
        store
            .ask_first()
            .expect("test setup")
            .iter()
            .any(|a| a.id == w.id)
    );

    let rule = store
        .correct(WEEK_NOW, "Rename to Invoice run", &[Target::Workflow(w.id)])
        .expect("test setup");
    assert_eq!(rule, Rule::Rename("Invoice run".into()));
    assert_eq!(
        invoice(&store).name,
        w.name,
        "nothing changes before refresh"
    );

    store
        .correct(WEEK_NOW, "Ich hasse das", &[Target::Workflow(w.id)])
        .expect("test setup");
    store
        .correct(WEEK_NOW, "It's for a client", &[Target::Workflow(w.id)])
        .expect("test setup");
    store.refresh(WEEK_NOW).expect("test setup");
    let changed = invoice(&store);
    assert_eq!(changed.name, "Invoice run");
    assert_eq!(changed.enjoyment, Enjoyment::Hate);
    assert!(changed.client);
    assert!(
        changed.desire > w.desire,
        "hated work is wanted gone: {} vs {}",
        changed.desire,
        w.desire
    );
    assert!(
        store
            .ask_first()
            .expect("test setup")
            .iter()
            .all(|a| a.id != w.id)
    );
    assert!(
        changed
            .reasons
            .iter()
            .any(|r| r.to_lowercase().contains("hate")),
        "{:?}",
        changed.reasons
    );

    store
        .correct(
            WEEK_NOW,
            "Das lässt sich nicht automatisieren",
            &[Target::Workflow(w.id)],
        )
        .expect("test setup");
    store
        .correct(WEEK_NOW, "hide this", &[Target::Workflow(w.id)])
        .expect("test setup");
    store.refresh(WEEK_NOW).expect("test setup");
    assert!(
        store
            .workflows()
            .expect("test setup")
            .iter()
            .all(|x| x.id != w.id)
    );
    assert!(
        store.workflow(w.id).expect("test setup").is_some(),
        "hidden is not deleted"
    );
}

#[test]
fn bad_corrections_are_rejected() {
    let (_db, mut store) = refreshed("bad-corrections");
    let id = invoice(&store).id;
    assert!(matches!(
        store.correct(1, "what a nice day", &[Target::Workflow(id)]),
        Err(Error::UnknownCorrection)
    ));
    assert!(matches!(
        store.correct(1, "hide this", &[Target::Workflow(7)]),
        Err(Error::NoWorkflow(7))
    ));
    assert!(matches!(
        store.correct(1, "hide this", &[Target::Step("x".into())]),
        Err(Error::BadTarget(_))
    ));
    assert!(matches!(
        store.correct(1, "These belong together", &[Target::Workflow(id)]),
        Err(Error::BadTarget(_))
    ));
}

#[test]
fn merge_and_split_change_the_workflow_shape() {
    let (_db, mut store) = refreshed("merge-split");
    let w = invoice(&store);
    store
        .correct(
            WEEK_NOW,
            "These belong together",
            &[
                Target::Step(w.steps[0].key.clone()),
                Target::Step(w.steps[1].key.clone()),
            ],
        )
        .expect("test setup");
    store.refresh(WEEK_NOW).expect("test setup");
    let merged = invoice(&store);
    assert_eq!(
        merged.steps.len(),
        2,
        "mail and spreadsheet are one step now: {:?}",
        merged.steps
    );

    store
        .correct(
            WEEK_NOW,
            "This is not one workflow",
            &[Target::Workflow(merged.id)],
        )
        .expect("test setup");
    store.refresh(WEEK_NOW).expect("test setup");
    assert!(
        store
            .workflows()
            .expect("test setup")
            .iter()
            .all(|x| x.case_count < 5 || x.basis != "cases")
    );
}

#[test]
fn only_the_last_90_days_count() {
    let (_db, mut store) = refreshed("window");
    let later = WEEK_NOW + 100 * 86_400;
    let stats = store.refresh(later).expect("test setup");
    assert_eq!(stats.episodes, 0);
    assert!(store.workflows().expect("test setup").is_empty());
}

#[test]
fn refresh_never_reads_further_back_than_retention() {
    let (_db, mut store) = refreshed("retention-window");
    let later = WEEK_NOW + 40 * 86_400;
    assert!(store.refresh(later).expect("test setup").episodes > 0);
    store.set_retention_days(30);
    assert_eq!(store.refresh(later).expect("test setup").episodes, 0);
}

#[test]
fn a_model_judgment_replaces_the_category_prior_at_the_next_refresh() {
    let (_db, mut store) = refreshed("judgments");
    let before = invoice(&store);
    let samples = store.unjudged_steps(50).expect("test setup");
    let step = &before.steps[0];
    let sample = samples
        .iter()
        .find(|s| s.key == step.key)
        .expect("every step starts unjudged");
    assert!(!sample.apps.is_empty() && !sample.objects.is_empty());
    assert!(step.potential > 0.0);

    store
        .add_judgment(WEEK_NOW, &step.key, 0.0, "needs a person to decide")
        .expect("test setup");
    assert!(
        store
            .unjudged_steps(50)
            .expect("test setup")
            .iter()
            .all(|s| s.key != step.key)
    );
    store.refresh(WEEK_NOW).expect("test setup");
    let after = invoice(&store);
    assert_eq!(after.steps[0].potential, 0.0);
    assert!(after.potential < before.potential);
}
