//! Starts Sextant at login and again after a crash, through a launch agent. Quit from the menu
//! exits cleanly, so launchd leaves Sextant off until the next login.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const LABEL: &str = "app.sextant.desktop";

fn agent_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

/// The installed app, or `None` for a binary outside an app bundle such as a cargo build.
fn bundled_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let in_bundle = exe
        .ancestors()
        .nth(3)
        .is_some_and(|bundle| bundle.extension().is_some_and(|ext| ext == "app"));
    in_bundle.then_some(exe)
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn agent(exe: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>LimitLoadToSessionType</key>
  <string>Aqua</string>
  <key>ProcessType</key>
  <string>Interactive</string>
</dict>
</plist>
"#,
        escape(&exe.to_string_lossy())
    )
}

/// Writes the launch agent for the installed app. Dev builds and cargo builds never start at
/// login.
pub fn enable() -> Result<()> {
    let Some(exe) = bundled_exe().filter(|_| !cfg!(debug_assertions)) else {
        return Ok(());
    };
    let path = agent_path()?;
    let text = agent(&exe);
    if fs::read_to_string(&path).is_ok_and(|old| old == text) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("creating ~/Library/LaunchAgents")?;
    }
    fs::write(&path, text).context("writing the launch agent")
}

/// Removes the launch agent, for the uninstall.
pub fn disable() -> Result<()> {
    match fs::remove_file(agent_path()?) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
            Err(err).context("removing the launch agent")
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_runs_the_app_at_login_and_after_a_crash_only() {
        let text = agent(Path::new("/Applications/A & B.app/Contents/MacOS/sextant"));
        assert!(
            text.contains("<string>/Applications/A &amp; B.app/Contents/MacOS/sextant</string>")
        );
        assert!(text.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(text.contains("<key>SuccessfulExit</key>\n    <false/>"));
    }
}
