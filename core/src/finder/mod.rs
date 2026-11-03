//! Turns events into what the routine tools read (plan.md §7): activities, steps that name them,
//! entities and tasks. `refresh(now)` rebuilds everything from the last 90 days.

mod actions;
mod activities;
mod entities;
mod steps;
mod tasks;

use rusqlite::{Connection, params};

use self::activities::EventRow;
pub use self::entities::ValueType;
use crate::error::Result;
use crate::model::Payload;
use crate::store::Store;
use crate::text::fnv1a;
use crate::time::{DAY, Local};

const WINDOW_DAYS: i64 = 90;

/// Row counts of one refresh, for logs. Never text.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RefreshStats {
    pub activities: usize,
    pub steps: usize,
    pub entities: usize,
    pub tasks: usize,
}

struct Analysis {
    activities: Vec<activities::Activity>,
    steps: Vec<steps::Step>,
    entities: Vec<entities::Entity>,
    tasks: Vec<tasks::Task>,
}

fn analyze(events: &[EventRow], now: i64) -> Analysis {
    let mut activities = activities::build(events, now);
    let steps = steps::build(&mut activities);
    let entities = entities::build(&activities);
    let acts = activity_actions(events, &activities);
    let tasks = tasks::build(&activities, &entities, &acts);
    Analysis {
        activities,
        steps,
        entities,
        tasks,
    }
}

/// The presses and edits of each activity, oldest first. Each belongs to the activity that spans its time.
fn activity_actions(
    past: &[EventRow],
    activities: &[activities::Activity],
) -> Vec<Vec<actions::Act>> {
    let presses: Vec<&EventRow> = past
        .iter()
        .filter(|e| e.kind == "press" || e.kind == "edit")
        .collect();
    let spans: Vec<(i64, i64)> = activities.iter().map(|e| (e.start, e.end)).collect();
    let times: Vec<i64> = presses.iter().map(|e| e.ts).collect();
    let mut out: Vec<Vec<actions::Act>> = (0..activities.len()).map(|_| Vec::new()).collect();
    for (press, activity) in presses.iter().zip(actions::attach(&spans, &times)) {
        if let Some(activity) = activity {
            out[activity].push(actions::act(&press.payload));
        }
    }
    out
}

fn load_events(conn: &Connection, from: i64, to: i64) -> Result<Vec<EventRow>> {
    let mut statement = conn.prepare_cached(
        "SELECT ts, kind, json FROM events WHERE ts BETWEEN ?1 AND ?2 ORDER BY ts, id",
    )?;
    let rows = statement.query_map(params![from, to], |row| {
        let json: String = row.get(2)?;
        Ok(EventRow {
            ts: row.get(0)?,
            kind: row.get(1)?,
            payload: serde_json::from_str::<Payload>(&json).unwrap_or_default(),
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

fn id(index: usize) -> i64 {
    index as i64 + 1
}

/// Fits a JS number, so MCP clients can hold it.
fn stable_id(variant: &str) -> i64 {
    (fnv1a(variant) >> 16) as i64
}

/// Survives refreshes: an activity keeps its id while its start and object stay the same. A task
/// takes the id of its first activity.
fn activity_id(activity: &activities::Activity) -> i64 {
    stable_id(&format!("{}|{}", activity.start, activity.key))
}

/// Root of `index` in a union-find forest, halving the path on the way.
fn find(parent: &mut [usize], mut index: usize) -> usize {
    while parent[index] != index {
        parent[index] = parent[parent[index]];
        index = parent[index];
    }
    index
}

fn persist(conn: &mut Connection, analysis: &Analysis) -> Result<()> {
    let tx = conn.transaction()?;
    for table in ["activities", "steps", "tasks", "task_activities"] {
        tx.execute(&format!("DELETE FROM {table}"), [])?;
    }
    {
        let activity_ids: Vec<i64> = analysis.activities.iter().map(activity_id).collect();
        let mut insert = tx.prepare("INSERT INTO activities(id, start_ts, end_ts, app, object_key, step_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
        for (e, &activity) in analysis.activities.iter().zip(&activity_ids) {
            insert.execute(params![activity, e.start, e.end, e.app, e.key, id(e.step)])?;
        }
        let mut insert = tx.prepare("INSERT INTO steps(id, key, name) VALUES (?1, ?2, ?3)")?;
        for (i, s) in analysis.steps.iter().enumerate() {
            insert.execute(params![id(i), s.key, s.name])?;
        }
        let mut insert = tx.prepare(
            "INSERT INTO tasks(id, label, first_ts, last_ts, active_s) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        let mut activity_link =
            tx.prepare("INSERT INTO task_activities(task_id, activity_id) VALUES (?1, ?2)")?;
        for t in &analysis.tasks {
            let task_id = activity_ids[t.activities[0]];
            insert.execute(params![task_id, t.label(), t.first, t.last, t.active_s])?;
            for &activity in &t.activities {
                activity_link.execute(params![task_id, activity_ids[activity]])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

impl Store {
    /// Rebuilds activities, steps, entities and tasks from the last 90 days. Only events up to
    /// `when.now` count.
    pub fn refresh(&mut self, when: Local) -> Result<RefreshStats> {
        let events = load_events(&self.conn, when.now - WINDOW_DAYS * DAY, when.now)?;
        let analysis = analyze(&events, when.now);
        persist(&mut self.conn, &analysis)?;
        Ok(RefreshStats {
            activities: analysis.activities.len(),
            steps: analysis.steps.len(),
            entities: analysis.entities.len(),
            tasks: analysis.tasks.len(),
        })
    }
}
