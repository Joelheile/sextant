use std::path::PathBuf;

use anyhow::{Context, Result};

/// Dev builds refuse to run without `SEXTANT_HOME`, so they never touch the real data folder.
pub fn data_dir() -> Result<PathBuf> {
    if cfg!(debug_assertions) {
        let dir = std::env::var_os("SEXTANT_HOME")
            .filter(|dir| !dir.is_empty())
            .context("dev builds need SEXTANT_HOME; start them through cargo, which sets it from .cargo/config.toml")?;
        return Ok(dir.into());
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join("Library/Application Support/Sextant"))
}

pub fn db_path(dir: &std::path::Path) -> PathBuf {
    dir.join("sextant.db")
}
