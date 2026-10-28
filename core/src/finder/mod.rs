//! The automation finder (plan.md §7): events to episodes, steps, entities, follows, cases,
//! workflows, commitments and scores. `refresh(now)` rebuilds everything from the last 90 days.

mod commitments;
mod entities;
mod episodes;
mod graph;
mod score;
mod steps;

use std::collections::{BTreeSet, HashMap};

use rusqlite::{Connection, params};

use self::episodes::EventRow;
pub use self::score::{Action, Enjoyment, Kind};
pub use self::steps::Category;
use crate::corrections::Overrides;
use crate::error::Result;
use crate::model::Payload;
use crate::store::Store;
use crate::text::fnv1a;
use crate::time::{DAY, day};

const WINDOW_DAYS: i64 = 90;
const KNOWN_KINDS: &str = "'focus', 'idle', 'pause', 'visit', 'calendar', 'copy', 'reminder'";

/// Row counts of one refresh, for logs. Never text.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RefreshStats {
    pub episodes: usize,
    pub steps: usize,
    pub entities: usize,
    pub follows: usize,
    pub cases: usize,
    pub workflows: usize,
    pub commitments: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepInfo {
    pub key: String,
    pub name: String,
    pub category: Category,
    pub minutes: f64,
    pub potential: f64,
}

/// A ranked workflow. Every score has a line in `reasons`.
#[derive(Debug, Clone, PartialEq)]
pub struct Workflow {
    /// Stable for a given step sequence.
    pub id: i64,
    pub name: String,
    /// Step keys joined with " > ". Corrections attach to it.
    pub variant: String,
    /// `cases` (one entity, many cases) or `chain` (routine without a shared entity).
    pub basis: String,
    pub steps: Vec<StepInfo>,
    pub case_count: usize,
    pub hours_week: f64,
    pub potential: f64,
    pub desire: f64,
    pub benefit: f64,
    pub procrastination: f64,
    pub kind: Kind,
    pub action: Action,
    pub first_step: String,
    pub enjoyment: Enjoyment,
    pub client: bool,
    pub rough: bool,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyPoint {
    pub from: String,
    pub to: String,
    pub copies: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseExample {
    pub entity: String,
    pub first_ts: i64,
    pub last_ts: i64,
    /// Step names in time order, repeats collapsed.
    pub path: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCommitment {
    pub ts: i64,
    pub source: String,
    pub text: String,
    pub due_ts: Option<i64>,
    pub overdue: bool,
}

/// Everything `get_workflow` and the `SKILL.md` builder need.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowDetail {
    pub workflow: Workflow,
    pub copy_points: Vec<CopyPoint>,
    pub examples: Vec<CaseExample>,
    pub commitments: Vec<OpenCommitment>,
}

struct Analysis {
    episodes: Vec<episodes::Episode>,
    steps: Vec<steps::Step>,
    step_scores: Vec<score::StepScore>,
    entities: Vec<entities::Entity>,
    follows: Vec<graph::Follow>,
    cases: Vec<entities::Case>,
    workflows: Vec<(graph::Workflow, score::Scored)>,
    commitments: Vec<commitments::Commitment>,
}

fn analyze(
    events: &[EventRow],
    spoken: &[(i64, String)],
    overrides: &Overrides,
    now: i64,
    utc_offset_s: i32,
) -> Analysis {
    let mut episodes = episodes::build(events, now);
    let steps = steps::build(
        &mut episodes,
        &overrides.field("merge_into"),
        &overrides.field("name"),
    );
    let step_scores = score::score_steps(&steps, &episodes, utc_offset_s);
    let (entities, mut cases) = entities::build(&episodes);
    let copies: Vec<(i64, &str)> = events
        .iter()
        .filter(|e| e.kind == "copy")
        .map(|e| (e.ts, e.payload.app.as_str()))
        .collect();
    let follows = graph::follows(&episodes, &copies, utc_offset_s);
    let mut workflows = graph::from_cases(&mut cases, &steps, &overrides.keys_with("split"));
    let routines = graph::from_chains(
        graph::chains(&follows),
        &follows,
        &episodes,
        &steps,
        &workflows,
    );
    workflows.extend(routines);
    let commitments = commitments::build(events, spoken, &entities, &cases, &episodes, now);
    let observed_days = episodes
        .iter()
        .map(|e| day(e.start, utc_offset_s))
        .collect::<BTreeSet<_>>()
        .len();
    let cx = score::Context {
        episodes: &episodes,
        steps: &steps,
        step_scores: &step_scores,
        follows: &follows,
        cases: &cases,
        commitments: &commitments,
        overrides,
        observed_days,
        now,
    };
    let workflows = workflows
        .into_iter()
        .map(|w| {
            let scored = score::score_workflow(&w, &cx);
            (w, scored)
        })
        .collect();
    Analysis {
        episodes,
        steps,
        step_scores,
        entities,
        follows,
        cases,
        workflows,
        commitments,
    }
}

fn load_events(conn: &Connection, from: i64, to: i64) -> Result<Vec<EventRow>> {
    let mut statement = conn.prepare_cached(&format!(
        "SELECT ts, kind, source, json FROM events WHERE ts BETWEEN ?1 AND ?2 AND kind IN ({KNOWN_KINDS}) ORDER BY ts, id"
    ))?;
    let rows = statement.query_map(params![from, to], |row| {
        let json: String = row.get(3)?;
        Ok(EventRow {
            ts: row.get(0)?,
            kind: row.get(1)?,
            live: row.get::<_, String>(2)? == "live",
            payload: serde_json::from_str::<Payload>(&json).unwrap_or_default(),
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

fn load_spoken(conn: &Connection, from: i64, to: i64) -> Result<Vec<(i64, String)>> {
    let mut statement =
        conn.prepare_cached("SELECT ts_start, text FROM utterances WHERE source = 'mic' AND ts_start BETWEEN ?1 AND ?2 ORDER BY ts_start")?;
    let rows = statement.query_map(params![from, to], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

fn id(index: usize) -> i64 {
    index as i64 + 1
}

fn n(count: usize) -> i64 {
    count as i64
}

/// Fits a JS number, so MCP clients can hold it.
fn stable_id(variant: &str) -> i64 {
    (fnv1a(variant) >> 16) as i64
}

fn persist(conn: &mut Connection, analysis: &Analysis) -> Result<()> {
    let tx = conn.transaction()?;
    for table in [
        "episodes",
        "steps",
        "entities",
        "mentions",
        "follows",
        "cases",
        "workflows",
        "commitments",
    ] {
        tx.execute(&format!("DELETE FROM {table}"), [])?;
    }
    {
        let mut insert = tx.prepare("INSERT INTO episodes(id, start_ts, end_ts, app, object_key, step_id, live) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)")?;
        for (i, e) in analysis.episodes.iter().enumerate() {
            insert.execute(params![
                id(i),
                e.start,
                e.end,
                e.app,
                e.key,
                id(e.step),
                e.live
            ])?;
        }
        let mut insert = tx.prepare("INSERT INTO steps(id, key, name, category, episodes, minutes, days, potential, reasons) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)")?;
        for (i, (s, score)) in analysis.steps.iter().zip(&analysis.step_scores).enumerate() {
            let reasons = serde_json::to_string(&score.reasons)?;
            insert.execute(params![
                id(i),
                s.key,
                s.name,
                s.category.as_str(),
                n(s.episodes.len()),
                score.minutes,
                n(score.days),
                score.potential,
                reasons
            ])?;
        }
        let mut insert =
            tx.prepare("INSERT INTO entities(id, kind, value, episodes) VALUES (?1, ?2, ?3, ?4)")?;
        let mut mention =
            tx.prepare("INSERT INTO mentions(entity_id, episode_id) VALUES (?1, ?2)")?;
        for (i, e) in analysis.entities.iter().enumerate() {
            insert.execute(params![
                id(i),
                e.kind.as_str(),
                e.value,
                n(e.episodes.len())
            ])?;
            for episode in &e.episodes {
                mention.execute(params![id(i), id(*episode)])?;
            }
        }
        let mut insert = tx.prepare("INSERT INTO follows(from_step, to_step, count, median_gap_s, copies, days) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
        for f in &analysis.follows {
            insert.execute(params![
                id(f.from),
                id(f.to),
                n(f.count),
                f.median_gap_s,
                n(f.copies),
                n(f.days)
            ])?;
        }
        let mut insert = tx.prepare(
            "INSERT INTO cases(entity_id, first_ts, last_ts, variant) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for c in &analysis.cases {
            let variant = c.workflow.map(|w| analysis.workflows[w].0.variant.as_str());
            insert.execute(params![id(c.entity), c.first, c.last, variant])?;
        }
        let mut insert = tx.prepare(
            "INSERT OR IGNORE INTO workflows(id, variant, basis, name, sequence, case_count, hours_week, potential, desire, benefit, procrastination, kind, action, first_step, copies, enjoyment, client, hidden, rough, reasons)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
        )?;
        for (w, s) in &analysis.workflows {
            let sequence: Vec<&str> = w
                .sequence
                .iter()
                .map(|&i| analysis.steps[i].key.as_str())
                .collect();
            let copies: Vec<(&str, &str, usize)> = s
                .copies
                .iter()
                .map(|&(a, b, n)| {
                    (
                        analysis.steps[a].key.as_str(),
                        analysis.steps[b].key.as_str(),
                        n,
                    )
                })
                .collect();
            insert.execute(params![
                stable_id(&w.variant),
                w.variant,
                if matches!(w.basis, graph::Basis::Cases) {
                    "cases"
                } else {
                    "chain"
                },
                s.name,
                serde_json::to_string(&sequence)?,
                n(w.count),
                s.hours_week,
                s.potential,
                s.desire,
                s.benefit,
                s.procrastination,
                s.kind.as_str(),
                s.action.as_str(),
                s.first_step,
                serde_json::to_string(&copies)?,
                s.enjoyment.as_str(),
                s.client,
                s.hidden,
                s.rough,
                serde_json::to_string(&s.reasons)?,
            ])?;
        }
        let mut insert = tx.prepare("INSERT INTO commitments(ts, source, text, due_ts, entity_id, overdue) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
        for c in &analysis.commitments {
            insert.execute(params![
                c.ts,
                c.source,
                c.text,
                c.due,
                c.entity.map(id),
                c.overdue
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

impl Store {
    /// Rebuilds episodes, steps, entities, follows, cases, workflows, commitments and scores
    /// from the last 90 days. Corrections apply here.
    pub fn refresh(&mut self, now: i64) -> Result<RefreshStats> {
        let from = now - WINDOW_DAYS * DAY;
        let events = load_events(&self.conn, from, now)?;
        let spoken = load_spoken(&self.conn, from, now)?;
        let overrides = Overrides::load(&self.conn)?;
        let analysis = analyze(&events, &spoken, &overrides, now, self.utc_offset_s);
        persist(&mut self.conn, &analysis)?;
        Ok(RefreshStats {
            episodes: analysis.episodes.len(),
            steps: analysis.steps.len(),
            entities: analysis.entities.len(),
            follows: analysis.follows.len(),
            cases: analysis.cases.len(),
            workflows: analysis.workflows.len(),
            commitments: analysis.commitments.len(),
        })
    }

    /// Visible workflows, best estimated benefit first.
    pub fn workflows(&self) -> Result<Vec<Workflow>> {
        self.read_workflows(
            "WHERE hidden = 0 ORDER BY benefit DESC, hours_week DESC, id",
            params![],
        )
    }

    pub fn workflow(&self, id: i64) -> Result<Option<Workflow>> {
        Ok(self
            .read_workflows("WHERE id = ?1", params![id])?
            .into_iter()
            .next())
    }

    /// §7.11 The 3 biggest visible workflows whose enjoyment is still unknown.
    pub fn ask_first(&self) -> Result<Vec<Workflow>> {
        self.read_workflows(
            "WHERE hidden = 0 AND enjoyment = 'unknown' ORDER BY hours_week DESC, id LIMIT 3",
            params![],
        )
    }

    fn read_workflows(&self, clause: &str, args: impl rusqlite::Params) -> Result<Vec<Workflow>> {
        let steps = self.step_infos()?;
        let mut statement = self.conn.prepare(&format!(
            "SELECT id, name, variant, basis, sequence, case_count, hours_week, potential, desire, benefit, procrastination,
                    kind, action, first_step, enjoyment, client, rough, reasons FROM workflows {clause}"
        ))?;
        let rows = statement.query_map(args, |row| {
            let text = |i: usize| row.get::<_, String>(i);
            Ok((
                Workflow {
                    id: row.get(0)?,
                    name: text(1)?,
                    variant: text(2)?,
                    basis: text(3)?,
                    steps: Vec::new(),
                    case_count: row.get::<_, i64>(5)? as usize,
                    hours_week: row.get(6)?,
                    potential: row.get(7)?,
                    desire: row.get(8)?,
                    benefit: row.get(9)?,
                    procrastination: row.get(10)?,
                    kind: Kind::parse(&text(11)?).unwrap_or(Kind::LongTail),
                    action: Action::parse(&text(12)?).unwrap_or(Action::Watch),
                    first_step: text(13)?,
                    enjoyment: Enjoyment::parse(&text(14)?).unwrap_or(Enjoyment::Unknown),
                    client: row.get(15)?,
                    rough: row.get(16)?,
                    reasons: Vec::new(),
                },
                text(4)?,
                text(17)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (mut workflow, sequence, reasons) = row?;
            let keys: Vec<String> = serde_json::from_str(&sequence)?;
            workflow.steps = keys.iter().filter_map(|k| steps.get(k).cloned()).collect();
            workflow.reasons = serde_json::from_str(&reasons)?;
            out.push(workflow);
        }
        Ok(out)
    }

    fn step_infos(&self) -> Result<HashMap<String, StepInfo>> {
        let mut statement = self
            .conn
            .prepare_cached("SELECT key, name, category, minutes, potential FROM steps")?;
        let rows = statement.query_map([], |row| {
            let key: String = row.get(0)?;
            Ok((
                key.clone(),
                StepInfo {
                    key,
                    name: row.get(1)?,
                    category: Category::parse(&row.get::<_, String>(2)?).unwrap_or(Category::Other),
                    minutes: row.get(3)?,
                    potential: row.get(4)?,
                },
            ))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Steps, copy points, up to 3 example cases and linked commitments of one workflow.
    pub fn workflow_detail(&self, id: i64) -> Result<Option<WorkflowDetail>> {
        let Some(workflow) = self.workflow(id)? else {
            return Ok(None);
        };
        let steps = self.step_infos()?;
        let name = |key: &str| {
            steps
                .get(key)
                .map_or_else(|| key.to_string(), |s| s.name.clone())
        };
        let copies: String = self.conn.query_row(
            "SELECT copies FROM workflows WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        let copies: Vec<(String, String, usize)> = serde_json::from_str(&copies)?;
        let copy_points = copies
            .iter()
            .map(|(a, b, n)| CopyPoint {
                from: name(a),
                to: name(b),
                copies: *n,
            })
            .collect();

        let mut examples = Vec::new();
        let mut cases = self.conn.prepare(
            "SELECT c.entity_id, e.value, c.first_ts, c.last_ts FROM cases c JOIN entities e ON e.id = c.entity_id
             WHERE c.variant = ?1 ORDER BY c.first_ts, c.entity_id LIMIT 3",
        )?;
        let mut path = self.conn.prepare(
            "SELECT s.name FROM mentions m JOIN episodes ep ON ep.id = m.episode_id JOIN steps s ON s.id = ep.step_id
             WHERE m.entity_id = ?1 ORDER BY ep.start_ts, ep.id",
        )?;
        let rows = cases.query_map(params![workflow.variant], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        for row in rows {
            let (entity_id, entity, first_ts, last_ts) = row?;
            let mut names: Vec<String> = path
                .query_map(params![entity_id], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            names.dedup();
            examples.push(CaseExample {
                entity,
                first_ts,
                last_ts,
                path: names,
            });
        }

        let mut statement = self.conn.prepare(
            "SELECT m.ts, m.source, m.text, m.due_ts, m.overdue FROM commitments m JOIN cases c ON c.entity_id = m.entity_id
             WHERE c.variant = ?1 ORDER BY m.ts, m.id",
        )?;
        let commitments = statement
            .query_map(params![workflow.variant], |row| {
                Ok(OpenCommitment {
                    ts: row.get(0)?,
                    source: row.get(1)?,
                    text: row.get(2)?,
                    due_ts: row.get(3)?,
                    overdue: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(Some(WorkflowDetail {
            workflow,
            copy_points,
            examples,
            commitments,
        }))
    }
}
