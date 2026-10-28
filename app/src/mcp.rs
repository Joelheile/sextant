//! `sextant mcp`: the same binary as a stdio MCP server (plan.md §8). Read-only, no Tauri.

use std::io;
use std::path::Path;

use anyhow::{Context, Result};
use sextant_core::Store;
use sextant_core::mcp::Server;

use crate::{clock, home, keys, prefs};

pub fn run(dir: &Path) -> Result<()> {
    let path = home::db_path(dir);
    let key = keys::load()?.filter(|_| path.exists());
    let key = key.context("Sextant has no database yet: open the app once first")?;
    let mut store = Store::open_read_only(&path, &key).context("opening the Sextant database")?;
    store.set_utc_offset(clock::utc_offset(clock::now()));

    let dir = dir.to_path_buf();
    let server = Server::new(store, move || prefs::load_or_locked(&dir), clock::now);
    server
        .serve(io::stdin().lock(), io::stdout().lock())
        .context("serving MCP")
}
