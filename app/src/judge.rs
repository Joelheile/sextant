//! Asks Apple's on-device model how much of each recurring step an agent could do. The answers
//! replace the category guess in the finder's potential at the next refresh. Without Apple
//! Intelligence nothing is stored and the category guess stays.

use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use tauri::{AppHandle, Manager};

use crate::clock;
use crate::commands::AppState;
use crate::engine::Engine;
use crate::platform::language::{self, Step};

const EVERY: Duration = Duration::from_secs(5 * 60);
/// Steps judged per round; each takes the model a few seconds.
const PER_ROUND: usize = 5;

pub fn spawn(app: AppHandle) -> Result<()> {
    thread::Builder::new()
        .name("sextant-judge".into())
        .spawn(move || {
            loop {
                thread::sleep(EVERY);
                if let Ok(engine) = app.state::<AppState>().engine()
                    && let Err(err) = round(engine)
                {
                    eprintln!("judging steps failed: {err:#}");
                }
            }
        })
        .context("starting the judge thread")?;
    Ok(())
}

fn round(engine: &Engine) -> Result<()> {
    let steps = engine.read(|store| store.unjudged_steps(PER_ROUND))?;
    let judged: Vec<(String, f64, String)> = steps
        .iter()
        .filter_map(|step| {
            let (share, reason) = language::judge(&Step {
                name: &step.name,
                category: &step.category,
                apps: &step.apps,
                objects: &step.objects,
            })?;
            Some((step.key.clone(), share, reason))
        })
        .collect();
    if judged.is_empty() {
        return Ok(());
    }
    let now = clock::now();
    engine.change(move |store| {
        for (key, share, reason) in &judged {
            store.add_judgment(now, key, *share, reason)?;
        }
        Ok(())
    })
}
