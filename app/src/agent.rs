//! The agent bridge: runs your own Claude Code (`claude -p`) or Codex (`codex exec`) under your
//! own login with the `find_routines` prompt. The agent reads your weeks through Sextant's MCP
//! tools and saves routines with `save_routine`, so every answer it gets has a row in the sent
//! log. Sextant opens no connection itself: the CLI does, only after you pick one. The CLI runs
//! in an empty temporary folder with Sextant as its only MCP server. Claude gets no other tools.
//! Codex has no mode without tools: its shell can still read files, but only in a read-only
//! sandbox with the network off, without your Codex config or rules, and with web search, apps,
//! plugins, the browser, computer use and hooks turned off.

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
use sextant_core::agent::{AgentName, Run};
use sextant_core::mcp::FIND_ROUTINES;
use tauri::{AppHandle, Manager, State};

use crate::clock;
use crate::commands::{AppState, Reply, msg};
use crate::engine::Engine;

const TIMEOUT: Duration = Duration::from_secs(15 * 60);
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
/// Codex starts MCP servers with a short environment, so these reach `sextant mcp` by name. Only
/// debug builds read them.
const SERVER_ENV: &str = r#"["SEXTANT_HOME", "SEXTANT_DEV_KEY", "SEXTANT_NOW"]"#;

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

/// Runs `agent` on `prompt` with `server` (the `sextant` binary) as its only MCP server, and
/// returns what it printed.
fn ask(agent: AgentName, server: &Path, prompt: &str) -> Result<String> {
    let path = find(agent).with_context(|| format!("{agent} is not installed"))?;
    let dir = std::env::temp_dir().join(format!(
        "sextant-agent-{}-{}",
        std::process::id(),
        clock::now()
    ));
    std::fs::create_dir_all(&dir).context("creating the agent folder")?;
    let answer = run(agent, &path, &dir, server, prompt);
    let _ = std::fs::remove_dir_all(&dir);
    answer
}

fn run(agent: AgentName, path: &Path, dir: &Path, server: &Path, prompt: &str) -> Result<String> {
    let server = server.to_str().context("the Sextant path is not UTF-8")?;
    let mut command = Command::new(path);
    match agent {
        AgentName::Claude => command
            .args([
                "-p",
                "--output-format",
                "json",
                "--tools",
                "",
                "--allowedTools",
                "mcp__sextant",
                "--strict-mcp-config",
                "--no-session-persistence",
                "--mcp-config",
            ])
            .arg(sextant_core::connect::claude_desktop_config("", server)?),
        AgentName::Codex => command
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
                "-c",
            ])
            .arg(format!(
                "mcp_servers.sextant.command={}",
                Value::from(server)
            ))
            .args([
                "-c",
                "mcp_servers.sextant.args=[\"mcp\"]",
                "-c",
                "mcp_servers.sextant.required=true",
                "-c",
                "mcp_servers.sextant.default_tools_approval_mode=\"approve\"",
                "-c",
            ])
            .arg(format!("mcp_servers.sextant.env_vars={SERVER_ENV}"))
            .args(["--color", "never", "-"]),
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

    let mut stdin = child.stdin.take().context("agent stdin")?;
    let prompt = prompt.to_owned();
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
    if agent == AgentName::Claude {
        claude_succeeded(&out)?;
    }
    Ok(out)
}

fn signed_out(stderr: &str) -> bool {
    let text = stderr.to_lowercase();
    ["sign in again", "not logged in", "/login", "refresh token"]
        .iter()
        .any(|hint| text.contains(hint))
}

/// `claude -p --output-format json` exits 0 even when the run failed, and says so in `is_error`.
fn claude_succeeded(out: &str) -> Result<()> {
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
    Ok(())
}

fn routine_count(engine: &Engine) -> Result<usize> {
    let routines = engine.read(|store| store.routines(0))?;
    Ok(routines["routines"].as_array().map_or(0, Vec::len))
}

/// One run: the agent reads and saves through the tools, then the run is recorded. Logs only
/// counts and timings.
fn analyze(engine: &Engine, settings: &Settings, agent: AgentName) -> Result<()> {
    if !settings.mcp_workflows {
        bail!(
            "turn on \"Agents can read my activities, tasks and routines\" in Privacy first: your agent reads your weeks through it"
        );
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        bail!("an analysis is already running");
    }
    let result = (|| {
        let started = Instant::now();
        let before = routine_count(engine)?;
        let server = std::env::current_exe().context("finding the Sextant binary")?;
        ask(agent, &server, FIND_ROUTINES)?;
        eprintln!(
            "agent run: {} routines saved, {} s",
            routine_count(engine)? - before,
            started.elapsed().as_secs()
        );
        let now = clock::now();
        engine.write(move |store| store.save_run(now, agent))
    })();
    RUNNING.store(false, Ordering::SeqCst);
    result
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
    /// Saved routines, as the `routines` tool lists them.
    routines: Value,
}

fn status(engine: &Engine, settings: &Settings) -> Result<AgentStatus> {
    let run: Option<Run> = engine.read(|store| store.agent_run())?;
    let routines = engine.read(|store| store.routines(clock::local().utc_offset_s))?;
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
        routines: routines["routines"].clone(),
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
        analyze(engine, &settings, agent).map_err(|err| format!("{err:#}"))?;
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
                if due && analyze(engine, &settings, agent).is_err() {
                    eprintln!("weekly agent run failed");
                }
            }
        })
        .context("starting the agent thread")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use sextant_core::fixtures::{
        Fixture, TEST_SECRET, UTC_OFFSET_S, WEEK_NOW, WEEK_START, actions_week, load, local, week,
    };
    use sextant_core::mcp::write_skill;
    use sextant_core::{Routine, Store};

    use super::*;

    const KEY_HEX: &str = "0707070707070707070707070707070707070707070707070707070707070707";

    /// A temporary Sextant home holding `fixture`, set up for `sextant mcp`. Returns the folder,
    /// the database and the server binary.
    fn home(fixture: &Fixture) -> (PathBuf, PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("sextant-bridge-{}", clock::now()));
        std::fs::create_dir_all(&dir).expect("test setup");
        let db = dir.join("sextant.db");
        let mut store = Store::open(&db, &[7; 32], &TEST_SECRET).expect("test setup");
        load(&mut store, fixture, WEEK_NOW).expect("test setup");
        store.refresh(local(WEEK_NOW)).expect("test setup");
        drop(store);
        // SAFETY: the ignored live tests are run one at a time and nothing else reads these.
        unsafe {
            std::env::set_var("SEXTANT_HOME", &dir);
            std::env::set_var("SEXTANT_DEV_KEY", KEY_HEX);
            std::env::set_var("SEXTANT_NOW", WEEK_NOW.to_string());
        }
        // The test binary sits in target/debug/deps, the app binary one folder up.
        let server = std::env::current_exe()
            .ok()
            .and_then(|exe| Some(exe.parent()?.parent()?.join("sextant")))
            .filter(|path| path.is_file())
            .expect("build the app first: cargo build -p sextant-app");
        (dir, db, server)
    }

    /// Runs the real CLI on the fixture week through `sextant mcp`, so it costs a little of your
    /// plan. Build the binary first, then run
    /// `cargo build -p sextant-app && cargo test -p sextant-app live_ -- --ignored --nocapture`.
    fn live(agent: AgentName) {
        let (dir, db, server) = home(&week());

        let started = Instant::now();
        ask(agent, &server, FIND_ROUTINES).expect("agent run");
        let store = Store::open_read_only(&db, &[7; 32]).expect("test setup");
        let routines = store.routines(0).expect("routines");
        let routines = routines["routines"].as_array().expect("list");
        eprintln!(
            "{agent}: {} routines in {:?}",
            routines.len(),
            started.elapsed()
        );
        for routine in routines {
            eprintln!(
                "  {:.1} h  {}  |  {}",
                routine["hours_week"].as_f64().unwrap_or(0.0),
                routine["title"],
                routine["how"]
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            routines.iter().any(|routine| {
                let text = format!("{} {}", routine["title"], routine["how"]).to_lowercase();
                text.contains("invoice") || text.contains("rechnung")
            }),
            "the invoice routine was not saved"
        );
    }

    #[test]
    fn a_revoked_login_reads_as_signed_out() {
        assert!(signed_out(
            "ERROR codex_login::auth::manager: Failed to refresh token: Please log out and sign in again"
        ));
        assert!(!signed_out("error: unexpected argument '--foo'"));
    }

    #[test]
    fn a_failed_claude_run_is_an_error_even_with_exit_code_zero() {
        assert!(claude_succeeded(r#"{"is_error": false, "result": "saved 2"}"#).is_ok());
        assert!(claude_succeeded(r#"{"is_error": true, "result": "out of turns"}"#).is_err());
    }

    #[test]
    #[ignore = "calls the claude CLI"]
    fn live_claude_saves_the_invoice_routine() {
        live(AgentName::Claude);
    }

    #[test]
    #[ignore = "calls the codex CLI"]
    fn live_codex_saves_the_invoice_routine() {
        live(AgentName::Codex);
    }

    /// Claude writes a skill for the saved invoice routine, and every citation resolves. It
    /// can't save files here, so it replies with the SKILL.md.
    #[test]
    #[ignore = "calls the claude CLI"]
    fn live_claude_writes_a_skill_whose_citations_resolve() {
        let (dir, db, server) = home(&actions_week().0);
        let settings = Settings::default();
        let store = Store::open_read_only(&db, &[7; 32]).expect("test setup");
        let week = store
            .week_page(&settings, WEEK_START, UTC_OFFSET_S)
            .expect("week");
        let invoices = week["repeated_tasks"]
            .as_array()
            .and_then(|kinds| kinds.iter().find(|kind| kind["label"] == "inv #"))
            .map(|kind| kind["task_ids"].clone())
            .expect("the invoice tasks");
        let routine: Routine = serde_json::from_value(serde_json::json!({
            "title": "Send invoices",
            "task_ids": invoices,
            "how": "Fill the invoice from the sheet and email it.",
            "approval": "sending the email",
            "why": "Every weekday.",
        }))
        .expect("test setup");
        let saved = store
            .save_routine(&settings, &routine, WEEK_NOW, UTC_OFFSET_S)
            .expect("test setup");
        let id = saved["id"].as_i64().expect("id");

        let prompt = format!(
            "{}\n\nYou can't save files here. Instead end your reply with the whole SKILL.md.",
            write_skill(id)
        );
        let started = Instant::now();
        let out = ask(AgentName::Claude, &server, &prompt).expect("agent run");
        let reply: Value = serde_json::from_str(&out).expect("json");
        let skill = reply["result"].as_str().expect("reply");
        eprintln!("claude: skill in {:?}\n{skill}", started.elapsed());
        let checked = store.check_skill(&settings, id, skill);
        let _ = std::fs::remove_dir_all(&dir);
        let checked = checked.expect("every citation resolves");
        assert!(checked["steps"].as_u64() >= Some(3), "{checked}");
    }
}
