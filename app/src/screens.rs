//! The typed commands behind Workflows, Search, Results, Privacy and the weekly report
//! (plan.md §9). Reads go through the read-only connection, changes through the worker thread.
//! Text from the database is redacted again on the way out, as in the MCP server.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sextant_core::{
    Action, Category, Enjoyment, Kind, Live, MIN_DAYS, Measurement, Session, Settings, Store,
    Target, Workflow, WorkflowDetail, parse_correction, redact, skill,
};
use tauri::{AppHandle, Manager, State};

use crate::commands::{AppState, MAX_ROWS, Reply, msg};
use crate::keys::Keychain;
use crate::{clock, data};

const SKILLS_FOLDER: &str = "skills";
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
const MAX_IMAGE_BYTES: usize = 8 << 20;
const RETENTION_CHOICES: [Option<u32>; 4] = [Some(30), Some(90), Some(365), None];

#[derive(Serialize)]
pub struct StepView {
    key: String,
    name: String,
    category: &'static str,
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
                category: step.category.as_str(),
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
        cases: workflow.case_count,
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
pub async fn workflow_overview(
    state: State<'_, AppState>,
    offset: usize,
    limit: usize,
) -> Reply<Overview> {
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
}

/// The answer to the quick enjoyment question. Stored as a correction, never inferred.
#[tauri::command]
pub async fn answer_enjoyment(state: State<'_, AppState>, id: i64, enjoyment: String) -> Reply<()> {
    let enjoyment = Enjoyment::parse(&enjoyment).ok_or("unknown answer")?;
    state
        .engine()?
        .change(move |store| {
            store
                .answer_enjoyment(clock::now(), Target::Workflow(id), enjoyment)
                .map(|_| ())
        })
        .map_err(msg)
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
pub async fn workflow_detail(state: State<'_, AppState>, id: i64) -> Reply<Option<Detail>> {
    state
        .engine()?
        .read(|store| {
            store
                .workflow_detail(id)?
                .map(|found| detail(store, found))
                .transpose()
        })
        .map_err(msg)
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
    state: State<'_, AppState>,
    id: i64,
    phrase: String,
    steps: Vec<String>,
) -> Reply<Option<Understood>> {
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
    state: State<'_, AppState>,
    id: i64,
    folder: Option<String>,
) -> Reply<AutomationPlan> {
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
}

/// Writes the `SKILL.md` and returns its path.
#[tauri::command]
pub async fn write_skill(
    state: State<'_, AppState>,
    id: i64,
    folder: Option<String>,
) -> Reply<String> {
    let (workflow, text) = state
        .engine()?
        .read(|store| {
            let text = redact(&store.skill_md(id)?, &store.secret());
            Ok((workflow_of(store, id)?, text))
        })
        .map_err(msg)?;
    let path = skill_path(&state.dir, folder.as_deref(), &workflow)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(msg)?;
    }
    fs::write(&path, text).map_err(msg)?;
    Ok(path.to_string_lossy().into_owned())
}

/// "I handed this off": starts the before and after measurement.
#[tauri::command]
pub async fn report_handoff(state: State<'_, AppState>, id: i64) -> Reply<()> {
    state
        .engine()?
        .write(move |store| {
            store
                .record_handoff(id, clock::now(), "started", "")
                .map(|_| ())
        })
        .map_err(msg)
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
pub async fn results(state: State<'_, AppState>) -> Reply<Results> {
    let measured = state
        .engine()?
        .read(|store| store.results(clock::now()))
        .map_err(msg)?;
    Ok(Results {
        min_days: MIN_DAYS,
        rows: measured
            .into_iter()
            .take(MAX_ROWS)
            .map(Into::into)
            .collect(),
    })
}

#[derive(Serialize)]
pub struct LineView {
    ts: i64,
    text: String,
}

#[derive(Serialize)]
pub struct SessionView {
    source: &'static str,
    app: String,
    title: String,
    url: String,
    start: i64,
    end: i64,
    lines: Vec<LineView>,
}

#[derive(Serialize)]
pub struct SearchPage {
    sessions: Vec<SessionView>,
    /// Pass as `offset` for the next page; `None` on the last one.
    next_offset: Option<usize>,
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

/// One box over screen text and transcripts, grouped by session, newest first.
#[tauri::command]
pub async fn search(
    state: State<'_, AppState>,
    query: String,
    offset: usize,
    limit: usize,
) -> Reply<SearchPage> {
    let settings = crate::prefs::load(&state.dir).map_err(msg)?;
    if query.trim().is_empty() {
        return Ok(SearchPage {
            sessions: Vec::new(),
            next_offset: None,
        });
    }
    state
        .engine()?
        .read(|store| {
            let page = store.search_sessions(
                &settings,
                &query,
                (None, None),
                None,
                limit.clamp(1, MAX_ROWS),
                offset,
            )?;
            Ok(SearchPage {
                next_offset: page.next_offset,
                sessions: page
                    .sessions
                    .into_iter()
                    .map(|s| session_view(s, store))
                    .collect(),
            })
        })
        .map_err(msg)
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
pub async fn weekly_report(state: State<'_, AppState>) -> Reply<ReportView> {
    let report = state
        .engine()?
        .read(|store| store.weekly_report(clock::now()))
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

/// Keeps raw data for 30, 90 or 365 days, or forever (`None`), and applies it now.
#[tauri::command]
pub async fn set_retention(state: State<'_, AppState>, days: Option<u32>) -> Reply<Settings> {
    if !RETENTION_CHOICES.contains(&days) {
        return Err("choose 30, 90, 365 days or forever".into());
    }
    let engine = state.engine()?;
    let settings = engine
        .change_settings(move |s| s.retention_days = days)
        .map_err(msg)?;
    if let Some(days) = days {
        engine
            .write(move |store| store.retention(clock::now(), days).map(|_| ()))
            .map_err(msg)?;
    }
    Ok(settings)
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
