//! User settings: one small JSON file in the data folder, read by the app and by `sextant mcp`.

use serde::{Deserialize, Serialize};

use crate::agent::AgentName;
use crate::error::Result;

/// Backfill sources the user can untick in onboarding (plan.md §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sources {
    /// Chrome, Arc, Brave and Edge.
    pub browsers: bool,
    /// Needs Full Disk Access.
    pub safari: bool,
    /// Calendar and reminders.
    pub calendar: bool,
    pub git: bool,
    pub files: bool,
}

impl Default for Sources {
    fn default() -> Self {
        Self {
            browsers: true,
            safari: true,
            calendar: true,
            git: true,
            files: true,
        }
    }
}

/// Live capture sources (plan.md §3). Each one can be switched off on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Live {
    /// Focused app, window title, document and browser address.
    pub metadata: bool,
    /// OCR text of the focused window.
    pub ocr: bool,
    /// Which app copied, never what.
    pub copy: bool,
    /// Presses on labelled controls and the values left in text fields, never keystrokes.
    pub actions: bool,
    /// Mic and system audio transcripts during calls.
    pub audio: bool,
    /// Safari, Firefox and other browsers that cannot report a private window. Off: their
    /// windows are never read, because a private one could not be told apart.
    pub opaque_browsers: bool,
}

impl Default for Live {
    fn default() -> Self {
        Self {
            metadata: true,
            ocr: true,
            copy: true,
            actions: true,
            audio: false,
            opaque_browsers: false,
        }
    }
}

impl Live {
    /// The screen sources. Audio has its own switch and its own pipeline.
    pub fn any(&self) -> bool {
        self.metadata || self.ocr || self.copy || self.actions
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Apps whose screen text never leaves the database through MCP (plan.md §4: password
    /// managers, Health, messengers). Compared case-insensitively against the app name.
    pub excluded_apps: Vec<String>,
    /// Switch for `search`, `context_at` and `timeline`.
    pub mcp_rewind: bool,
    /// Switch for `week`, `day`, `task`, `list_workflows`, `get_workflow`, `get_skill` and
    /// `report_handoff`. The typed values in `task` also need `mcp_rewind`.
    pub mcp_workflows: bool,
    pub onboarded: bool,
    pub sources: Sources,
    pub live: Live,
    /// Folders the user picked for git backfill.
    pub git_folders: Vec<String>,
    /// How long raw data stays (plan.md §3): 30, 90 or 365 days, `None` for forever.
    pub retention_days: Option<u32>,
    /// The agent CLI the bridge runs: `claude`, `codex`, or `None` while the bridge is off.
    pub agent: Option<AgentName>,
    /// Runs the agent once a week on its own.
    pub agent_weekly: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            excluded_apps: [
                "1Password",
                "Bitwarden",
                "Dashlane",
                "LastPass",
                "KeePassXC",
                "Passwords",
                "Keychain Access",
                "Health",
                "Messages",
                "WhatsApp",
                "Signal",
                "Telegram",
            ]
            .map(String::from)
            .to_vec(),
            mcp_rewind: true,
            mcp_workflows: true,
            onboarded: false,
            sources: Sources::default(),
            live: Live::default(),
            git_folders: Vec::new(),
            retention_days: Some(90),
            agent: None,
            agent_weekly: false,
        }
    }
}

impl Settings {
    /// An empty or missing file means defaults, so pass `""` when there is no file yet.
    pub fn from_json(text: &str) -> Result<Self> {
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        Ok(serde_json::from_str(text)?)
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn is_excluded(&self, app: &str) -> bool {
        let app = app.to_lowercase();
        !app.is_empty()
            && self
                .excluded_apps
                .iter()
                .any(|excluded| app.starts_with(&excluded.to_lowercase()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_keys_fall_back_to_defaults() {
        let settings = Settings::from_json(r#"{"mcp_rewind": false}"#).expect("parses");
        assert!(!settings.mcp_rewind && settings.mcp_workflows);
        assert_eq!(Settings::from_json("").expect("empty"), Settings::default());
        let round = Settings::from_json(&settings.to_json().expect("json")).expect("round trip");
        assert_eq!(round, settings);
    }

    #[test]
    fn the_agent_is_claude_or_codex() {
        let settings = Settings::from_json(r#"{"agent": "codex"}"#).expect("parses");
        assert_eq!(settings.agent, Some(AgentName::Codex));
        assert!(Settings::from_json(r#"{"agent": "gemini"}"#).is_err());
    }

    #[test]
    fn exclusion_ignores_case_and_version_suffixes() {
        let settings = Settings::default();
        assert!(settings.is_excluded("1password 8"));
        assert!(settings.is_excluded("Passwords"));
        assert!(settings.is_excluded("Messages"));
        assert!(!settings.is_excluded("Mail"));
        assert!(!settings.is_excluded(""));
    }
}
