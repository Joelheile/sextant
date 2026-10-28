//! `settings.json` in the data folder: read by the app and, for every request, by `sextant mcp`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sextant_core::Settings;

fn path(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

/// Defaults when there is no file yet.
pub fn load(dir: &Path) -> Result<Settings> {
    let text = match fs::read_to_string(path(dir)) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err).context("reading settings.json"),
    };
    Ok(Settings::from_json(&text)?)
}

/// Read by the MCP server: when the file is broken, every tool stays off rather than guessing.
pub fn load_or_locked(dir: &Path) -> Settings {
    load(dir).unwrap_or_else(|_| Settings {
        mcp_rewind: false,
        mcp_workflows: false,
        ..Settings::default()
    })
}

fn save(dir: &Path, settings: &Settings) -> Result<()> {
    let target = path(dir);
    let temp = target.with_extension("json.tmp");
    fs::write(&temp, settings.to_json()?).context("writing settings.json")?;
    fs::rename(&temp, &target).context("replacing settings.json")
}

/// Applies `change` to the saved settings and writes them back.
pub fn update(dir: &Path, change: impl FnOnce(&mut Settings)) -> Result<Settings> {
    let mut settings = load(dir)?;
    change(&mut settings);
    save(dir, &settings)?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sextant-prefs-{}-{label}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("test setup");
        dir
    }

    #[test]
    fn a_missing_file_means_defaults_and_updates_persist() {
        let dir = temp("update");
        assert_eq!(load(&dir).expect("defaults"), Settings::default());
        update(&dir, |s| {
            s.onboarded = true;
            s.mcp_rewind = false;
        })
        .expect("saves");
        let saved = load(&dir).expect("loads");
        assert!(saved.onboarded && !saved.mcp_rewind && saved.mcp_workflows);
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn a_broken_file_switches_the_mcp_tools_off() {
        let dir = temp("broken");
        fs::write(path(&dir), "{not json").expect("test setup");
        assert!(load(&dir).is_err());
        let locked = load_or_locked(&dir);
        assert!(!locked.mcp_rewind && !locked.mcp_workflows);
        fs::remove_dir_all(dir).expect("cleanup");
    }
}
