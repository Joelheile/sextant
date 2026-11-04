//! Starts Sextant at login and again after a crash, through a launch agent. Opening the app from
//! Finder hands it to the agent too, so launchd always runs the one copy. Quit from the menu
//! exits cleanly, so launchd leaves Sextant off until the app is opened again or the next login.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, ensure};

const LABEL: &str = "app.sextant.desktop";

/// The exit code that asks launchd to start Sextant again (`EX_TEMPFAIL`). `KeepAlive` restarts
/// any exit that is not a success.
pub(crate) const RELAUNCH_CODE: i32 = 75;

/// Whether this process is the launch agent's job. launchd sets `XPC_SERVICE_NAME` to the job
/// label; an app opened from Finder or `open` gets another name.
pub(crate) fn started_by_launchd() -> bool {
    is_agent_job(std::env::var("XPC_SERVICE_NAME").ok().as_deref())
}

fn is_agent_job(service: Option<&str>) -> bool {
    service == Some(LABEL)
}

fn agent_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

/// The installed app, or `None` for a dev build or a binary outside an app bundle such as a
/// cargo build.
pub(crate) fn installed_exe() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        return None;
    }
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

/// Writes the launch agent to `path` and returns whether its text changed.
fn write_agent(path: &Path, text: &str) -> Result<bool> {
    if fs::read_to_string(path).is_ok_and(|old| old == text) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("creating ~/Library/LaunchAgents")?;
    }
    fs::write(path, text).context("writing the launch agent")?;
    Ok(true)
}

/// Starts the installed app through its launch agent, loading the agent first if launchd does not
/// know it yet, and returns whether it did. A copy opened from Finder runs outside launchd, so
/// nothing would start it again after a crash. Dev builds and cargo builds run in place.
pub fn hand_to_launchd() -> Result<bool> {
    let Some(exe) = installed_exe().filter(|_| !started_by_launchd()) else {
        return Ok(false);
    };
    let path = agent_path()?;
    let changed = write_agent(&path, &agent(&exe))?;
    // SAFETY: getuid has no preconditions and cannot fail.
    let domain = format!("gui/{}", unsafe { libc::getuid() });
    let job = format!("{domain}/{LABEL}");
    let loaded = launchctl(&["print", &job]).is_ok();
    // launchd keeps the definition it loaded, so after a move it would start the old path.
    if loaded && changed {
        launchctl(&["bootout", &job])?;
    }
    if !loaded || changed {
        launchctl(&["bootstrap", &domain, &path.to_string_lossy()])?;
    }
    launchctl(&["kickstart", &job])?;
    Ok(true)
}

fn launchctl(args: &[&str]) -> Result<()> {
    let status = Command::new("/bin/launchctl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("running launchctl")?;
    ensure!(
        status.success(),
        "launchctl {} failed: {status}",
        args.join(" ")
    );
    Ok(())
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

    #[test]
    fn writing_the_agent_reports_whether_it_changed() {
        let dir = std::env::temp_dir().join(format!("sextant-agent-{}", std::process::id()));
        let path = dir.join("LaunchAgents/agent.plist");
        assert!(write_agent(&path, "old path").expect("creates"));
        assert!(!write_agent(&path, "old path").expect("keeps"));
        assert!(write_agent(&path, "new path").expect("replaces"));
        assert_eq!(fs::read_to_string(&path).expect("reads"), "new path");
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn only_the_launch_agent_job_counts_as_started_by_launchd() {
        assert!(is_agent_job(Some("app.sextant.desktop")));
        assert!(!is_agent_job(Some(
            "application.app.sextant.desktop.1234.5678"
        )));
        assert!(!is_agent_job(Some("0")));
        assert!(!is_agent_job(None));
    }
}
