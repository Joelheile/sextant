//! "Connect to Claude": the file work around `sextant_core::connect`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

/// Dev builds use a file in the dev data folder, so they never touch the real Claude config.
pub fn desktop_config_path() -> Result<PathBuf> {
    if cfg!(debug_assertions) {
        return Ok(crate::home::data_dir()?.join("claude_desktop_config.json"));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join("Library/Application Support/Claude/claude_desktop_config.json"))
}

#[derive(Debug, Serialize)]
pub struct Written {
    pub path: String,
    pub backup: Option<String>,
}

/// Adds Sextant to the Claude Desktop config at `path`. An existing file is copied to
/// `<name>.sextant-<stamp>.bak` first, and a config that is not valid JSON is left alone.
pub fn write_desktop_config(path: &Path, binary: &str, stamp: i64) -> Result<Written> {
    let existing = match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err).context("reading the Claude Desktop config"),
    };
    let merged =
        sextant_core::connect::claude_desktop_config(existing.as_deref().unwrap_or(""), binary)?;

    let mut backup = None;
    if existing.is_some() {
        let mut name = path.as_os_str().to_owned();
        name.push(format!(".sextant-{stamp}.bak"));
        let target = PathBuf::from(name);
        fs::copy(path, &target).context("backing up the Claude Desktop config")?;
        backup = Some(target.to_string_lossy().into_owned());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("creating the Claude folder")?;
    }
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, merged).context("writing the Claude Desktop config")?;
    fs::rename(&temp, path).context("replacing the Claude Desktop config")?;
    Ok(Written {
        path: path.to_string_lossy().into_owned(),
        backup,
    })
}

/// Takes Sextant out of the Claude Desktop config at `path`. A missing file, a config without
/// Sextant or one that is not valid JSON stays as it is.
pub fn remove_from_desktop_config(path: &Path) -> Result<()> {
    let existing = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err).context("reading the Claude Desktop config"),
    };
    let Some(left) = sextant_core::connect::without_sextant(&existing) else {
        return Ok(());
    };
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, left).context("writing the Claude Desktop config")?;
    fs::rename(&temp, path).context("replacing the Claude Desktop config")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sextant-connect-{}-{label}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("test setup");
        dir
    }

    #[test]
    fn merges_into_an_existing_config_and_keeps_a_backup() {
        let dir = temp("merge");
        let config = dir.join("claude_desktop_config.json");
        let original = r#"{"mcpServers": {"notes": {"command": "npx"}}}"#;
        fs::write(&config, original).expect("test setup");

        let written = write_desktop_config(
            &config,
            "/Applications/Sextant.app/Contents/MacOS/sextant",
            42,
        )
        .expect("writes");

        let backup = written.backup.expect("a backup");
        assert!(backup.ends_with("claude_desktop_config.json.sextant-42.bak"));
        assert_eq!(fs::read_to_string(backup).expect("backup"), original);
        let merged: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config).expect("config")).expect("json");
        assert_eq!(merged["mcpServers"]["notes"]["command"], "npx");
        assert_eq!(merged["mcpServers"]["sextant"]["args"][0], "mcp");
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn creates_the_file_and_folder_when_missing() {
        let dir = temp("fresh");
        let config = dir.join("Claude/claude_desktop_config.json");
        let written = write_desktop_config(&config, "/bin/sextant", 1).expect("writes");
        assert_eq!(written.backup, None);
        assert!(config.is_file());
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn removing_undoes_the_connect_and_keeps_the_rest() {
        let dir = temp("remove");
        let config = dir.join("claude_desktop_config.json");
        fs::write(&config, r#"{"mcpServers": {"notes": {"command": "npx"}}}"#).expect("test setup");
        write_desktop_config(&config, "/bin/sextant", 1).expect("test setup");

        remove_from_desktop_config(&config).expect("removes");
        remove_from_desktop_config(&dir.join("missing.json")).expect("nothing to remove");

        let left: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config).expect("config")).expect("json");
        assert_eq!(left["mcpServers"]["notes"]["command"], "npx");
        assert!(left["mcpServers"].get("sextant").is_none());
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn leaves_a_broken_config_untouched() {
        let dir = temp("broken");
        let config = dir.join("claude_desktop_config.json");
        fs::write(&config, "{oops").expect("test setup");
        assert!(write_desktop_config(&config, "/bin/sextant", 1).is_err());
        assert_eq!(fs::read_to_string(&config).expect("config"), "{oops");
        assert_eq!(
            fs::read_dir(&dir).expect("dir").count(),
            1,
            "no backup, no temp file"
        );
        fs::remove_dir_all(dir).expect("cleanup");
    }
}
