//! The `sextant` commands (plan.md §9). Each one sends a request to the running app over the
//! control socket and prints the answer. `connect` needs no app: it only writes config files.

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use sextant_core::agent::AgentName;
use sextant_core::connect::claude_code_command;
use sextant_core::{Live, Sent, Settings};

use crate::agent::AgentStatus;
use crate::commands::{Models, Permission};
use crate::control::{Request, Status, call};
use crate::platform::SettingsPane;
use crate::{clock, connect, tui};

pub const COMMANDS: [&str; 14] = [
    "status",
    "setup",
    "connect",
    "pause",
    "resume",
    "audio",
    "routines",
    "agent",
    "sent",
    "privacy",
    "debug",
    "uninstall",
    "tui",
    "help",
];

const HELP: &str = "\
Usage: sextant <command>

  status                 What Sextant captures right now
  setup                  Grant access and start capture
  connect                Add Sextant to Claude Desktop and print the Claude Code command
  pause | resume         Pause capture for an hour, or resume it
  audio on|off           Transcribe calls, or stop. The first time downloads the speech models
  routines               The routines your agent saved, with their hours a week
  agent                  The agent bridge: which agent, when it last ran
  agent use claude|codex [--weekly]
  agent off
  agent run              Run your agent now; it reads your weeks and saves routines
  sent                   What agents received, newest first
  privacy                What is captured, what agents can read, how long data stays
  privacy keep 30|90|365|forever
  privacy cap 1|2|5|10|none            Storage cap in GB
  privacy forget 15m|1h|1d             Delete what was captured recently
  privacy exclude-app|include-app <name>
  privacy exclude-site|include-site <domain>
  privacy capture metadata|ocr|copy|actions|browsers on|off
  privacy agents screen-text|activities on|off
  privacy delete-everything
  debug                  The debug report, to paste into a GitHub issue
  uninstall              Delete everything, disconnect Claude Desktop, move Sextant to the Trash
  tui                    The same, in one screen
  mcp                    The MCP server over stdio, for your agent
";

pub fn run(dir: &Path, args: &[String]) -> Result<()> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["status"] => print_status(&call(dir, &Request::Status)?),
        ["setup"] => setup(dir)?,
        ["connect"] => connect()?,
        ["pause"] => print_status(&call(dir, &Request::Pause)?),
        ["resume"] => print_status(&call(dir, &Request::Resume)?),
        ["audio", "on"] => audio_on(dir)?,
        ["audio", "off"] => {
            let _: Settings = call(dir, &Request::Audio { on: false })?;
            println!("Call audio is off.");
        }
        ["routines"] => print_routines(&call(dir, &Request::Agent)?),
        ["agent"] => print_agent(&call(dir, &Request::Agent)?),
        ["agent", "use", name, rest @ ..] => {
            let agent = AgentName::parse(name).context("choose claude or codex")?;
            let weekly = match rest {
                [] => false,
                ["--weekly"] => true,
                _ => bail!("usage: sextant agent use claude|codex [--weekly]"),
            };
            let _: Settings = call(
                dir,
                &Request::ChooseAgent {
                    agent: Some(agent),
                    weekly,
                },
            )?;
            print_agent(&call(dir, &Request::Agent)?);
        }
        ["agent", "off"] => {
            let _: Settings = call(
                dir,
                &Request::ChooseAgent {
                    agent: None,
                    weekly: false,
                },
            )?;
            println!("The agent bridge is off.");
        }
        ["agent", "run"] => {
            println!("Your agent is reading your weeks. This can take a few minutes.");
            print_routines(&call(dir, &Request::RunAgent)?);
        }
        ["sent"] => print_sent(&call::<Vec<Sent>>(dir, &Request::Sent)?),
        ["privacy", rest @ ..] => privacy(dir, rest)?,
        ["debug"] => println!("{}", call::<String>(dir, &Request::Debug)?),
        ["uninstall"] => {
            confirm(
                "This deletes everything Sextant captured, disconnects Claude Desktop and moves Sextant to the Trash.",
            )?;
            let _: () = call(dir, &Request::Uninstall)?;
            println!("Sextant is uninstalled.");
        }
        ["tui"] => tui::run(dir)?,
        [] | ["help" | "--help" | "-h"] => print!("{HELP}"),
        _ => bail!("unknown command: sextant {}\n\n{HELP}", words.join(" ")),
    }
    Ok(())
}

fn privacy(dir: &Path, words: &[&str]) -> Result<()> {
    let change = |request: Request| -> Result<()> {
        let settings: Settings = call(dir, &request)?;
        print_privacy(&settings, None);
        Ok(())
    };
    match words {
        [] => {
            let settings: Settings = call(dir, &Request::Settings)?;
            let status: Status = call(dir, &Request::Status)?;
            print_privacy(&settings, Some(status.storage_bytes));
        }
        ["keep", days] => change(Request::Retention {
            days: parse_choice(days, "forever")?,
        })?,
        ["cap", gb] => change(Request::StorageCap {
            mb: parse_choice(gb, "none")?.map(|gb| gb * 1000),
        })?,
        ["forget", span] => {
            let minutes = match *span {
                "15m" => 15,
                "1h" => 60,
                "1d" => 1440,
                _ => bail!("choose 15m, 1h or 1d"),
            };
            let _: () = call(dir, &Request::Forget { minutes })?;
            println!("Deleted what was captured in the last {span}.");
        }
        ["exclude-app" | "include-app", name @ ..] if !name.is_empty() => {
            let name = name.join(" ");
            let mut apps = call::<Settings>(dir, &Request::Settings)?.excluded_apps;
            toggle(&mut apps, &name, words[0] == "exclude-app");
            change(Request::ExcludedApps { apps })?;
        }
        ["exclude-site" | "include-site", site] => {
            let mut sites = call::<Settings>(dir, &Request::Settings)?.excluded_sites;
            toggle(&mut sites, &site.to_lowercase(), words[0] == "exclude-site");
            change(Request::ExcludedSites { sites })?;
        }
        ["capture", source, switch] => {
            let on = parse_switch(switch)?;
            let mut live = call::<Settings>(dir, &Request::Settings)?.live;
            match *source {
                "metadata" => live.metadata = on,
                "ocr" => live.ocr = on,
                "copy" => live.copy = on,
                "actions" => live.actions = on,
                "browsers" => live.opaque_browsers = on,
                _ => bail!("choose metadata, ocr, copy, actions or browsers"),
            }
            change(Request::Capture { live })?;
        }
        ["agents", group, switch] => {
            let on = parse_switch(switch)?;
            let settings: Settings = call(dir, &Request::Settings)?;
            let (rewind, workflows) = match *group {
                "screen-text" => (on, settings.mcp_workflows),
                "activities" => (settings.mcp_rewind, on),
                _ => bail!("choose screen-text or activities"),
            };
            change(Request::Mcp { rewind, workflows })?;
        }
        ["delete-everything"] => {
            confirm("This deletes everything Sextant captured, its settings and its key.")?;
            let _: () = call(dir, &Request::DeleteEverything)?;
            println!("Everything is deleted. Sextant starts again; run `sextant setup`.");
        }
        _ => bail!(
            "unknown command: sextant privacy {}\n\n{HELP}",
            words.join(" ")
        ),
    }
    Ok(())
}

/// `None` for the word that means "no limit", else the number.
fn parse_choice(text: &str, unlimited: &str) -> Result<Option<u32>> {
    if text == unlimited {
        return Ok(None);
    }
    text.parse()
        .map(Some)
        .with_context(|| format!("{text} is not a number"))
}

fn parse_switch(text: &str) -> Result<bool> {
    match text {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => bail!("choose on or off"),
    }
}

fn toggle(list: &mut Vec<String>, item: &str, present: bool) {
    list.retain(|entry| !entry.eq_ignore_ascii_case(item));
    if present {
        list.push(item.to_owned());
    }
}

fn confirm(warning: &str) -> Result<()> {
    println!("{warning}\nType \"delete\" to go on:");
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    if line.trim() != "delete" {
        bail!("nothing was deleted");
    }
    Ok(())
}

fn wait_for_enter(prompt: &str) -> Result<()> {
    print!("{prompt} ");
    io::stdout().flush()?;
    io::stdin().lock().read_line(&mut String::new())?;
    Ok(())
}

fn pane(name: &str) -> Option<SettingsPane> {
    match name {
        "screen-recording" => Some(SettingsPane::ScreenRecording),
        "accessibility" => Some(SettingsPane::Accessibility),
        _ => None,
    }
}

fn permission_label(name: &str) -> &str {
    match name {
        "screen-recording" => "Screen Recording, to read the text on screen",
        "accessibility" => "Accessibility, to read the focused app, window and controls",
        "automation" => "Automation, to read the address of the browser tab",
        other => other,
    }
}

/// Walks through each permission capture needs, then starts capture. Screen Recording applies
/// only after Sextant opens again, so a new grant relaunches it.
fn setup(dir: &Path) -> Result<()> {
    let mut relaunch = false;
    for permission in call::<Vec<Permission>>(dir, &Request::Permissions)? {
        let label = permission_label(&permission.name);
        if permission.status == "granted" {
            println!("Allowed: {label}.");
            continue;
        }
        match pane(&permission.name) {
            Some(pane) => {
                println!("Sextant needs {label}.");
                let _: () = call(dir, &Request::Allow { pane })?;
                wait_for_enter(
                    "System Settings is open: drag Sextant into the list, then press Enter.",
                )?;
                relaunch |= pane == SettingsPane::ScreenRecording;
            }
            None => {
                println!("Sextant needs {label}. Click Allow in the prompt.");
                let granted: bool = call(dir, &Request::AskAutomation)?;
                println!(
                    "{}",
                    if granted {
                        "Allowed."
                    } else {
                        "Not allowed. Capture still works without the tab address."
                    }
                );
            }
        }
    }
    let _: Settings = call(dir, &Request::FinishSetup)?;
    println!("Capture is on.");
    if relaunch {
        let _: () = call(dir, &Request::Relaunch)?;
        println!("Sextant opens again so Screen Recording applies.");
    }
    println!("Next: `sextant connect` to let Claude read your weeks.");
    Ok(())
}

fn connect() -> Result<()> {
    let binary = std::env::current_exe().context("finding the Sextant binary")?;
    let binary = binary.to_string_lossy();
    let written =
        connect::write_desktop_config(&connect::desktop_config_path()?, &binary, clock::now())?;
    println!(
        "Claude Desktop: added to {}. Restart Claude Desktop.",
        written.path
    );
    if let Some(backup) = written.backup {
        println!("The old config is kept at {backup}.");
    }
    println!("Claude Code: run\n  {}", claude_code_command(&binary));
    Ok(())
}

const AUDIO_CONSENT: &str = "\
When a call app such as Zoom, Teams or Meet uses your microphone, Sextant writes down what you
and the other side say. The sound is never saved: only the text stays, on this Mac.

In Germany, recording a call without the other side knowing can be a crime (section 201 StGB).
Tell everyone on a call that you transcribe it.

Speech recognition: Parakeet TDT 0.6B v3 by NVIDIA (CC BY 4.0, ONNX version by istupakov).
Voice detection: Silero VAD (MIT). Runtime: ONNX Runtime (MIT).";

/// The first time, asks before the one-time model download and waits for it, then switches
/// audio on.
fn audio_on(dir: &Path) -> Result<()> {
    let models: Models = call(dir, &Request::Models)?;
    if !models.installed {
        println!("{AUDIO_CONSENT}\n");
        println!(
            "This needs a one-time download of about {} MB from github.com and huggingface.co. Sextant makes no other network connection.",
            models.bytes / 1_000_000
        );
        wait_for_enter("Press Enter to download, or Ctrl-C to stop.")?;
        download(dir)?;
    }
    let _: Settings = call(dir, &Request::Audio { on: true })?;
    println!("Call audio is on.");
    Ok(())
}

fn download(dir: &Path) -> Result<()> {
    let _: Models = call(dir, &Request::DownloadModels)?;
    loop {
        thread::sleep(Duration::from_millis(500));
        let models: Models = call(dir, &Request::Models)?;
        let state = &models.download;
        if state.running {
            print!(
                "\r{} of {} MB",
                state.done / 1_000_000,
                state.total / 1_000_000
            );
            io::stdout().flush()?;
            continue;
        }
        println!();
        if let Some(failed) = &state.failed {
            bail!("the download failed: {failed}");
        }
        if !models.installed {
            bail!("the download stopped");
        }
        return Ok(());
    }
}

pub fn status_lines(status: &Status) -> Vec<String> {
    let mut lines = vec![status.line.clone()];
    if let Some(problem) = &status.problem {
        lines.push(format!("Sextant did not start: {problem}"));
    }
    if !status.set_up {
        lines.push("Setup is not finished: run `sextant setup`.".into());
    }
    if let Some((ts, app)) = &status.last_capture {
        lines.push(format!("Last capture: {} in {app}", clock::date_time(*ts)));
    }
    lines.push(format!("Data on this Mac: {}", size(status.storage_bytes)));
    lines
}

fn print_status(status: &Status) {
    for line in status_lines(status) {
        println!("{line}");
    }
}

pub fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1_000_000_000 => format!("{:.1} GB", b as f64 / 1e9),
        b if b >= 1_000_000 => format!("{} MB", b / 1_000_000),
        b => format!("{} KB", b.div_ceil(1000)),
    }
}

pub fn agent_label(agent: AgentName) -> &'static str {
    match agent {
        AgentName::Claude => "Claude Code",
        AgentName::Codex => "Codex",
    }
}

/// The line that has your agent turn a routine into a skill. Claude Code runs MCP prompts as
/// slash commands; Codex gets the request in words and finds `check_skill` on its own.
pub fn skill_prompt(agent: AgentName, id: i64) -> String {
    match agent {
        AgentName::Claude => format!("/mcp__sextant__write_skill {id}"),
        AgentName::Codex => format!("Write a skill for Sextant routine {id}."),
    }
}

pub fn agent_lines(status: &AgentStatus) -> Vec<String> {
    let installed: Vec<&str> = status.installed.iter().map(|a| agent_label(*a)).collect();
    let mut lines = vec![match status.chosen {
        Some(agent) if status.weekly => format!("Agent: {}, runs weekly", agent_label(agent)),
        Some(agent) => format!("Agent: {}", agent_label(agent)),
        None => "Agent bridge: off. Run `sextant agent use claude` or `codex`.".into(),
    }];
    lines.push(if installed.is_empty() {
        "Installed: neither Claude Code nor Codex".into()
    } else {
        format!("Installed: {}", installed.join(", "))
    });
    lines.push(match (status.last_run, status.last_agent) {
        (Some(ts), Some(agent)) => {
            format!(
                "Last run: {} with {}",
                clock::date_time(ts),
                agent_label(agent)
            )
        }
        _ => "Last run: never".into(),
    });
    if status.running {
        lines.push("Running now.".into());
    }
    lines
}

fn print_agent(status: &AgentStatus) {
    for line in agent_lines(status) {
        println!("{line}");
    }
}

pub fn routines(status: &AgentStatus) -> &[Value] {
    status.routines.as_array().map_or(&[], Vec::as_slice)
}

pub fn routine_lines(routine: &Value, agent: Option<AgentName>) -> Vec<String> {
    let text = |key: &str| routine[key].as_str().unwrap_or_default().to_owned();
    let id = routine["id"].as_i64().unwrap_or_default();
    let mut lines = vec![format!(
        "#{id} {}: {:.1} h a week, {} activities",
        text("title"),
        routine["hours_week"].as_f64().unwrap_or_default(),
        routine["activities"].as_i64().unwrap_or_default()
    )];
    for (label, key) in [
        ("How", "how"),
        ("When", "trigger"),
        ("Ask first", "approval"),
        ("Why", "why"),
    ] {
        let value = text(key);
        if !value.is_empty() {
            lines.push(format!("  {label}: {value}"));
        }
    }
    if let Some(agent) = agent {
        lines.push(format!("  Skill: {}", skill_prompt(agent, id)));
    }
    lines
}

pub fn routines_total(status: &AgentStatus) -> String {
    let routines = routines(status);
    let hours: f64 = routines
        .iter()
        .map(|routine| routine["hours_week"].as_f64().unwrap_or_default())
        .sum();
    format!("{hours:.1} h a week in {} routines", routines.len())
}

fn print_routines(status: &AgentStatus) {
    if routines(status).is_empty() {
        println!("No routines yet. Run `sextant agent run` to have your agent look for them.");
        return;
    }
    println!("{}\n", routines_total(status));
    for routine in routines(status) {
        for line in routine_lines(routine, status.chosen) {
            println!("{line}");
        }
        println!();
    }
}

pub fn sent_line(sent: &Sent) -> String {
    format!(
        "{}  {:<16} {:<14} {}",
        clock::date_time(sent.ts),
        sent.client,
        sent.tool,
        size(sent.bytes as u64)
    )
}

fn print_sent(sent: &[Sent]) {
    if sent.is_empty() {
        println!("No agent has received anything yet.");
    }
    for row in sent {
        println!("{}", sent_line(row));
    }
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

pub fn privacy_lines(settings: &Settings, used: Option<u64>) -> Vec<String> {
    let Live {
        metadata,
        ocr,
        copy,
        actions,
        audio,
        opaque_browsers,
    } = settings.live;
    let mut lines = vec![
        format!(
            "Capture: apps and windows {}, screen text {}, copies {}, actions {}, call audio {}, browsers without private-window info {}",
            on_off(metadata),
            on_off(ocr),
            on_off(copy),
            on_off(actions),
            on_off(audio),
            on_off(opaque_browsers)
        ),
        format!(
            "Agents can read: screen text and search {}, activities and routines {}",
            on_off(settings.mcp_rewind),
            on_off(settings.mcp_workflows)
        ),
        format!(
            "Keep data: {}. Storage cap: {}.",
            settings
                .retention_days
                .map_or("forever".into(), |days| format!("{days} days")),
            settings
                .storage_cap_mb
                .map_or("none".into(), |mb| format!("{} GB", mb / 1000))
        ),
    ];
    if let Some(bytes) = used {
        lines.push(format!("Data on this Mac: {}", size(bytes)));
    }
    lines.push(format!(
        "Excluded apps: {}",
        settings.excluded_apps.join(", ")
    ));
    lines.push(format!(
        "Excluded sites: {}",
        settings.excluded_sites.join(", ")
    ));
    lines
}

fn print_privacy(settings: &Settings, used: Option<u64>) {
    for line in privacy_lines(settings, used) {
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn including_an_app_removes_it_whatever_its_case_and_excluding_adds_it_once() {
        let mut apps = vec!["Signal".to_owned(), "Health".to_owned()];
        toggle(&mut apps, "signal", false);
        assert_eq!(apps, ["Health"]);
        toggle(&mut apps, "Slack", true);
        toggle(&mut apps, "slack", true);
        assert_eq!(apps, ["Health", "slack"]);
    }

    #[test]
    fn choices_take_numbers_or_the_word_for_no_limit() {
        assert_eq!(parse_choice("90", "forever").expect("number"), Some(90));
        assert_eq!(parse_choice("forever", "forever").expect("word"), None);
        assert!(parse_choice("soon", "forever").is_err());
    }

    #[test]
    fn sizes_read_like_the_window_showed_them() {
        assert_eq!(size(1_234_000_000), "1.2 GB");
        assert_eq!(size(56_000_000), "56 MB");
        assert_eq!(size(1_500), "2 KB");
    }

    #[test]
    fn a_routine_prints_its_hours_fields_and_the_skill_line() {
        let routine = serde_json::json!({
            "id": 4, "title": "Send invoices", "hours_week": 2.5, "activities": 12,
            "how": "Copy the amount into lexoffice", "trigger": "", "approval": "", "why": "Same steps each time",
        });
        assert_eq!(
            routine_lines(&routine, Some(AgentName::Claude)),
            [
                "#4 Send invoices: 2.5 h a week, 12 activities",
                "  How: Copy the amount into lexoffice",
                "  Why: Same steps each time",
                "  Skill: /mcp__sextant__write_skill 4",
            ]
        );
    }
}
