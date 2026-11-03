use std::fmt::Write;

use insta::assert_snapshot;
use sextant_core::fixtures::*;
use sextant_core::redact;

#[test]
fn week_load_and_refresh_counts() {
    let db = TempDb::new("golden-counts");
    let mut store = db.open().expect("test setup");
    let flushed = load(&mut store, &week(), WEEK_NOW).expect("test setup");
    let stats = store.refresh(local(WEEK_NOW)).expect("test setup");
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
        .map(|s| format!("{s}\n=> {}", redact(s, &TEST_SECRET)))
        .collect();
    assert_snapshot!(out.join("\n\n"));
}
