//! The agent bridge: runs your own Claude Code (`claude -p`) or Codex (`codex exec`) under your
//! own login and asks it for automation ideas on the redacted step graph (see
//! `sextant_core::agent`). Sextant opens no connection itself: the CLI does, only after you pick
//! one. The prompt goes in on stdin and the CLI runs in an empty temporary folder. Claude runs
//! with no tools. Codex has no mode without tools: its shell can still read files, but only in
//! a read-only sandbox with the network off, without your Codex config or rules, and with web
//! search, apps, plugins, the browser, computer use and hooks turned off.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use sextant_core::Settings;
use sextant_core::agent::{AgentName, PROMPT, Run, SCHEMA, parse_ideas};
use tauri::{AppHandle, Manager, State};

use crate::clock;
use crate::commands::{AppState, Reply, msg};
use crate::engine::Engine;

const TIMEOUT: Duration = Duration::from_secs(10 * 60);
const WEEK_S: i64 = 7 * 86_400;
const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);
/// Apps started from Finder get a short PATH, so the usual install folders are searched too.
const HOME_DIRS: [&str; 5] = [
    ".local/bin",
    ".claude/local",
    ".npm-global/bin",
    ".bun/bin",
    ".volta/bin",
];
const SYSTEM_DIRS: [&str; 3] = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"];

static RUNNING: AtomicBool = AtomicBool::new(false);

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.extend(HOME_DIRS.iter().map(|dir| Path::new(&home).join(dir)));
    }
    dirs.extend(SYSTEM_DIRS.iter().map(PathBuf::from));
    dirs.dedup();
    dirs
}

fn find(agent: AgentName) -> Option<PathBuf> {
    search_dirs()
        .into_iter()
        .map(|dir| dir.join(agent.as_str()))
        .find(|path| path.is_file())
}

/// Runs `agent` on the brief and returns its answer text.
fn ask(agent: AgentName, brief: &str) -> Result<String> {
    let path = find(agent).with_context(|| format!("{agent} is not installed"))?;
    let dir = std::env::temp_dir().join(format!(
        "sextant-agent-{}-{}",
        std::process::id(),
        clock::now()
    ));
    std::fs::create_dir_all(&dir).context("creating the agent folder")?;
    let answer = run(agent, &path, &dir, brief);
    let _ = std::fs::remove_dir_all(&dir);
    answer
}

fn run(agent: AgentName, path: &Path, dir: &Path, brief: &str) -> Result<String> {
    let mut command = Command::new(path);
    match agent {
        AgentName::Claude => command.args([
            "-p",
            "--output-format",
            "json",
            "--tools",
            "",
            "--strict-mcp-config",
            "--no-session-persistence",
            "--json-schema",
            SCHEMA,
        ]),
        AgentName::Codex => {
            let schema = dir.join("schema.json");
            std::fs::write(&schema, SCHEMA).context("writing the answer schema")?;
            command
                .args([
                    "exec",
                    "--sandbox",
                    "read-only",
                    "--skip-git-repo-check",
                    "--ephemeral",
                    "--ignore-user-config",
                    "--ignore-rules",
                    "-c",
                    "web_search=\"disabled\"",
                    "-c",
                    "features.apps=false",
                    "-c",
                    "features.plugins=false",
                    "-c",
                    "features.browser_use=false",
                    "-c",
                    "features.in_app_browser=false",
                    "-c",
                    "features.computer_use=false",
                    "-c",
                    "features.hooks=false",
                    "--color",
                    "never",
                    "--output-schema",
                ])
                .arg(&schema)
                .arg("-")
        }
    };
    let path_var = std::env::join_paths(search_dirs()).context("building PATH")?;
    let mut child = command
        .current_dir(dir)
        .env("PATH", path_var)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {agent}"))?;

    let prompt = format!("{PROMPT}\n\n{brief}\n");
    let mut stdin = child.stdin.take().context("agent stdin")?;
    let writer = thread::spawn(move || stdin.write_all(prompt.as_bytes()));
    let mut stdout = child.stdout.take().context("agent stdout")?;
    let reader = thread::spawn(move || {
        let mut out = String::new();
        stdout.read_to_string(&mut out).map(|_| out)
    });
    let mut stderr = child.stderr.take().context("agent stderr")?;
    let errors = thread::spawn(move || {
        let mut out = String::new();
        let _ = stderr.read_to_string(&mut out);
        out
    });

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "{agent} took longer than {} minutes",
                TIMEOUT.as_secs() / 60
            );
        }
        thread::sleep(Duration::from_millis(250));
    };
    let _ = writer.join();
    let out = reader
        .join()
        .map_err(|_| anyhow::anyhow!("reading {agent} failed"))??;
    let err = errors.join().unwrap_or_default();
    if !status.success() {
        if signed_out(&err) {
            bail!("{agent} is signed out: open Terminal, run {agent} and sign in again");
        }
        let last = err
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("");
        let last: String = last.chars().take(200).collect();
        bail!("{agent} stopped ({status}): {last}");
    }
    match agent {
        AgentName::Claude => claude_answer(&out),
        AgentName::Codex => Ok(out),
    }
}

fn signed_out(stderr: &str) -> bool {
    let text = stderr.to_lowercase();
    ["sign in again", "not logged in", "/login", "refresh token"]
        .iter()
        .any(|hint| text.contains(hint))
}

/// `claude -p --output-format json` wraps the answer: `structured_output` with a schema, else
/// `result`.
fn claude_answer(out: &str) -> Result<String> {
    let value: Value = serde_json::from_str(out).context("reading the claude answer")?;
    if value["is_error"].as_bool() == Some(true) {
        let text: String = value["result"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(200)
            .collect();
        bail!("claude failed: {text}");
    }
    if value["structured_output"].is_object() {
        return Ok(value["structured_output"].to_string());
    }
    value["result"]
        .as_str()
        .map(str::to_owned)
        .context("the claude answer has no result")
}

/// One run: brief, ask, parse, save. Logs only counts and timings.
fn analyze(engine: &Engine, agent: AgentName) -> Result<()> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        bail!("an analysis is already running");
    }
    let result = (|| {
        let started = Instant::now();
        let (brief, workflows) =
            engine.read(|store| Ok((store.agent_brief()?, store.agent_workflows()?)))?;
        if workflows.is_empty() {
            bail!("no workflows yet; Sextant needs a few days of data first");
        }
        let answer = ask(agent, &brief)?;
        let ideas = parse_ideas(&answer, &workflows)?;
        eprintln!(
            "agent run: {} workflows in, {} ideas out, {} s",
            workflows.len(),
            ideas.len(),
            started.elapsed().as_secs()
        );
        let now = clock::now();
        engine.write(move |store| store.save_ideas(now, agent, &ideas))
    })();
    RUNNING.store(false, Ordering::SeqCst);
    result
}

#[derive(Serialize)]
pub struct IdeaView {
    workflow_id: i64,
    workflow: String,
    title: String,
    how: String,
    hours_week: f64,
    confidence: f64,
}

#[derive(Serialize)]
pub struct AgentStatus {
    /// The agents found on this Mac.
    installed: Vec<AgentName>,
    chosen: Option<AgentName>,
    weekly: bool,
    running: bool,
    last_run: Option<i64>,
    last_agent: Option<AgentName>,
    ideas: Vec<IdeaView>,
}

fn status(engine: &Engine, settings: &Settings) -> Result<AgentStatus> {
    let run: Option<Run> = engine.read(|store| store.agent_run())?;
    Ok(AgentStatus {
        installed: AgentName::ALL
            .into_iter()
            .filter(|&agent| find(agent).is_some())
            .collect(),
        chosen: settings.agent,
        weekly: settings.agent_weekly,
        running: RUNNING.load(Ordering::SeqCst),
        last_run: run.as_ref().map(|r| r.ts),
        last_agent: run.as_ref().map(|r| r.agent),
        ideas: run
            .map(|r| r.ideas)
            .unwrap_or_default()
            .into_iter()
            .map(|saved| IdeaView {
                workflow_id: saved.idea.workflow_id,
                workflow: saved.workflow,
                title: saved.idea.title,
                how: saved.idea.how,
                hours_week: saved.idea.hours_week,
                confidence: saved.idea.confidence,
            })
            .collect(),
    })
}

#[tauri::command]
pub async fn agent_status(state: State<'_, AppState>) -> Reply<AgentStatus> {
    let engine = state.engine()?;
    let settings = crate::prefs::load(&state.dir).map_err(msg)?;
    status(engine, &settings).map_err(|err| format!("{err:#}"))
}

/// Picks the agent (`None` turns the bridge off) and whether it runs weekly.
#[tauri::command]
pub async fn set_agent(
    state: State<'_, AppState>,
    agent: Option<AgentName>,
    weekly: bool,
) -> Reply<Settings> {
    state
        .engine()?
        .change_settings(move |s| {
            s.agent = agent;
            s.agent_weekly = weekly && agent.is_some();
        })
        .map_err(msg)
}

/// "Analyze now": runs the chosen agent and returns the new status.
#[tauri::command]
pub async fn run_agent(app: AppHandle) -> Reply<AgentStatus> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let engine = state.engine()?;
        let settings = crate::prefs::load(&state.dir).map_err(msg)?;
        let agent = settings.agent.ok_or("pick Claude Code or Codex first")?;
        analyze(engine, agent).map_err(|err| format!("{err:#}"))?;
        status(engine, &settings).map_err(|err| format!("{err:#}"))
    })
    .await
    .map_err(msg)?
}

/// Runs the agent once a week when the user switched that on.
pub fn spawn_weekly(app: AppHandle) -> Result<()> {
    thread::Builder::new()
        .name("sextant-agent".into())
        .spawn(move || {
            loop {
                thread::sleep(CHECK_EVERY);
                let state = app.state::<AppState>();
                let (Ok(engine), Ok(settings)) = (state.engine(), crate::prefs::load(&state.dir))
                else {
                    continue;
                };
                let Some(agent) = settings.agent.filter(|_| settings.agent_weekly) else {
                    continue;
                };
                let due = engine
                    .read(|store| store.agent_run())
                    .is_ok_and(|run| run.is_none_or(|r| r.ts < clock::now() - WEEK_S));
                if due && analyze(engine, agent).is_err() {
                    eprintln!("weekly agent run failed");
                }
            }
        })
        .context("starting the agent thread")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use sextant_core::fixtures::{TempDb, WEEK_NOW, load, local, week};

    use super::*;

    /// Calls the real CLI on the fixture week, so it costs a little of your plan. Run with
    /// `cargo test -p sextant-app live_ -- --ignored --nocapture`.
    fn live(agent: AgentName) {
        let db = TempDb::new(&format!("agent-{agent}"));
        let mut store = db.open().expect("test setup");
        load(&mut store, &week(), WEEK_NOW).expect("test setup");
        store.refresh(local(WEEK_NOW)).expect("test setup");
        let (brief, workflows) = (
            store.agent_brief().expect("brief"),
            store.agent_workflows().expect("workflows"),
        );
        assert!(!workflows.is_empty());
        let started = Instant::now();
        let answer = ask(agent, &brief).expect("agent answer");
        let ideas = parse_ideas(&answer, &workflows).expect("ideas");
        eprintln!("{agent}: {} ideas in {:?}", ideas.len(), started.elapsed());
        for idea in &ideas {
            eprintln!(
                "  {:.1} h  {}  |  {}",
                idea.hours_week, idea.title, idea.how
            );
        }
        assert!(!ideas.is_empty());
    }

    #[test]
    fn a_revoked_login_reads_as_signed_out() {
        assert!(signed_out(
            "ERROR codex_login::auth::manager: Failed to refresh token: Please log out and sign in again"
        ));
        assert!(!signed_out("error: unexpected argument '--foo'"));
    }

    #[test]
    #[ignore = "calls the claude CLI"]
    fn live_claude_returns_ideas() {
        live(AgentName::Claude);
    }

    #[test]
    #[ignore = "calls the codex CLI"]
    fn live_codex_returns_ideas() {
        live(AgentName::Codex);
    }
}
