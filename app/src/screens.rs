//! The typed commands behind Workflows, rewind search, Results, Privacy and the weekly report
//! (plan.md §9). Reads go through the read-only connection, changes through the worker thread.
//! Text from the database is redacted again on the way out, as in the MCP server.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sextant_core::{
    Action, Category, Enjoyment, Kind, Live, MIN_DAYS, Measurement, Sent, Session, Settings, Store,
    Target, Turn, Workflow, WorkflowDetail, keywords, parse_correction, redact, skill,
};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use crate::commands::{AppState, MAX_ROWS, Reply, blocking, msg};
use crate::keys::Keychain;
use crate::platform::language::Note;
use crate::{clock, connect, data, platform};

const SKILLS_FOLDER: &str = "skills";
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
const MAX_IMAGE_BYTES: usize = 8 << 20;
const RETENTION_CHOICES: [Option<u32>; 4] = [Some(30), Some(90), Some(365), None];
const STORAGE_CAP_CHOICES: [Option<u32>; 5] =
    [Some(1000), Some(2000), Some(5000), Some(10000), None];
/// "Delete the last 15 minutes, hour or day".
const FORGET_CHOICES: [i64; 3] = [15, 60, 1440];
const SENT_ROWS: usize = 50;

#[derive(Serialize)]
pub struct StepView {
    key: String,
    name: String,
    /// Typical wait before the next step, in seconds.
    gap_s: Option<i64>,
    /// How often data was copied by hand to the next step.
    copies: usize,
}

#[derive(Serialize)]
pub struct WorkflowRow {
    id: i64,
    name: String,
    steps: Vec<StepView>,
    hours_week: f64,
    cases: usize,
    benefit: f64,
    procrastination: f64,
    action: &'static str,
    kind: &'static str,
    enjoyment: &'static str,
    client: bool,
    rough: bool,
    handed_off: bool,
}

fn row(store: &Store, workflow: &Workflow) -> sextant_core::Result<WorkflowRow> {
    let links = store.step_links(workflow)?;
    let steps = workflow
        .steps
        .iter()
        .enumerate()
        .map(|(i, step)| {
            let link = workflow.steps.get(i + 1).and_then(|next| {
                links
                    .iter()
                    .find(|l| l.from == step.name && l.to == next.name)
            });
            StepView {
                key: step.key.clone(),
                name: step.name.clone(),
                gap_s: link.map(|l| l.median_gap_s),
                copies: link.map_or(0, |l| l.copies),
            }
        })
        .collect();
    Ok(WorkflowRow {
        id: workflow.id,
        name: workflow.name.clone(),
        steps,
        hours_week: workflow.hours_week,
        cases: workflow.task_count,
        benefit: workflow.benefit,
        procrastination: workflow.procrastination,
        action: workflow.action.as_str(),
        kind: workflow.kind.as_str(),
        enjoyment: workflow.enjoyment.as_str(),
        client: workflow.client,
        rough: workflow.rough,
        handed_off: store.is_handed_off(workflow.id)?,
    })
}

fn rows<'a>(
    store: &Store,
    workflows: impl Iterator<Item = &'a Workflow>,
) -> sextant_core::Result<Vec<WorkflowRow>> {
    workflows.map(|w| row(store, w)).collect()
}

#[derive(Serialize)]
pub struct Overview {
    hours_back: f64,
    /// Ranked workflows that are neither promises nor worth protecting.
    total: usize,
    /// The biggest workflows whose enjoyment is still unknown.
    ask: Vec<WorkflowRow>,
    ranked: Vec<WorkflowRow>,
    promises: Vec<WorkflowRow>,
    protect: Vec<WorkflowRow>,
}

/// The Workflows screen, one page of the ranked list at a time.
#[tauri::command]
pub async fn workflow_overview(app: AppHandle, offset: usize, limit: usize) -> Reply<Overview> {
    blocking(app, move |state| {
        state
            .engine()?
            .read(|store| {
                let all = store.workflows()?;
                let is_promise =
                    |w: &&Workflow| w.kind == Kind::PromiseGap && w.action != Action::Protect;
                let is_protect = |w: &&Workflow| w.action == Action::Protect;
                let ranked: Vec<&Workflow> = all
                    .iter()
                    .filter(|w| !is_promise(w) && !is_protect(w))
                    .collect();
                Ok(Overview {
                    hours_back: store.hours_back()?,
                    total: ranked.len(),
                    ask: rows(store, store.ask_first()?.iter())?,
                    ranked: rows(
                        store,
                        ranked.into_iter().skip(offset).take(limit.min(MAX_ROWS)),
                    )?,
                    promises: rows(store, all.iter().filter(is_promise).take(MAX_ROWS))?,
                    protect: rows(store, all.iter().filter(is_protect).take(MAX_ROWS))?,
                })
            })
            .map_err(msg)
    })
    .await
}

/// The answer to the quick enjoyment question. Stored as a correction, never inferred.
#[tauri::command]
pub async fn answer_enjoyment(app: AppHandle, id: i64, enjoyment: String) -> Reply<()> {
    blocking(app, move |state| {
        let enjoyment = Enjoyment::parse(&enjoyment).ok_or("unknown answer")?;
        state
            .engine()?
            .change(move |store| {
                store
                    .answer_enjoyment(clock::now(), Target::Workflow(id), enjoyment)
                    .map(|_| ())
            })
            .map_err(msg)
    })
    .await
}

#[derive(Serialize)]
pub struct CopyPointView {
    from: String,
    to: String,
    copies: usize,
}

#[derive(Serialize)]
pub struct ExampleView {
    entity: String,
    first_ts: i64,
    last_ts: i64,
    path: Vec<String>,
}

#[derive(Serialize)]
pub struct CommitmentView {
    ts: i64,
    source: String,
    text: String,
    due_ts: Option<i64>,
    overdue: bool,
}

#[derive(Serialize)]
pub struct Detail {
    workflow: WorkflowRow,
    reasons: Vec<String>,
    copy_points: Vec<CopyPointView>,
    examples: Vec<ExampleView>,
    commitments: Vec<CommitmentView>,
}

fn detail(store: &Store, found: WorkflowDetail) -> sextant_core::Result<Detail> {
    let secret = store.secret();
    let WorkflowDetail {
        workflow,
        copy_points,
        examples,
        commitments,
    } = found;
    Ok(Detail {
        workflow: row(store, &workflow)?,
        reasons: workflow
            .reasons
            .iter()
            .map(|r| redact(r, &secret))
            .collect(),
        copy_points: copy_points
            .into_iter()
            .map(|c| CopyPointView {
                from: c.from,
                to: c.to,
                copies: c.copies,
            })
            .collect(),
        examples: examples
            .into_iter()
            .map(|e| ExampleView {
                entity: redact(&e.entity, &secret),
                first_ts: e.first_ts,
                last_ts: e.last_ts,
                path: e.path,
            })
            .collect(),
        commitments: commitments
            .into_iter()
            .map(|c| CommitmentView {
                ts: c.ts,
                source: c.source,
                text: redact(&c.text, &secret),
                due_ts: c.due_ts,
                overdue: c.overdue,
            })
            .collect(),
    })
}

/// Reasons for every score, redacted example cases and linked promises of one workflow.
#[tauri::command]
pub async fn workflow_detail(app: AppHandle, id: i64) -> Reply<Option<Detail>> {
    blocking(app, move |state| {
        state
            .engine()?
            .read(|store| {
                store
                    .workflow_detail(id)?
                    .map(|found| detail(store, found))
                    .transpose()
            })
            .map_err(msg)
    })
    .await
}

#[derive(Serialize)]
pub struct Understood {
    rule: &'static str,
    arg: Option<String>,
}

/// Applies a correction written in plain English or German. `steps` holds the two step keys a
/// merge needs. `None` when no rule matches the phrase.
#[tauri::command]
pub async fn correct_workflow(
    app: AppHandle,
    id: i64,
    phrase: String,
    steps: Vec<String>,
) -> Reply<Option<Understood>> {
    blocking(app, move |state| {
        let Some(rule) = parse_correction(&phrase) else {
            return Ok(None);
        };
        let targets = match rule.name() {
            "merge" => steps.into_iter().map(Target::Step).collect(),
            _ => vec![Target::Workflow(id)],
        };
        state
            .engine()?
            .change(move |store| store.correct(clock::now(), &phrase, &targets))
            .map(|rule| {
                Some(Understood {
                    rule: rule.name(),
                    arg: rule.arg().map(str::to_owned),
                })
            })
            .map_err(msg)
    })
    .await
}

#[derive(Serialize)]
pub struct AutomationPlan {
    /// The `SKILL.md` that will be written.
    path: String,
    /// What to paste into Claude once it is written.
    prompt: String,
}

/// `name` as a folder name: lower case letters and digits joined by dashes.
fn slug(name: &str, id: i64) -> String {
    let words: Vec<String> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    if words.is_empty() {
        format!("workflow-{id}")
    } else {
        words.join("-")
    }
}

fn skill_path(data_dir: &Path, folder: Option<&str>, workflow: &Workflow) -> Reply<PathBuf> {
    let root = match folder {
        Some(folder) if Path::new(folder).is_absolute() => PathBuf::from(folder),
        Some(_) => return Err("pick a folder with a full path".into()),
        None => data_dir.join(SKILLS_FOLDER),
    };
    Ok(root
        .join(slug(&workflow.name, workflow.id))
        .join("SKILL.md"))
}

fn workflow_of(store: &Store, id: i64) -> sextant_core::Result<Workflow> {
    store
        .workflow(id)?
        .ok_or(sextant_core::Error::NoWorkflow(id))
}

/// Where the skill goes and the prompt for Claude. Writes nothing.
#[tauri::command]
pub async fn automation_plan(
    app: AppHandle,
    id: i64,
    folder: Option<String>,
) -> Reply<AutomationPlan> {
    blocking(app, move |state| {
        let workflow = state
            .engine()?
            .read(|store| workflow_of(store, id))
            .map_err(msg)?;
        let path = skill_path(&state.dir, folder.as_deref(), &workflow)?;
        let path = path.to_string_lossy().into_owned();
        Ok(AutomationPlan {
            prompt: skill::prompt(&workflow, &path),
            path,
        })
    })
    .await
}

/// Writes the `SKILL.md` and returns its path.
#[tauri::command]
pub async fn write_skill(app: AppHandle, id: i64, folder: Option<String>) -> Reply<String> {
    blocking(app, move |state| {
        let (workflow, text) = state
            .engine()?
            .read(|store| {
                let text = redact(
                    &store.skill_md(id, clock::local().utc_offset_s)?,
                    &store.secret(),
                );
                Ok((workflow_of(store, id)?, text))
            })
            .map_err(msg)?;
        let path = skill_path(&state.dir, folder.as_deref(), &workflow)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(msg)?;
        }
        fs::write(&path, text).map_err(msg)?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
}

/// "I handed this off": starts the before and after measurement.
#[tauri::command]
pub async fn report_handoff(app: AppHandle, id: i64) -> Reply<()> {
    blocking(app, move |state| {
        state
            .engine()?
            .write(move |store| {
                store
                    .record_handoff(id, clock::now(), "started", "")
                    .map(|_| ())
            })
            .map_err(msg)
    })
    .await
}

#[derive(Serialize)]
pub struct ResultRow {
    handoff_id: i64,
    workflow_id: i64,
    name: String,
    ts: i64,
    status: String,
    note: String,
    days_before: usize,
    days_after: usize,
    hours_before: Option<f64>,
    hours_after: Option<f64>,
    saved: Option<f64>,
    /// Days measured on the thinner side, out of `Results::min_days`.
    progress: usize,
}

#[derive(Serialize)]
pub struct Results {
    min_days: usize,
    rows: Vec<ResultRow>,
}

impl From<Measurement> for ResultRow {
    fn from(m: Measurement) -> Self {
        Self {
            saved: m.saved(),
            progress: m.progress(),
            handoff_id: m.handoff_id,
            workflow_id: m.workflow_id,
            name: m.name,
            ts: m.ts,
            status: m.status,
            note: m.note,
            days_before: m.days_before,
            days_after: m.days_after,
            hours_before: m.hours_before,
            hours_after: m.hours_after,
        }
    }
}

#[tauri::command]
pub async fn results(app: AppHandle) -> Reply<Results> {
    blocking(app, move |state| {
        let measured = state
            .engine()?
            .read(|store| store.results(clock::local()))
            .map_err(msg)?;
        Ok(Results {
            min_days: MIN_DAYS,
            rows: measured
                .into_iter()
                .take(MAX_ROWS)
                .map(Into::into)
                .collect(),
        })
    })
    .await
}

#[derive(Clone, Serialize)]
pub struct LineView {
    ts: i64,
    text: String,
}

#[derive(Clone, Serialize)]
pub struct SessionView {
    source: &'static str,
    app: String,
    title: String,
    url: String,
    start: i64,
    end: i64,
    lines: Vec<LineView>,
}

#[derive(Clone, Serialize)]
pub struct SearchPage {
    sessions: Vec<SessionView>,
}

fn session_view(session: Session, store: &Store) -> SessionView {
    let secret = store.secret();
    SessionView {
        source: match session.source {
            sextant_core::HitSource::Screen => "screen",
            sextant_core::HitSource::Mic => "mic",
            sextant_core::HitSource::System => "system",
        },
        app: session.app,
        title: redact(&session.title, &secret),
        url: redact(&session.url, &secret),
        start: session.start,
        end: session.end,
        lines: session
            .lines
            .into_iter()
            .map(|(ts, text)| LineView {
                ts,
                text: redact(&text, &secret),
            })
            .collect(),
    }
}

#[derive(Serialize)]
pub struct ReportRowView {
    categories: Vec<&'static str>,
    hours_week: f64,
    action: &'static str,
}

#[derive(Serialize)]
pub struct ReportView {
    from_ts: i64,
    to_ts: i64,
    hours_back: f64,
    hours_saved: f64,
    handoffs: usize,
    top: Vec<ReportRowView>,
}

/// The numbers and category names behind the share image. No entity, title, URL or text.
#[tauri::command]
pub async fn weekly_report(app: AppHandle) -> Reply<ReportView> {
    blocking(app, move |state| {
        let report = state
            .engine()?
            .read(|store| store.weekly_report(clock::local()))
            .map_err(msg)?;
        Ok(ReportView {
            from_ts: report.from_ts,
            to_ts: report.to_ts,
            hours_back: report.hours_back,
            hours_saved: report.hours_saved,
            handoffs: report.handoffs,
            top: report
                .top
                .into_iter()
                .map(|row| ReportRowView {
                    categories: row.categories.into_iter().map(Category::as_str).collect(),
                    hours_week: row.hours_week,
                    action: row.action.as_str(),
                })
                .collect(),
        })
    })
    .await
}

/// Writes the share image to the Downloads folder. Called only when the user presses Save.
/// `day` is the local date as `YYYY-MM-DD`.
#[tauri::command]
pub async fn save_report_image(day: String, png: Vec<u8>) -> Reply<String> {
    let shaped = day.len() == 10
        && day.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    if !shaped {
        return Err("the date must look like 2026-03-06".into());
    }
    if !png.starts_with(&PNG_MAGIC) || png.len() > MAX_IMAGE_BYTES {
        return Err("this is not a PNG image".into());
    }
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    let folder = PathBuf::from(home).join("Downloads");
    fs::create_dir_all(&folder).map_err(msg)?;
    let path = folder.join(format!("sextant-week-{day}.png"));
    fs::write(&path, png).map_err(msg)?;
    Ok(path.to_string_lossy().into_owned())
}

#[derive(Serialize)]
pub struct Storage {
    bytes: u64,
}

#[tauri::command]
pub async fn storage_used(state: State<'_, AppState>) -> Reply<Storage> {
    let dir = state.dir.clone();
    let bytes = tauri::async_runtime::spawn_blocking(move || data::storage_used(&dir))
        .await
        .map_err(msg)?;
    Ok(Storage { bytes })
}

#[tauri::command]
pub async fn set_excluded_apps(state: State<'_, AppState>, apps: Vec<String>) -> Reply<Settings> {
    let apps: Vec<String> = apps
        .into_iter()
        .map(|app| app.trim().to_owned())
        .filter(|app| !app.is_empty())
        .collect();
    state
        .engine()?
        .change_settings(move |s| s.excluded_apps = apps.clone())
        .map_err(msg)
}

#[tauri::command]
pub async fn set_live(state: State<'_, AppState>, live: Live) -> Reply<Settings> {
    state
        .engine()?
        .change_settings(move |s| {
            // Audio has its own switch, which also starts and stops the pipeline.
            s.live = Live {
                audio: s.live.audio,
                ..live
            };
        })
        .map_err(msg)
}

#[tauri::command]
pub async fn set_excluded_sites(state: State<'_, AppState>, sites: Vec<String>) -> Reply<Settings> {
    let sites: Vec<String> = sites
        .into_iter()
        .map(|site| site.trim().to_lowercase())
        .filter(|site| !site.is_empty())
        .collect();
    state
        .engine()?
        .change_settings(move |s| s.excluded_sites = sites.clone())
        .map_err(msg)
}

/// Caps raw data at 1, 2, 5 or 10 GB, or not at all (`None`), and applies it now.
#[tauri::command]
pub async fn set_storage_cap(app: AppHandle, mb: Option<u32>) -> Reply<Settings> {
    blocking(app, move |state| {
        if !STORAGE_CAP_CHOICES.contains(&mb) {
            return Err("choose 1, 2, 5 or 10 GB, or no cap".into());
        }
        let engine = state.engine()?;
        let settings = engine
            .change_settings(move |s| s.storage_cap_mb = mb)
            .map_err(msg)?;
        if let Some(mb) = mb {
            engine
                .write(move |store| {
                    store
                        .cap_storage(clock::local(), u64::from(mb) * 1_000_000)
                        .map(|_| ())
                })
                .map_err(msg)?;
        }
        Ok(settings)
    })
    .await
}

/// Deletes everything captured in the last 15 minutes, hour or day.
#[tauri::command]
pub async fn forget_recent(app: AppHandle, minutes: i64) -> Reply<()> {
    blocking(app, move |state| {
        if !FORGET_CHOICES.contains(&minutes) {
            return Err("choose 15 minutes, an hour or a day".into());
        }
        let when = clock::local();
        state
            .engine()?
            .write(move |store| {
                store
                    .forget(when, when.now - minutes * 60, i64::MAX)
                    .map(|_| ())
            })
            .map_err(msg)
    })
    .await
}

/// What agents received: each MCP answer and each bridge run, newest first.
#[tauri::command]
pub async fn sent_log(app: AppHandle) -> Reply<Vec<Sent>> {
    blocking(app, |state| {
        state
            .engine()?
            .read(|store| store.sent(SENT_ROWS))
            .map_err(msg)
    })
    .await
}

/// Keeps raw data for 30, 90 or 365 days, or forever (`None`), and applies it now.
#[tauri::command]
pub async fn set_retention(app: AppHandle, days: Option<u32>) -> Reply<Settings> {
    blocking(app, move |state| {
        if !RETENTION_CHOICES.contains(&days) {
            return Err("choose 30, 90, 365 days or forever".into());
        }
        let engine = state.engine()?;
        let settings = engine
            .change_settings(move |s| s.retention_days = days)
            .map_err(msg)?;
        if let Some(days) = days {
            engine
                .write(move |store| store.retention(clock::local(), days).map(|_| ()))
                .map_err(msg)?;
        }
        Ok(settings)
    })
    .await
}

/// Stops capture and audio, closes the database, empties the data folder, deletes the key and
/// starts the app again. The UI asks for a confirmation first.
#[tauri::command]
pub async fn delete_everything(app: AppHandle) -> Reply<()> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        handle.state::<AppState>().delete_everything(&Keychain)
    })
    .await
    .map_err(msg)?
    .map_err(|err| format!("{err:#}"))?;
    app.restart()
}

/// "Delete everything", then takes Sextant out of the Claude Desktop config, moves the app to
/// the Trash and quits. Dev builds keep the app.
#[tauri::command]
pub async fn uninstall(app: AppHandle) -> Reply<()> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<()> {
        handle.state::<AppState>().delete_everything(&Keychain)?;
        connect::remove_from_desktop_config(&connect::desktop_config_path()?)?;
        if !cfg!(debug_assertions) {
            platform::trash_app()?;
        }
        Ok(())
    })
    .await
    .map_err(msg)?
    .map_err(|err| format!("{err:#}"))?;
    app.exit(0);
    Ok(())
}

const REWIND_RESULTS: usize = 5;
const REWIND_PAGE: usize = 20;

#[derive(Serialize)]
pub struct TurnView {
    id: i64,
    ts: i64,
    question: String,
    answer: String,
}

impl From<Turn> for TurnView {
    fn from(turn: Turn) -> Self {
        Self {
            id: turn.id,
            ts: turn.ts,
            question: turn.question,
            answer: turn.answer,
        }
    }
}

#[derive(Serialize)]
pub struct Asked {
    turn: TurnView,
    page: SearchPage,
}

/// What `rewind_ask` streams while it works: the sessions as soon as they are found, then the
/// answer text so far as the model writes it.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Progress {
    Sessions { page: SearchPage },
    Answer { text: String },
}

const DAY_S: i64 = 86_400;
/// Lines per session shown to the model, and characters per line, so a prompt with five
/// sessions stays well inside the model's 4,096-token context.
const NOTE_LINES: usize = 6;
const NOTE_CHARS: usize = 200;

/// The span a plan's `time` names, in whole local days: today is 0, yesterday 1.
fn days_back(time: &str) -> Option<(i64, i64)> {
    match time {
        "today" => Some((0, 0)),
        "yesterday" => Some((1, 1)),
        "this week" => Some((6, 0)),
        "last week" => Some((13, 7)),
        "this month" => Some((30, 0)),
        _ => None,
    }
}

/// From local midnight `since` days ago to the end of the day `until` days ago.
fn plan_range(now: i64, time: &str) -> (Option<i64>, Option<i64>) {
    let Some((since, until)) = days_back(time) else {
        return (None, None);
    };
    let midnight = now - (now + i64::from(clock::utc_offset(now))).rem_euclid(DAY_S);
    (
        Some(midnight - since * DAY_S),
        Some(midnight - (until - 1) * DAY_S - 1),
    )
}

/// Sessions for a question, best first. The on-device model picks the terms and the time span
/// when it can; otherwise the question's keywords are searched over all time. A span that finds
/// nothing is dropped, since people often misremember when.
fn recall_views(state: &AppState, question: &str, now: i64) -> Result<Vec<SessionView>, String> {
    let settings = crate::prefs::load(&state.dir).map_err(msg)?;
    let (mut terms, range) = match platform::language::plan(question) {
        Some(plan) => (plan.terms, plan_range(now, &plan.time)),
        None => (Vec::new(), (None, None)),
    };
    if terms.iter().all(|term| term.trim().is_empty()) {
        terms = keywords(question);
    }
    if terms.is_empty() {
        return Err("Add a name, topic or word you remember.".into());
    }
    state
        .engine()?
        .read(move |store| {
            let mut found = store.recall(&settings, &terms, range, REWIND_RESULTS)?;
            if found.is_empty() && range != (None, None) {
                found = store.recall(&settings, &terms, (None, None), REWIND_RESULTS)?;
            }
            Ok(found.into_iter().map(|s| session_view(s, store)).collect())
        })
        .map_err(msg)
}

/// A short answer from the on-device model that cites one session, which moves to the top.
/// Empty when the model is unavailable or nothing was found.
fn answer(
    question: &str,
    sessions: &mut [SessionView],
    on_text: impl FnMut(&str) + Send,
) -> String {
    let notes: Vec<Note> = sessions
        .iter()
        .map(|session| Note {
            app: session.app.clone(),
            title: session.title.clone(),
            ts: session.end,
            lines: session
                .lines
                .iter()
                .take(NOTE_LINES)
                .map(|line| line.text.chars().take(NOTE_CHARS).collect())
                .collect(),
        })
        .collect();
    if notes.is_empty() {
        return String::new();
    }
    let Some(answer) = platform::language::answer(question, &notes, on_text) else {
        return String::new();
    };
    if (1..=sessions.len()).contains(&answer.note) {
        sessions[..answer.note].rotate_right(1);
    }
    answer.text
}

/// Answers a question in the rewind search panel and adds it to the history. Streams the sessions
/// and then the answer to `progress`, so the panel shows them before the turn is saved.
#[tauri::command]
pub async fn rewind_ask(
    app: AppHandle,
    question: String,
    progress: Channel<Progress>,
) -> Reply<Asked> {
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err("Type a question first.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let now = clock::now();
        let mut sessions = recall_views(&state, &question, now)?;
        progress
            .send(Progress::Sessions {
                page: SearchPage {
                    sessions: sessions.clone(),
                },
            })
            .map_err(msg)?;
        let text = answer(&question, &mut sessions, |text| {
            if let Err(err) = progress.send(Progress::Answer { text: text.into() }) {
                eprintln!("streaming the answer failed: {err}");
            }
        });
        let turn = state
            .engine()?
            .write(move |store| store.add_turn(now, &question, &text))
            .map_err(msg)?;
        Ok(Asked {
            turn: turn.into(),
            page: SearchPage { sessions },
        })
    })
    .await
    .map_err(msg)?
}

/// The sessions for a past question, found again so retention and exclusions apply.
#[tauri::command]
pub async fn rewind_results(app: AppHandle, question: String) -> Reply<SearchPage> {
    tauri::async_runtime::spawn_blocking(move || {
        let sessions = recall_views(&app.state::<AppState>(), &question, clock::now())?;
        Ok(SearchPage { sessions })
    })
    .await
    .map_err(msg)?
}

/// Up to 20 past questions older than turn `before` (or the newest), newest first.
#[tauri::command]
pub async fn rewind_history(app: AppHandle, before: Option<i64>) -> Reply<Vec<TurnView>> {
    blocking(app, move |state| {
        state
            .engine()?
            .read(|store| store.turns(before, REWIND_PAGE))
            .map(|turns| turns.into_iter().map(TurnView::from).collect())
            .map_err(msg)
    })
    .await
}

/// Fits the panel to its content, `height` logical pixels.
#[tauri::command]
pub async fn rewind_fit(app: AppHandle, height: f64) -> Reply<()> {
    crate::rewind::fit_panel(&app, height).map_err(msg)
}

#[tauri::command]
pub async fn rewind_hide(app: AppHandle) -> Reply<()> {
    crate::rewind::hide(&app).map_err(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_time_becomes_whole_local_days_before_now() {
        let now = 1_800_000_000;
        let (Some(from), Some(to)) = plan_range(now, "yesterday") else {
            panic!("yesterday has a span");
        };
        assert_eq!(to - from + 1, DAY_S);
        assert!(to < now && now - to <= DAY_S);
        let (Some(week), Some(today_end)) = plan_range(now, "this week") else {
            panic!("this week has a span");
        };
        assert_eq!(today_end - week + 1, 7 * DAY_S);
        assert!(today_end >= now);
        assert_eq!(plan_range(now, "any time"), (None, None));
    }
}
