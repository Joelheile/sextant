//! `sextant mcp`: the same binary as a stdio MCP server (plan.md §8). Read-only, no Tauri.

use std::io;
use std::path::Path;

use anyhow::{Context, Result};
use sextant_core::mcp::Server;
use sextant_core::{Local, Store};

use crate::{clock, home, keys, prefs};

pub fn run(dir: &Path) -> Result<()> {
    let path = home::db_path(dir);
    let key = keys::load()?.filter(|_| path.exists());
    let key = key.context("Sextant has no database yet: open the app once first")?;
    let store = Store::open_read_only(&path, &key).context("opening the Sextant database")?;

    let dir = dir.to_path_buf();
    let server = Server::new(store, move || prefs::load_or_locked(&dir), local);
    server
        .serve(io::stdin().lock(), io::stdout().lock())
        .context("serving MCP")
}

/// Debug builds take the time from `SEXTANT_NOW` (Unix seconds) when it is set, so the bridge
/// tests can read the fixture week.
fn local() -> Local {
    let fixed = std::env::var("SEXTANT_NOW")
        .ok()
        .filter(|_| cfg!(debug_assertions))
        .and_then(|now| now.parse::<i64>().ok());
    match fixed {
        Some(now) => Local {
            now,
            utc_offset_s: clock::utc_offset(now),
        },
        None => clock::local(),
    }
}
