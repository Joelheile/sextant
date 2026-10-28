//! Browser address and private-window state through AppleScript, with a 2 s kill timeout
//! (plan.md §4). Automation permission is checked first without asking, so a look never shows
//! the system prompt: only onboarding asks.

use std::ffi::c_void;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use sextant_core::capture::Browser;

use crate::capture::Tab;

const TIMEOUT: Duration = Duration::from_secs(2);

const TYPE_BUNDLE_ID: u32 = u32::from_be_bytes(*b"bund");
const TYPE_WILDCARD: u32 = u32::from_be_bytes(*b"****");

#[repr(C)]
struct AEDesc {
    descriptor_type: u32,
    data: *mut c_void,
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn AECreateDesc(kind: u32, data: *const c_void, size: isize, result: *mut AEDesc) -> i16;
    fn AEDisposeDesc(desc: *mut AEDesc) -> i16;
    fn AEDeterminePermissionToAutomateTarget(
        target: *const AEDesc,
        class: u32,
        id: u32,
        ask: u8,
    ) -> i32;
}

/// The Automation permission for `bundle_id`: `Some(true)` granted, `Some(false)` denied,
/// `None` not decided yet or the app is not running. `ask` shows the system prompt when undecided.
pub fn permission(bundle_id: &str, ask: bool) -> Option<bool> {
    let mut target = AEDesc {
        descriptor_type: 0,
        data: std::ptr::null_mut(),
    };
    // SAFETY: the bytes are valid for the call and the descriptor is disposed below.
    let created = unsafe {
        AECreateDesc(
            TYPE_BUNDLE_ID,
            bundle_id.as_ptr().cast(),
            bundle_id.len() as isize,
            &mut target,
        )
    };
    if created != 0 {
        return None;
    }
    // SAFETY: `target` was created above.
    let status = unsafe {
        AEDeterminePermissionToAutomateTarget(&target, TYPE_WILDCARD, TYPE_WILDCARD, ask as u8)
    };
    // SAFETY: `target` was created above and is not used again.
    unsafe { AEDisposeDesc(&mut target) };
    match status {
        0 => Some(true),
        -1743 => Some(false),
        _ => None,
    }
}

fn script(bundle_id: &str, browser: Browser) -> Result<String> {
    if !bundle_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        bail!("unexpected bundle id");
    }
    let tell = format!(r#"tell application id "{bundle_id}" to return "#);
    Ok(match browser {
        Browser::Chromium => {
            tell + "(URL of active tab of front window) & linefeed & (mode of front window)"
        }
        Browser::Arc => {
            tell + "(URL of active tab of front window) & linefeed & (incognito of front window)"
        }
        Browser::Opaque if bundle_id == "com.apple.Safari" => {
            tell + "(URL of current tab of front window)"
        }
        Browser::Opaque => bail!("this browser has no script for its address"),
    })
}

/// Reads the front window of the browser. An error means the window could not be told apart from
/// a private one, and the caller treats that as private.
pub fn tab(bundle_id: &str, browser: Browser) -> Result<Tab> {
    if permission(bundle_id, false) != Some(true) {
        bail!("Automation is not granted");
    }
    let script = script(bundle_id, browser)?;
    let mut child = Command::new("osascript")
        .args(["-e", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("running osascript")?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().context("waiting for osascript")? {
            break status;
        }
        if started.elapsed() >= TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            bail!("the browser did not answer in time");
        }
        thread::sleep(Duration::from_millis(10));
    };
    if !status.success() {
        bail!("the browser refused");
    }
    let output = child.wait_with_output().context("reading osascript")?;
    parse(&String::from_utf8_lossy(&output.stdout), browser)
}

fn parse(output: &str, browser: Browser) -> Result<Tab> {
    let mut lines = output.lines();
    let url = lines.next().unwrap_or_default().trim().to_string();
    let private = match (browser, lines.next().map(str::trim)) {
        (Browser::Chromium, Some("normal")) | (Browser::Arc, Some("false")) => false,
        (Browser::Chromium, Some(_)) | (Browser::Arc, Some(_)) => true,
        // Safari cannot say. Only reached after the user opted in.
        (Browser::Opaque, None) => false,
        _ => bail!("unexpected answer"),
    };
    Ok(Tab { url, private })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_window_is_not_private() {
        let tab = parse("https://example.com/a\nnormal\n", Browser::Chromium).expect("parses");
        assert_eq!(
            (tab.url.as_str(), tab.private),
            ("https://example.com/a", false)
        );
        assert!(
            !parse("https://example.com\nfalse\n", Browser::Arc)
                .expect("parses")
                .private
        );
    }

    #[test]
    fn anything_but_a_plain_normal_window_counts_as_private() {
        assert!(
            parse("https://example.com\nincognito\n", Browser::Chromium)
                .expect("parses")
                .private
        );
        assert!(
            parse("https://example.com\ntrue\n", Browser::Arc)
                .expect("parses")
                .private
        );
        assert!(
            parse("https://example.com\nsomething new\n", Browser::Chromium)
                .expect("parses")
                .private
        );
    }

    #[test]
    fn a_missing_mode_line_fails_closed() {
        assert!(parse("https://example.com\n", Browser::Chromium).is_err());
        assert!(parse("", Browser::Arc).is_err());
    }

    #[test]
    fn bundle_ids_cannot_inject_script() {
        assert!(script(r#"com.x" & (do shell script "x") & ""#, Browser::Chromium).is_err());
        assert!(script("com.google.Chrome", Browser::Chromium).is_ok());
        assert!(script("org.mozilla.firefox", Browser::Opaque).is_err());
    }
}
