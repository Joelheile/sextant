//! `sextant tui`: status, routines, the agent bridge, Privacy and the sent log on one screen.
//! Plain terminal escapes on a raw-mode terminal, refreshed every 2 seconds. Destructive actions
//! stay in the CLI, where they ask for a typed confirmation.

use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use sextant_core::agent::AgentName;
use sextant_core::{Live, Sent, Settings};

use crate::agent::AgentStatus;
use crate::cli::{
    agent_lines, privacy_lines, routine_lines, routines, routines_total, sent_line, status_lines,
};
use crate::control::{Request, Status, call};
use crate::engine::AudioState;

const REFRESH_EVERY: Duration = Duration::from_secs(2);
const TABS: [&str; 5] = ["Status", "Routines", "Agent", "Privacy", "Sent"];
const RETENTION: [Option<u32>; 4] = [Some(30), Some(90), Some(365), None];
const STORAGE_CAP_MB: [Option<u32>; 5] = [Some(1000), Some(2000), Some(5000), Some(10000), None];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    Enter,
}

/// A row of the Privacy tab that a key press changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Switch {
    Metadata,
    Ocr,
    Copy,
    Actions,
    Browsers,
    ScreenText,
    Activities,
    Retention,
    StorageCap,
}

const SWITCHES: [Switch; 9] = [
    Switch::Metadata,
    Switch::Ocr,
    Switch::Copy,
    Switch::Actions,
    Switch::Browsers,
    Switch::ScreenText,
    Switch::Activities,
    Switch::Retention,
    Switch::StorageCap,
];

impl Switch {
    fn label(self, settings: &Settings) -> String {
        let on = |on: bool| if on { "on" } else { "off" };
        let live = settings.live;
        match self {
            Self::Metadata => format!("Apps and windows: {}", on(live.metadata)),
            Self::Ocr => format!("Screen text: {}", on(live.ocr)),
            Self::Copy => format!("Which app copied: {}", on(live.copy)),
            Self::Actions => format!("Presses and field values: {}", on(live.actions)),
            Self::Browsers => format!(
                "Browsers without private-window info: {}",
                on(live.opaque_browsers)
            ),
            Self::ScreenText => format!(
                "Agents can read screen text and search: {}",
                on(settings.mcp_rewind)
            ),
            Self::Activities => format!(
                "Agents can read activities and routines: {}",
                on(settings.mcp_workflows)
            ),
            Self::Retention => format!(
                "Keep data: {}",
                settings
                    .retention_days
                    .map_or("forever".into(), |days| format!("{days} days"))
            ),
            Self::StorageCap => format!(
                "Storage cap: {}",
                settings
                    .storage_cap_mb
                    .map_or("none".into(), |mb| format!("{} GB", mb / 1000))
            ),
        }
    }

    /// The request that flips this switch or moves it to the next choice.
    fn next(self, settings: &Settings) -> Request {
        let flip = |change: fn(&mut Live)| {
            let mut live = settings.live;
            change(&mut live);
            Request::Capture { live }
        };
        match self {
            Self::Metadata => flip(|live| live.metadata = !live.metadata),
            Self::Ocr => flip(|live| live.ocr = !live.ocr),
            Self::Copy => flip(|live| live.copy = !live.copy),
            Self::Actions => flip(|live| live.actions = !live.actions),
            Self::Browsers => flip(|live| live.opaque_browsers = !live.opaque_browsers),
            Self::ScreenText => Request::Mcp {
                rewind: !settings.mcp_rewind,
                workflows: settings.mcp_workflows,
            },
            Self::Activities => Request::Mcp {
                rewind: settings.mcp_rewind,
                workflows: !settings.mcp_workflows,
            },
            Self::Retention => Request::Retention {
                days: following(&RETENTION, settings.retention_days),
            },
            Self::StorageCap => Request::StorageCap {
                mb: following(&STORAGE_CAP_MB, settings.storage_cap_mb),
            },
        }
    }
}

/// The choice after `current`, wrapping around. A value not in the list goes to the first.
fn following(choices: &[Option<u32>], current: Option<u32>) -> Option<u32> {
    let next = choices
        .iter()
        .position(|&choice| choice == current)
        .map_or(0, |index| (index + 1) % choices.len());
    choices[next]
}

/// Off, then Claude Code, then Codex.
fn next_agent(current: Option<AgentName>) -> Option<AgentName> {
    match current {
        None => Some(AgentName::Claude),
        Some(AgentName::Claude) => Some(AgentName::Codex),
        Some(AgentName::Codex) => None,
    }
}

struct Screen {
    tab: usize,
    /// The selected row in Routines and Privacy.
    row: usize,
    status: Result<Status, String>,
    agent: Option<AgentStatus>,
    settings: Option<Settings>,
    sent: Vec<Sent>,
    note: String,
    run: Option<Receiver<Result<AgentStatus>>>,
}

impl Screen {
    fn new() -> Self {
        Self {
            tab: 0,
            row: 0,
            status: Err("Connecting to Sextant".into()),
            agent: None,
            settings: None,
            sent: Vec::new(),
            note: String::new(),
            run: None,
        }
    }

    fn load(&mut self, dir: &Path) {
        self.status = call(dir, &Request::Status).map_err(|err| format!("{err:#}"));
        if self.status.is_err() {
            return;
        }
        self.agent = call(dir, &Request::Agent).ok();
        self.settings = call(dir, &Request::Settings).ok();
        self.sent = call(dir, &Request::Sent).unwrap_or_default();
    }

    fn rows(&self) -> usize {
        match TABS[self.tab] {
            "Routines" => self.agent.as_ref().map_or(0, |agent| routines(agent).len()),
            "Privacy" => SWITCHES.len(),
            _ => 0,
        }
    }

    /// Applies `key`. Returns `false` to quit.
    fn press(&mut self, dir: &Path, key: Key) -> bool {
        match key {
            Key::Char('q') => return false,
            Key::Char('\t') | Key::Right => self.switch_tab(self.tab + 1),
            Key::Left => self.switch_tab(self.tab + TABS.len() - 1),
            Key::Char(digit @ '1'..='5') => self.switch_tab(digit as usize - '1' as usize),
            Key::Up => self.row = self.row.saturating_sub(1),
            Key::Down => self.row = (self.row + 1).min(self.rows().saturating_sub(1)),
            key => {
                if let Some(request) = self.action(key) {
                    self.send(dir, &request);
                }
            }
        }
        true
    }

    fn switch_tab(&mut self, tab: usize) {
        self.tab = tab % TABS.len();
        self.row = 0;
    }

    /// The request a key sends on the current tab.
    fn action(&mut self, key: Key) -> Option<Request> {
        let status = self.status.as_ref().ok()?;
        match (TABS[self.tab], key) {
            ("Status", Key::Char('p')) if status.paused_until.is_some() => Some(Request::Resume),
            ("Status", Key::Char('p')) => Some(Request::Pause),
            ("Status", Key::Char('a')) => Some(Request::Audio {
                on: status.audio == AudioState::Off,
            }),
            ("Agent", Key::Char('c')) => {
                let agent = self.agent.as_ref()?;
                let chosen = next_agent(agent.chosen);
                Some(Request::ChooseAgent {
                    agent: chosen,
                    weekly: agent.weekly && chosen.is_some(),
                })
            }
            ("Agent", Key::Char('w')) => {
                let agent = self.agent.as_ref()?;
                Some(Request::ChooseAgent {
                    agent: agent.chosen,
                    weekly: !agent.weekly,
                })
            }
            ("Agent", Key::Char('r')) => {
                self.note = "Your agent is reading your weeks. This can take a few minutes.".into();
                self.start_run();
                None
            }
            ("Privacy", Key::Enter | Key::Char(' ')) => {
                let settings = self.settings.as_ref()?;
                Some(SWITCHES.get(self.row)?.next(settings))
            }
            _ => None,
        }
    }

    fn send(&mut self, dir: &Path, request: &Request) {
        let reply: Result<serde_json::Value> = call(dir, request);
        self.note = match reply {
            Ok(_) => "Saved.".into(),
            Err(err) => format!("{err:#}"),
        };
        self.load(dir);
    }

    fn start_run(&mut self) {
        if self.run.is_some() {
            return;
        }
        let (done, wait) = mpsc::channel();
        let dir = crate::home::data_dir();
        thread::spawn(move || {
            let result = dir.and_then(|dir| call(&dir, &Request::RunAgent));
            let _ = done.send(result);
        });
        self.run = Some(wait);
    }

    fn check_run(&mut self) {
        let Some(wait) = &self.run else {
            return;
        };
        let Ok(result) = wait.try_recv() else {
            return;
        };
        self.note = match result {
            Ok(agent) => format!("Done. {}.", routines_total(&agent)),
            Err(err) => format!("{err:#}"),
        };
        self.run = None;
    }

    fn lines(&self) -> Vec<String> {
        let tabs: Vec<String> = TABS
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let label = format!("{} {name}", index + 1);
                if index == self.tab {
                    format!("\x1b[7m {label} \x1b[0m")
                } else {
                    format!(" {label} ")
                }
            })
            .collect();
        let mut lines = vec![format!("Sextant  {}", tabs.join(" ")), String::new()];
        match &self.status {
            Err(err) => lines.push(err.clone()),
            Ok(status) => lines.extend(self.body(status)),
        }
        lines.push(String::new());
        lines.push(self.note.clone());
        lines.push(format!("\x1b[2m{}\x1b[0m", self.keys()));
        lines
    }

    fn body(&self, status: &Status) -> Vec<String> {
        match TABS[self.tab] {
            "Status" => status_lines(status),
            "Routines" => match &self.agent {
                Some(agent) if !routines(agent).is_empty() => {
                    let mut lines = vec![routines_total(agent), String::new()];
                    for (index, routine) in routines(agent).iter().enumerate() {
                        let mut block = routine_lines(routine, agent.chosen);
                        if index == self.row {
                            lines.extend(block);
                        } else {
                            block.truncate(1);
                            lines.extend(block);
                        }
                    }
                    lines
                }
                _ => vec![
                    "No routines yet. Press 3, then r, to have your agent look for them.".into(),
                ],
            },
            "Agent" => self.agent.as_ref().map(agent_lines).unwrap_or_default(),
            "Privacy" => match &self.settings {
                Some(settings) => {
                    let mut lines: Vec<String> = SWITCHES
                        .iter()
                        .enumerate()
                        .map(|(index, switch)| {
                            let label = switch.label(settings);
                            if index == self.row {
                                format!("\x1b[7m> {label}\x1b[0m")
                            } else {
                                format!("  {label}")
                            }
                        })
                        .collect();
                    lines.push(String::new());
                    lines.extend(privacy_lines(settings, None).into_iter().skip(3));
                    lines.push(
                        "Excluded apps and sites, forget and delete: `sextant privacy`.".into(),
                    );
                    lines
                }
                None => Vec::new(),
            },
            _ if self.sent.is_empty() => vec!["No agent has received anything yet.".into()],
            _ => self.sent.iter().map(sent_line).collect(),
        }
    }

    fn keys(&self) -> &'static str {
        match TABS[self.tab] {
            "Status" => "p pause or resume  a audio on or off  tab next  q quit",
            "Routines" => "up and down to open a routine  tab next  q quit",
            "Agent" => "c change agent  w weekly  r run now  tab next  q quit",
            "Privacy" => "up and down to pick  space to change  tab next  q quit",
            _ => "tab next  q quit",
        }
    }
}

/// Puts the terminal in raw mode on the alternate screen, and back when dropped.
struct Terminal {
    saved: libc::termios,
}

impl Terminal {
    fn open() -> Result<Self> {
        // SAFETY: `termios` is plain data that `tcgetattr` fills for stdin.
        let saved = unsafe {
            let mut saved: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut saved) != 0 {
                return Err(io::Error::last_os_error()).context("`sextant tui` needs a terminal");
            }
            saved
        };
        let mut raw = saved;
        raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
        raw.c_iflag &= !(libc::IXON | libc::ICRNL);
        // A read returns after 100 ms without input, so the screen keeps refreshing.
        raw.c_cc[libc::VMIN] = 0;
        raw.c_cc[libc::VTIME] = 1;
        // SAFETY: `raw` is a valid `termios` copied from the current one.
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error()).context("switching the terminal to raw mode");
        }
        print!("\x1b[?1049h\x1b[?25l");
        io::stdout().flush()?;
        Ok(Self { saved })
    }

    fn width() -> usize {
        // SAFETY: `winsize` is plain data that `ioctl` fills for stdout.
        unsafe {
            let mut size: libc::winsize = std::mem::zeroed();
            if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) == 0 && size.ws_col > 0
            {
                usize::from(size.ws_col)
            } else {
                80
            }
        }
    }

    fn draw(lines: &[String]) -> io::Result<()> {
        let width = Self::width();
        let mut out = String::from("\x1b[H\x1b[2J");
        for line in lines {
            out.push_str(&clip(line, width));
            out.push_str("\r\n");
        }
        let mut stdout = io::stdout().lock();
        stdout.write_all(out.as_bytes())?;
        stdout.flush()
    }

    fn key() -> io::Result<Option<Key>> {
        let mut bytes = [0u8; 8];
        let read = io::stdin().lock().read(&mut bytes)?;
        Ok(parse_key(&bytes[..read]))
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        print!("\x1b[?25h\x1b[?1049l");
        let _ = io::stdout().flush();
        // SAFETY: restores the `termios` read in `open`.
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.saved);
        }
    }
}

fn parse_key(bytes: &[u8]) -> Option<Key> {
    match bytes {
        [0x1b, b'[', b'A', ..] => Some(Key::Up),
        [0x1b, b'[', b'B', ..] => Some(Key::Down),
        [0x1b, b'[', b'C', ..] => Some(Key::Right),
        [0x1b, b'[', b'D', ..] => Some(Key::Left),
        [b'\r' | b'\n', ..] => Some(Key::Enter),
        // Ctrl-C quits too.
        [3, ..] | [0x1b] => Some(Key::Char('q')),
        [byte, ..] if byte.is_ascii() => Some(Key::Char(char::from(*byte))),
        _ => None,
    }
}

/// Cuts `line` to `width` visible characters. Escape sequences take no width.
fn clip(line: &str, width: usize) -> String {
    let mut out = String::new();
    let mut visible = 0;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            out.push(c);
            for c in chars.by_ref() {
                out.push(c);
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        if visible < width {
            out.push(c);
            visible += 1;
        }
    }
    out
}

pub fn run(dir: &Path) -> Result<()> {
    let _terminal = Terminal::open()?;
    let mut screen = Screen::new();
    screen.load(dir);
    let mut loaded = Instant::now();
    loop {
        Terminal::draw(&screen.lines())?;
        if let Some(key) = Terminal::key()?
            && !screen.press(dir, key)
        {
            return Ok(());
        }
        screen.check_run();
        if loaded.elapsed() >= REFRESH_EVERY {
            screen.load(dir);
            loaded = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_and_the_cap_cycle_through_their_choices() {
        assert_eq!(following(&RETENTION, Some(90)), Some(365));
        assert_eq!(following(&RETENTION, None), Some(30));
        assert_eq!(following(&STORAGE_CAP_MB, Some(5000)), Some(10000));
        assert_eq!(following(&STORAGE_CAP_MB, Some(123)), Some(1000));
    }

    #[test]
    fn a_privacy_switch_changes_only_its_own_setting() {
        let settings = Settings::default();
        let Request::Capture { live } = Switch::Ocr.next(&settings) else {
            panic!("a capture switch");
        };
        assert_eq!(
            live,
            Live {
                ocr: !settings.live.ocr,
                ..settings.live
            }
        );
        let Request::Mcp { rewind, workflows } = Switch::ScreenText.next(&settings) else {
            panic!("an agent switch");
        };
        assert_eq!(
            (rewind, workflows),
            (!settings.mcp_rewind, settings.mcp_workflows)
        );
    }

    #[test]
    fn the_agent_cycles_off_claude_codex() {
        assert_eq!(next_agent(None), Some(AgentName::Claude));
        assert_eq!(next_agent(Some(AgentName::Claude)), Some(AgentName::Codex));
        assert_eq!(next_agent(Some(AgentName::Codex)), None);
    }

    #[test]
    fn keys_and_clipping() {
        assert_eq!(parse_key(b"\x1b[A"), Some(Key::Up));
        assert_eq!(parse_key(b"\t"), Some(Key::Char('\t')));
        assert_eq!(parse_key(b"\x03"), Some(Key::Char('q')));
        assert_eq!(parse_key(b""), None);
        assert_eq!(clip("\x1b[7mabcdef\x1b[0m", 3), "\x1b[7mabc\x1b[0m");
    }

    #[test]
    fn without_the_app_the_screen_says_so_and_ignores_actions() {
        let mut screen = Screen::new();
        screen.status = Err("Sextant is not running: open Sextant.app first".into());
        assert!(screen.action(Key::Char('p')).is_none());
        assert!(
            screen
                .lines()
                .contains(&"Sextant is not running: open Sextant.app first".to_owned())
        );
    }
}
