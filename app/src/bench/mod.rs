//! `sextant bench` and `sextant bench --live <seconds>`; `--asr` and
//! `--fetch-models` are in [`asr`]. All print counts and timings only: no text, title or
//! address ever reaches the output.
//!
//! The fixture mode times OCR on synthetic windows. The live mode runs real capture for the given
//! time into a throwaway database with a random key in the temp folder. It never reads the app's
//! data folder or the Keychain, and it needs Screen Recording for the terminal that runs it.

mod asr;

use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use sextant_core::{Item, Settings, Store};

use crate::capture::{Control, Ingest, Sink, Stats, percentile};
use crate::{clock, platform};

const BUDGET_MS: f64 = 250.0;
const PUMP_S: f64 = 0.1;

pub fn run(args: &[String]) -> Result<()> {
    match args {
        [] => fixtures(),
        [flag, seconds] if flag == "--live" => live(
            seconds
                .parse()
                .context("the number of seconds is not a number")?,
        ),
        [flag, rest @ ..] if flag == "--asr" => asr::asr(rest),
        [flag, dir @ ..] if flag == "--fetch-models" => asr::fetch_models(dir.first()),
        _ => bail!(
            "usage: sextant bench [--live <seconds> | --asr [--coreml] [models] [speech] | --fetch-models [folder]]"
        ),
    }
}

fn fixtures() -> Result<()> {
    println!("OCR time per window on synthetic fixtures (budget p95 under {BUDGET_MS} ms)");
    let mut over = false;
    for timing in platform::ocr_fixture_timings()? {
        println!(
            "  {}x{} {:<8} p50 {:6.1} ms  p95 {:6.1} ms  text not found {}",
            timing.width,
            timing.height,
            timing.level,
            timing.p50_ms,
            timing.p95_ms,
            timing.missing.len()
        );
        over |= timing.level == "fast" && timing.p95_ms >= BUDGET_MS;
    }
    if over {
        bail!("the chosen OCR level is over the budget");
    }
    Ok(())
}

/// User and system CPU seconds and the peak resident size in bytes of this process.
fn usage() -> (f64, u64) {
    // SAFETY: `rusage` is plain data that `getrusage` fills.
    let usage = unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut usage);
        usage
    };
    let seconds = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    (
        seconds(usage.ru_utime) + seconds(usage.ru_stime),
        usage.ru_maxrss as u64,
    )
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

fn scratch_dir() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("sextant-bench-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .context("creating the scratch folder")?;
    Ok(dir)
}

fn live(seconds: u64) -> Result<()> {
    let dir = scratch_dir()?;
    let result = live_in(&dir, seconds);
    let _ = fs::remove_dir_all(&dir);
    result
}

fn live_in(dir: &Path, seconds: u64) -> Result<()> {
    let db = dir.join("bench.db");
    let (mut key, mut secret) = ([0u8; 32], [0u8; 16]);
    getrandom::fill(&mut key).map_err(|err| anyhow::anyhow!("no random numbers: {err}"))?;
    getrandom::fill(&mut secret).map_err(|err| anyhow::anyhow!("no random numbers: {err}"))?;
    let mut store = Store::open(&db, &key, &secret).context("opening the scratch database")?;

    let (record, records) = mpsc::channel::<(Item, Instant)>();
    let sink: Sink = Arc::new(move |item, taken| {
        let _ = record.send((item, taken));
    });
    let control = Arc::new(Control::new(Settings::default()));
    let pipeline = platform::start_live(control, sink)?;

    println!("live bench for {seconds} s: use the computer as usual, then read the numbers");
    let (cpu_before, _) = usage();
    let started = Instant::now();
    let mut ingest = Ingest::new();
    let mut latency_ms = Vec::new();
    let mut flush = |ingest: &mut Ingest, store: &mut Store| -> Result<()> {
        let (_, latencies) = ingest.flush(store, clock::now())?;
        latency_ms.extend(latencies.iter().map(|l| l.as_secs_f64() * 1000.0));
        Ok(())
    };
    while started.elapsed() < Duration::from_secs(seconds) {
        // The workspace only reports the frontmost app while the main run loop runs.
        platform::pump(PUMP_S);
        while let Ok((item, taken)) = records.try_recv() {
            ingest.push(item, taken);
        }
        if ingest.due() {
            flush(&mut ingest, &mut store)?;
        }
    }
    let stats = pipeline.stats.clone();
    pipeline.stop();
    while let Ok((item, taken)) = records.try_recv() {
        ingest.push(item, taken);
    }
    flush(&mut ingest, &mut store)?;

    let wall = started.elapsed().as_secs_f64();
    let (cpu_after, peak_rss) = usage();
    report(
        &stats,
        &store,
        &db,
        (cpu_after - cpu_before) / wall * 100.0,
        peak_rss,
        &latency_ms,
    )
}

fn report(
    stats: &Stats,
    store: &Store,
    db: &Path,
    cpu_percent: f64,
    peak_rss: u64,
    latency_ms: &[f64],
) -> Result<()> {
    let ocr_ms = stats.ocr_ms();
    let counts = store.counts()?;
    let bytes = file_size(db) + file_size(&db.with_extension("db-wal"));
    println!("cpu                    {cpu_percent:.1} % of one core");
    println!(
        "peak rss               {:.1} MB",
        peak_rss as f64 / 1_048_576.0
    );
    println!(
        "looks                  {} (frames captured {}, skipped by the diff gate {}, OCR'd {}, dropped under load {})",
        Stats::get(&stats.looks),
        Stats::get(&stats.captured),
        Stats::get(&stats.skipped),
        Stats::get(&stats.ocr),
        Stats::get(&stats.dropped),
    );
    println!(
        "errors                 capture {}, OCR {}",
        Stats::get(&stats.capture_errors),
        Stats::get(&stats.ocr_errors),
    );
    println!(
        "OCR per window         p50 {:.1} ms, p95 {:.1} ms",
        percentile(&ocr_ms, 50.0),
        percentile(&ocr_ms, 95.0)
    );
    println!(
        "capture to searchable  p50 {:.2} s, p95 {:.2} s",
        percentile(latency_ms, 50.0) / 1000.0,
        percentile(latency_ms, 95.0) / 1000.0
    );
    println!("db bytes written       {bytes}");
    println!(
        "rows                   events {}, frames {}, lines {}, windows {}",
        counts.events, counts.frames, counts.lines, counts.windows
    );
    let blocked: Vec<String> = stats
        .blocked()
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|(name, count)| format!("{name} {count}"))
        .collect();
    println!(
        "closed by a gate       {}",
        if blocked.is_empty() {
            "none".into()
        } else {
            blocked.join(", ")
        }
    );
    println!("presses seen           {}", Stats::get(&stats.presses));
    println!("field edits seen       {}", Stats::get(&stats.edits));
    if Stats::get(&stats.captured) == 0 && Stats::get(&stats.capture_errors) > 0 {
        println!(
            "no frame was captured: grant Screen Recording to the app that runs this command \
             (System Settings, Privacy and Security) and run it again"
        );
    }
    Ok(())
}
