//! User settings: one small JSON file in the data folder, read by the app and by `sextant mcp`.

use serde::{Deserialize, Serialize};

use crate::agent::AgentName;
use crate::error::Result;
use crate::text::split_url;

/// Live capture sources. Each one can be switched off on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Live {
    /// Focused app, window title, document and browser address.
    pub metadata: bool,
    /// OCR text of the focused window.
    pub ocr: bool,
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
            actions: true,
            audio: false,
            opaque_browsers: false,
        }
    }
}

impl Live {
    /// The screen sources. Audio has its own switch and its own pipeline.
    pub fn any(&self) -> bool {
        self.metadata || self.ocr || self.actions
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Apps whose screen text never leaves the database through MCP (password
    /// managers, Health, messengers). Compared case-insensitively against the app name.
    pub excluded_apps: Vec<String>,
    /// Sites never read: banks and payment sites by default. A
    /// site matches its subdomains too.
    pub excluded_sites: Vec<String>,
    /// Switch for `search`, `context_at` and `timeline`.
    pub mcp_rewind: bool,
    /// Switch for `week`, `day`, `task` and the routine tools and prompts. The typed values in
    /// `task` also need `mcp_rewind`.
    pub mcp_workflows: bool,
    pub onboarded: bool,
    pub live: Live,
    /// How long raw data stays: 30, 90 or 365 days, `None` for forever.
    pub retention_days: Option<u32>,
    /// Above this many megabytes of raw data, the oldest days go first. `None` for no cap.
    pub storage_cap_mb: Option<u32>,
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
            excluded_sites: [
                "paypal.com",
                "wise.com",
                "revolut.com",
                "n26.com",
                "sparkasse.de",
                "deutsche-bank.de",
                "commerzbank.de",
                "ing.de",
                "dkb.de",
                "comdirect.de",
                "postbank.de",
                "consorsbank.de",
                "traderepublic.com",
                "chase.com",
                "bankofamerica.com",
                "wellsfargo.com",
                "capitalone.com",
                "citi.com",
                "americanexpress.com",
                "schwab.com",
                "fidelity.com",
            ]
            .map(String::from)
            .to_vec(),
            mcp_rewind: true,
            mcp_workflows: true,
            onboarded: false,
            live: Live::default(),
            retention_days: Some(90),
            storage_cap_mb: Some(5000),
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

    /// `url` is on an excluded site or one of its subdomains.
    pub fn is_excluded_site(&self, url: &str) -> bool {
        let (host, _) = split_url(url);
        !host.is_empty()
            && self.excluded_sites.iter().any(|site| {
                let site = site.trim().to_lowercase();
                host == site || host.ends_with(&format!(".{site}"))
            })
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

    #[test]
    fn a_site_excludes_its_subdomains_but_not_lookalikes() {
        let settings = Settings::default();
        assert!(settings.is_excluded_site("https://www.paypal.com/myaccount"));
        assert!(settings.is_excluded_site("https://banking.sparkasse.de/login"));
        assert!(!settings.is_excluded_site("https://notpaypal.com/"));
        assert!(!settings.is_excluded_site("https://github.com/paypal.com"));
        assert!(!settings.is_excluded_site(""));
    }
}
