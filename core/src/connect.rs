//! "Connect to Claude": the text the app writes into Claude Desktop's config and
//! shows for Claude Code. File access stays in the app.

use serde_json::{Value, json};

use crate::error::{Error, Result};

/// Adds or replaces the `sextant` entry under `mcpServers`, keeping every other setting.
/// `existing` is the current file content, empty when there is no file yet.
pub fn claude_desktop_config(existing: &str, binary: &str) -> Result<String> {
    let mut config: Value = if existing.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(existing)
            .map_err(|_| Error::Invalid("the Claude Desktop config is not valid JSON".into()))?
    };
    let root = config
        .as_object_mut()
        .ok_or_else(|| Error::Invalid("the Claude Desktop config is not a JSON object".into()))?;
    let servers = root
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| {
            Error::Invalid("mcpServers in the Claude Desktop config is not an object".into())
        })?;
    servers.insert(
        "sextant".into(),
        json!({"command": binary, "args": ["mcp"]}),
    );
    Ok(serde_json::to_string_pretty(&config)? + "\n")
}

/// The config without the `sextant` entry, for the uninstall. `None` when there is nothing to
/// remove, including a config that is not valid JSON, which is left alone.
pub fn without_sextant(existing: &str) -> Option<String> {
    let mut config: Value = serde_json::from_str(existing).ok()?;
    config
        .get_mut("mcpServers")?
        .as_object_mut()?
        .remove("sextant")?;
    serde_json::to_string_pretty(&config)
        .ok()
        .map(|text| text + "\n")
}

/// The one-line command for Claude Code. The path is quoted when the shell would split it.
pub fn claude_code_command(binary: &str) -> String {
    let plain = binary
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+@%:,=".contains(c));
    let path = if plain {
        binary.to_string()
    } else {
        format!("'{}'", binary.replace('\'', r"'\''"))
    };
    format!("claude mcp add sextant -- {path} mcp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merging_keeps_other_servers_and_settings() {
        let existing = r#"{"globalShortcut": "Alt+Space", "mcpServers": {"notes": {"command": "npx"}, "sextant": {"command": "/old"}}}"#;
        let merged =
            claude_desktop_config(existing, "/Applications/Sextant.app/Contents/MacOS/sextant")
                .expect("merges");
        let value: Value = serde_json::from_str(&merged).expect("valid json");
        assert_eq!(value["globalShortcut"], "Alt+Space");
        assert_eq!(value["mcpServers"]["notes"]["command"], "npx");
        assert_eq!(
            value["mcpServers"]["sextant"],
            json!({"command": "/Applications/Sextant.app/Contents/MacOS/sextant", "args": ["mcp"]})
        );
    }

    #[test]
    fn a_missing_file_gets_a_fresh_config() {
        let merged = claude_desktop_config("  ", "/bin/sextant").expect("merges");
        let value: Value = serde_json::from_str(&merged).expect("valid json");
        assert_eq!(value["mcpServers"]["sextant"]["args"], json!(["mcp"]));
    }

    #[test]
    fn broken_configs_are_refused_not_overwritten() {
        for bad in ["{not json", "[]", r#"{"mcpServers": []}"#] {
            assert!(claude_desktop_config(bad, "/bin/sextant").is_err(), "{bad}");
        }
    }

    #[test]
    fn the_uninstall_removes_only_the_sextant_entry() {
        let existing = r#"{"globalShortcut": "Alt+Space", "mcpServers": {"notes": {"command": "npx"}, "sextant": {"command": "/old"}}}"#;
        let left: Value =
            serde_json::from_str(&without_sextant(existing).expect("removes")).expect("json");
        assert_eq!(left["globalShortcut"], "Alt+Space");
        assert_eq!(left["mcpServers"]["notes"]["command"], "npx");
        assert!(left["mcpServers"].get("sextant").is_none());
        for untouched in ["{not json", r#"{"mcpServers": {"notes": {}}}"#, "{}"] {
            assert_eq!(without_sextant(untouched), None, "{untouched}");
        }
    }

    #[test]
    fn the_claude_code_command_quotes_odd_paths() {
        assert_eq!(
            claude_code_command("/Applications/Sextant.app/Contents/MacOS/sextant"),
            "claude mcp add sextant -- /Applications/Sextant.app/Contents/MacOS/sextant mcp"
        );
        assert_eq!(
            claude_code_command("/Users/Jo Doe/it's/sextant"),
            r"claude mcp add sextant -- '/Users/Jo Doe/it'\''s/sextant' mcp"
        );
    }
}
