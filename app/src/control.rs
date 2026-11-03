//! The control socket: the running app listens on `sextant.sock` in the data folder, and the
//! `sextant` CLI and TUI send it one JSON request per connection. The app keeps the only write
//! connection and the settings capture runs with, so every change goes through it. The folder is
//! private to the user, and the socket is too.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sextant_core::Live;
use sextant_core::agent::AgentName;
use tauri::{AppHandle, Manager};

use crate::commands::{AppState, Reply, msg};
use crate::engine::{AudioState, CaptureState};
use crate::platform::{Prompt, SettingsPane};
use crate::{agent, commands, data, debug, engine, platform, prefs, screens, tray};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    Status,
    Pause,
    Resume,
    Audio {
        on: bool,
    },
    Models,
    DownloadModels,
    Settings,
    Permissions,
    /// Opens a System Settings pane, with the panel to drag Sextant's icon from.
    Allow {
        pane: SettingsPane,
    },
    AskAutomation,
    FinishSetup,
    Relaunch,
    Agent,
    ChooseAgent {
        agent: Option<AgentName>,
        weekly: bool,
    },
    RunAgent,
    Sent,
    Capture {
        live: Live,
    },
    Mcp {
        rewind: bool,
        workflows: bool,
    },
    ExcludedApps {
        apps: Vec<String>,
    },
    ExcludedSites {
        sites: Vec<String>,
    },
    Retention {
        days: Option<u32>,
    },
    StorageCap {
        mb: Option<u32>,
    },
    Forget {
        minutes: i64,
    },
    Debug,
    DeleteEverything,
    Uninstall,
}

/// What `sextant status` shows.
#[derive(Debug, Serialize, Deserialize)]
pub struct Status {
    /// The tray's status line.
    pub line: String,
    pub paused_until: Option<i64>,
    pub audio: AudioState,
    /// Why the engine did not start, if it did not.
    pub problem: Option<String>,
    pub last_capture: Option<(i64, String)>,
    pub storage_bytes: u64,
    pub set_up: bool,
}

/// What the app does after it has answered.
#[derive(Debug, PartialEq, Eq)]
pub enum After {
    Nothing,
    Restart,
    Quit,
}

const DRAG_HINT: &str = "Drag Sextant into the list";

pub fn socket_path(dir: &Path) -> PathBuf {
    dir.join("sextant.sock")
}

/// Starts listening. A second copy of the app leaves the first one's socket alone.
pub fn serve(app: AppHandle) -> Result<()> {
    let dir = app.state::<AppState>().dir.clone();
    let listener = bind(&dir)?;
    thread::Builder::new()
        .name("sextant-control".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let app = app.clone();
                let spawned = thread::Builder::new().name("sextant-request".into()).spawn(
                    move || match answer(stream, &app.state::<AppState>()) {
                        After::Nothing => {}
                        After::Restart => app.restart(),
                        After::Quit => app.exit(0),
                    },
                );
                if spawned.is_err() {
                    eprintln!("could not answer a control request");
                }
            }
        })
        .context("starting the control thread")?;
    Ok(())
}

/// Whether a Sextant app answers on the socket in `dir`.
pub fn running(dir: &Path) -> bool {
    UnixStream::connect(socket_path(dir)).is_ok()
}

fn bind(dir: &Path) -> Result<UnixListener> {
    engine::create_private_dir(dir)?;
    let path = socket_path(dir);
    if running(dir) {
        bail!("another Sextant is already running");
    }
    // Left behind by a crash.
    let _ = fs::remove_file(&path);
    let listener = UnixListener::bind(&path).context("opening the control socket")?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .context("making the control socket private")?;
    Ok(listener)
}

/// Reads one request, answers it and says what to do next.
fn answer(stream: UnixStream, state: &AppState) -> After {
    let mut line = String::new();
    let mut reader = BufReader::new(&stream);
    let (reply, after) = match reader.read_line(&mut line) {
        Ok(_) => match serde_json::from_str::<Request>(&line) {
            Ok(request) => {
                let after = after(&request);
                (handle(state, request), after)
            }
            Err(err) => (Err(format!("unknown request: {err}")), After::Nothing),
        },
        Err(err) => (Err(msg(err)), After::Nothing),
    };
    let after = if reply.is_ok() { after } else { After::Nothing };
    let mut writer = &stream;
    let sent = serde_json::to_writer(&mut writer, &reply)
        .map_err(msg)
        .and_then(|()| writer.write_all(b"\n").map_err(msg));
    if sent.is_err() {
        eprintln!("could not send a control reply");
    }
    after
}

fn after(request: &Request) -> After {
    match request {
        Request::DeleteEverything | Request::Relaunch => After::Restart,
        Request::Uninstall => After::Quit,
        _ => After::Nothing,
    }
}

fn json(value: impl Serialize) -> Reply<Value> {
    serde_json::to_value(value).map_err(msg)
}

pub fn handle(state: &AppState, request: Request) -> Reply<Value> {
    match request {
        Request::Status => json(status(state)),
        Request::Pause => {
            state.engine()?.pause_capture();
            json(status(state))
        }
        Request::Resume => {
            state.engine()?.resume_capture();
            json(status(state))
        }
        Request::Audio { on } => json(commands::switch_audio(state, on)?),
        Request::Models => json(commands::models(state)?),
        Request::DownloadModels => {
            state.engine()?.download_models().map_err(msg)?;
            json(commands::models(state)?)
        }
        Request::Settings => json(prefs::load(&state.dir).map_err(msg)?),
        Request::Permissions => json(commands::permissions(&state.dir)?),
        Request::Allow { pane } => {
            platform::open_settings_pane(pane, Some(DRAG_HINT)).map_err(msg)?;
            json(())
        }
        Request::AskAutomation => {
            json(platform::request_permission(Prompt::Automation).map_err(msg)?)
        }
        Request::FinishSetup => json(commands::finish_setup(state)?),
        Request::Relaunch => json(()),
        Request::Agent => json(agent::bridge(state)?),
        Request::ChooseAgent { agent, weekly } => json(agent::choose(state, agent, weekly)?),
        Request::RunAgent => json(agent::run_now(state)?),
        Request::Sent => json(screens::sent(state)?),
        Request::Capture { live } => json(screens::capture_sources(state, live)?),
        Request::Mcp { rewind, workflows } => {
            json(commands::switch_mcp(&state.dir, rewind, workflows)?)
        }
        Request::ExcludedApps { apps } => json(screens::exclude_apps(state, apps)?),
        Request::ExcludedSites { sites } => json(screens::exclude_sites(state, sites)?),
        Request::Retention { days } => json(screens::keep_for(state, days)?),
        Request::StorageCap { mb } => json(screens::cap_storage(state, mb)?),
        Request::Forget { minutes } => json(screens::forget(state, minutes)?),
        Request::Debug => json(debug::report(state)?),
        Request::DeleteEverything => {
            state
                .delete_everything(&crate::keys::Keychain)
                .map_err(|err| format!("{err:#}"))?;
            json(())
        }
        Request::Uninstall => json(screens::remove_sextant(state)?),
    }
}

fn status(state: &AppState) -> Status {
    let look = tray::look(state);
    let engine = state.engine();
    Status {
        line: look.text,
        paused_until: match look.capture {
            CaptureState::Paused(until) => Some(until),
            _ => None,
        },
        audio: look.audio,
        problem: engine.as_ref().err().cloned(),
        last_capture: engine
            .ok()
            .and_then(|engine| engine.read(|store| store.last_capture()).ok().flatten()),
        storage_bytes: data::storage_used(&state.dir),
        set_up: prefs::load(&state.dir).is_ok_and(|settings| settings.onboarded),
    }
}

/// Sends `request` to the running app and waits for its answer.
pub fn call<T: DeserializeOwned>(dir: &Path, request: &Request) -> Result<T> {
    let stream = UnixStream::connect(socket_path(dir))
        .context("Sextant is not running: open Sextant.app first")?;
    let mut writer = &stream;
    serde_json::to_writer(&mut writer, request).context("sending the request")?;
    writer.write_all(b"\n").context("sending the request")?;
    let mut line = String::new();
    BufReader::new(&stream)
        .read_line(&mut line)
        .context("reading Sextant's answer")?;
    let reply: Reply<T> = serde_json::from_str(&line).context("reading Sextant's answer")?;
    reply.map_err(anyhow::Error::msg)
}

#[cfg(test)]
mod tests {
    use sextant_core::Settings;
    use sextant_core::fixtures::{TEST_KEY, TEST_SECRET, WEEK_NOW, load, local, week};
    use sextant_core::{Sent, Store};

    use super::*;
    use crate::engine::Engine;
    use crate::home;

    fn temp(label: &str) -> PathBuf {
        // Short: a socket path must fit in 104 bytes.
        let dir = PathBuf::from(format!("/tmp/sx-{}-{label}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("test setup");
        dir
    }

    /// An app state with the fixture week, and a thread answering on its socket.
    fn serving(label: &str) -> PathBuf {
        let dir = temp(label);
        let mut store =
            Store::open(&home::db_path(&dir), &TEST_KEY, &TEST_SECRET).expect("test setup");
        load(&mut store, &week(), WEEK_NOW).expect("test setup");
        store.refresh(local(WEEK_NOW)).expect("test setup");
        drop(store);
        let state = AppState::new(dir.clone());
        state.start(|dir| Engine::start(dir, &TEST_KEY));
        let listener = bind(&dir).expect("binds");
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                assert_eq!(answer(stream, &state), After::Nothing);
            }
        });
        dir
    }

    #[test]
    fn the_cli_reads_and_changes_settings_through_the_socket() {
        let dir = serving("settings");
        let status: Status = call(&dir, &Request::Status).expect("answers");
        assert_eq!(status.line, "Capture is off");
        assert!(status.problem.is_none());
        assert!(status.last_capture.is_some(), "the fixture week has frames");

        let settings: Settings =
            call(&dir, &Request::Retention { days: Some(30) }).expect("answers");
        assert_eq!(settings.retention_days, Some(30));
        assert_eq!(prefs::load(&dir).expect("saved").retention_days, Some(30));

        let refused = call::<Settings>(&dir, &Request::Retention { days: Some(7) });
        assert_eq!(
            refused.expect_err("only the listed choices").to_string(),
            "choose 30, 90, 365 days or forever"
        );
        let sent: Vec<Sent> = call(&dir, &Request::Sent).expect("answers");
        assert!(sent.is_empty());
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn a_second_app_does_not_take_over_the_socket() {
        let dir = serving("second");
        assert!(bind(&dir).is_err());
        let _: Status = call(&dir, &Request::Status).expect("the first still answers");
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn a_stale_socket_is_replaced_and_kept_private() {
        let dir = temp("stale");
        drop(UnixListener::bind(socket_path(&dir)).expect("test setup"));
        bind(&dir).expect("replaces the stale socket");
        let mode = fs::metadata(socket_path(&dir))
            .expect("exists")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn without_the_app_the_cli_says_so() {
        let dir = temp("absent");
        let err = call::<Status>(&dir, &Request::Status).expect_err("nothing listens");
        assert_eq!(
            err.to_string(),
            "Sextant is not running: open Sextant.app first"
        );
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn only_a_successful_wipe_restarts_the_app() {
        assert_eq!(after(&Request::DeleteEverything), After::Restart);
        assert_eq!(after(&Request::Uninstall), After::Quit);
        assert_eq!(after(&Request::Status), After::Nothing);
    }
}
