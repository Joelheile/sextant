use std::fmt::Write;

use insta::assert_snapshot;
use sextant_core::fixtures::*;
use sextant_core::{Store, Workflow, parse_correction, redact};

fn refreshed(label: &str) -> (TempDb, Store) {
    let db = TempDb::new(label);
    let mut store = db.open().expect("test setup");
    load(&mut store, &week(), WEEK_NOW).expect("test setup");
    store.refresh(WEEK_NOW).expect("test setup");
    (db, store)
}

fn show(w: &Workflow) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{} [{}] id={}", w.name, w.basis, w.id);
    let _ = writeln!(out, "variant: {}", w.variant);
    let _ = writeln!(
        out,
        "cases={} hours/week={:.3} potential={:.3} desire={:.3} benefit={:.3} procrastination={:.3}",
        w.case_count, w.hours_week, w.potential, w.desire, w.benefit, w.procrastination
    );
    let _ = writeln!(
        out,
        "kind={} action={} enjoyment={} client={} rough={}",
        w.kind.as_str(),
        w.action.as_str(),
        w.enjoyment.as_str(),
        w.client,
        w.rough
    );
    let _ = writeln!(out, "first step: {}", w.first_step);
    for step in &w.steps {
        let _ = writeln!(
            out,
            "step {} ({}) minutes={:.1} potential={:.3}",
            step.name,
            step.category.as_str(),
            step.minutes,
            step.potential
        );
    }
    for reason in &w.reasons {
        let _ = writeln!(out, "reason: {reason}");
    }
    out
}

#[test]
fn week_workflows() {
    let (_db, store) = refreshed("golden-workflows");
    let text: Vec<String> = store
        .workflows()
        .expect("test setup")
        .iter()
        .map(show)
        .collect();
    assert_snapshot!(text.join("\n"));
}

#[test]
fn invoice_skill_md() {
    let (_db, store) = refreshed("golden-skill");
    let id = store.workflows().expect("test setup")[0].id;
    assert_snapshot!(store.skill_md(id).expect("test setup"));
}

#[test]
fn week_load_and_refresh_counts() {
    let db = TempDb::new("golden-counts");
    let mut store = db.open().expect("test setup");
    let flushed = load(&mut store, &week(), WEEK_NOW).expect("test setup");
    let stats = store.refresh(WEEK_NOW).expect("test setup");
    assert_snapshot!(format!("{flushed:#?}\n{stats:#?}"));
}

#[test]
fn search_results() {
    let (_db, store) = refreshed("golden-search");
    let mut out = String::new();
    for query in [
        "pricing",
        "Rechnung",
        "Preisliste",
        "INV-2026-0418",
        "Müller",
    ] {
        let _ = writeln!(out, "## {query}");
        for hit in store
            .search(query, None, None, None, 5, 0)
            .expect("test setup")
        {
            let _ = writeln!(
                out,
                "{} {:?} {} | {} | {} | {}",
                hit.ts - WEEK_START,
                hit.source,
                hit.app,
                hit.title,
                hit.url,
                hit.text
            );
        }
    }
    assert_snapshot!(out);
}

#[test]
fn redaction_samples() {
    let samples = [
        "Write to anna.keller@acme-gmbh.de or Anna.Keller@acme-gmbh.de",
        "Zahlung an IBAN DE89 3704 0044 0532 0130 00 bitte",
        "IBAN DE89 3704 0044 0532 0130 01 has a bad check digit",
        "Card 4111 1111 1111 1111 and 4111 1111 1111 1112",
        "export OPENAI_API_KEY=sk-proj-abcdef0123456789abcdef0123456789",
        "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk",
        "Invoice INV-2026-0412 total €1,250.00 due 15 March 2026",
        "Rechnung RE-2026-0415 über 980,00 € an Müller AG",
    ];
    let out: Vec<String> = samples
        .iter()
        .map(|s| format!("{s}\n=> {}", redact(s)))
        .collect();
    assert_snapshot!(out.join("\n\n"));
}

#[test]
fn correction_phrases() {
    let phrases = [
        "I hate this",
        "Ich hasse das",
        "I don't like it",
        "ich mag das nicht",
        "I like this",
        "Ich liebe das",
        "It's for a client",
        "Das ist Kundenarbeit",
        "This can't be automated",
        "Das lässt sich nicht automatisieren",
        "Rename to Invoice run",
        "Benenne das um in Rechnungslauf",
        "hide this",
        "Bitte ausblenden",
        "These belong together",
        "Das gehört zusammen",
        "This is not one workflow",
        "Das sind zwei Workflows",
        "Nice weather today",
    ];
    let out: Vec<String> = phrases
        .iter()
        .map(|p| format!("{p:?} => {:?}", parse_correction(p)))
        .collect();
    assert_snapshot!(out.join("\n"));
}
