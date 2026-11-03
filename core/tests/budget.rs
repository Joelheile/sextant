//! Plan.md §10 budgets. Run with `cargo test -p sextant-core --release -- --nocapture`.
//! Debug builds skip them: the numbers mean nothing without optimisations.

use std::time::{Duration, Instant};

use sextant_core::Store;
use sextant_core::fixtures::*;

const SEARCH_P95: Duration = Duration::from_millis(100);
const REFRESH: Duration = Duration::from_secs(2);
const DAY: i64 = 86_400;

fn percentile(times: &mut [Duration], percent: usize) -> Duration {
    times.sort();
    times[(times.len() * percent).div_ceil(100) - 1]
}

fn run(store: &Store, query: &str) -> Duration {
    let start = Instant::now();
    let hits = store
        .search(query, None, None, None, 20, 0)
        .expect("test setup");
    let elapsed = start.elapsed();
    assert!(!hits.is_empty(), "{query:?} found nothing");
    elapsed
}

#[test]
#[cfg_attr(debug_assertions, ignore = "budget tests run in release mode")]
fn search_over_one_year_stays_under_100_ms() {
    let db = TempDb::new("budget-search");
    let mut store = db.open().expect("test setup");
    let now = WEEK_START + 400 * DAY;
    let started = Instant::now();
    let sample = fill_frames(&mut store, now, 365, 2000, 42).expect("test setup");
    let size = std::fs::metadata(&db.path).expect("test setup").len()
        + std::fs::metadata(db.path.with_extension("db-wal")).map_or(0, |m| m.len());
    println!(
        "generated 365 days x 2000 frames in {:.1?}, {:.1} MB per day",
        started.elapsed(),
        size as f64 / 365.0 / 1e6
    );

    let reader = store.reader().expect("test setup");
    let rare: Vec<String> = sample
        .words
        .iter()
        .step_by(sample.words.len() / 100)
        .cloned()
        .collect();
    let phrases: Vec<String> = sample
        .phrases
        .iter()
        .step_by(sample.phrases.len() / 100)
        .cloned()
        .collect();
    let common = common_words(20);
    let mut classes = [
        ("rare word", rare),
        ("two-word phrase", phrases),
        ("common word", common),
    ];

    let mut all = Vec::new();
    for (name, queries) in &mut classes {
        let mut times: Vec<Duration> = queries.iter().map(|q| run(&reader, q)).collect();
        println!(
            "{name:16} n={:3} p95={:?} max={:?}",
            times.len(),
            percentile(&mut times, 95),
            times.iter().max().expect("test setup")
        );
        all.extend(times);
    }
    let total = percentile(&mut all, 95);
    println!(
        "search p95 over {} queries: {total:?} (budget {SEARCH_P95:?}), p99 {:?}, max {:?}",
        all.len(),
        percentile(&mut all, 99),
        all.last().expect("test setup")
    );
    assert!(
        total < SEARCH_P95,
        "search p95 {total:?} is over the {SEARCH_P95:?} budget"
    );
}

#[test]
#[cfg_attr(debug_assertions, ignore = "budget tests run in release mode")]
fn refresh_over_90_days_stays_under_2_s() {
    let db = TempDb::new("budget-refresh");
    let mut store = db.open().expect("test setup");
    let spec = Spec {
        first_monday: WEEK_START,
        weeks: 13,
        noise_per_day: 250,
        frames: false,
    };
    let fixture = generate(&spec);
    let now = WEEK_START + 90 * DAY;
    let flushed = load(&mut store, &fixture, now).expect("test setup");
    println!(
        "loaded {} events, {} utterances",
        flushed.events, flushed.utterances
    );

    let mut times = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let stats = store.refresh(local(now)).expect("test setup");
        times.push(start.elapsed());
        println!("refresh: {:?} {stats:?}", times.last().expect("test setup"));
    }
    let worst = *times.iter().max().expect("test setup");
    println!("refresh over 90 days: worst of 5 = {worst:?} (budget {REFRESH:?})");
    assert!(
        worst < REFRESH,
        "refresh {worst:?} is over the {REFRESH:?} budget"
    );
}
