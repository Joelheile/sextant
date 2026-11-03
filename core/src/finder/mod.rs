//! Turns events into what the routine tools read (plan.md §7): episodes, steps that name them,
//! entities and tasks. `refresh(now)` rebuilds everything from the last 90 days.

mod actions;
mod entities;
mod episodes;
mod steps;
mod tasks;

use rusqlite::{Connection, params};

pub use self::entities::ValueType;
use self::episodes::EventRow;
use crate::error::Result;
use crate::model::{EventKind, Payload};
use crate::store::Store;
use crate::text::fnv1a;
use crate::time::{DAY, Local};

const WINDOW_DAYS: i64 = 90;
/// Commit and file events stay out on purpose: they feed the MCP timeline, not the map.
const MAP_KINDS: [EventKind; 7] = [
    EventKind::Focus,
    EventKind::Idle,
    EventKind::Pause,
    EventKind::Visit,
    EventKind::Calendar,
    EventKind::Press,
    EventKind::Edit,
];

/// Row counts of one refresh, for logs. Never text.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RefreshStats {
    pub episodes: usize,
    pub steps: usize,
    pub entities: usize,
    pub tasks: usize,
}

struct Analysis {
    episodes: Vec<episodes::Episode>,
    steps: Vec<steps::Step>,
    entities: Vec<entities::Entity>,
    tasks: Vec<tasks::Task>,
}

fn analyze(events: &[EventRow], now: i64) -> Analysis {
    let mut episodes = episodes::build(events, now);
    let steps = steps::build(&mut episodes);
    let entities = entities::build(&episodes);
    let acts = episode_actions(events, &episodes);
    let tasks = tasks::build(&episodes, &entities, &acts);
    Analysis {
        episodes,
        steps,
        entities,
        tasks,
    }
}

/// The presses and edits of each episode, oldest first. Each belongs to the episode that spans its time.
fn episode_actions(past: &[EventRow], episodes: &[episodes::Episode]) -> Vec<Vec<actions::Act>> {
    let presses: Vec<&EventRow> = past
        .iter()
        .filter(|e| e.kind == "press" || e.kind == "edit")
        .collect();
    let spans: Vec<(i64, i64)> = episodes.iter().map(|e| (e.start, e.end)).collect();
    let times: Vec<i64> = presses.iter().map(|e| e.ts).collect();
    let mut out: Vec<Vec<actions::Act>> = (0..episodes.len()).map(|_| Vec::new()).collect();
    for (press, episode) in presses.iter().zip(actions::attach(&spans, &times)) {
        if let Some(episode) = episode {
            out[episode].push(actions::act(&press.payload));
        }
    }
    out
}

fn load_events(conn: &Connection, from: i64, to: i64) -> Result<Vec<EventRow>> {
    let kinds = MAP_KINDS
        .map(|kind| format!("'{}'", kind.as_str()))
        .join(", ");
    let mut statement = conn.prepare_cached(&format!(
        "SELECT ts, kind, source, json FROM events WHERE ts BETWEEN ?1 AND ?2 AND kind IN ({kinds}) ORDER BY ts, id"
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

/// Survives refreshes: an activity keeps its id while its start and object stay the same. A task
/// takes the id of its first activity.
fn activity_id(episode: &episodes::Episode) -> i64 {
    stable_id(&format!("{}|{}", episode.start, episode.key))
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
    for table in [
        "episodes",
        "steps",
        "entities",
        "mentions",
        "tasks",
        "task_episodes",
    ] {
        tx.execute(&format!("DELETE FROM {table}"), [])?;
    }
    {
        let activity_ids: Vec<i64> = analysis.episodes.iter().map(activity_id).collect();
        let mut insert = tx.prepare("INSERT INTO episodes(id, start_ts, end_ts, app, object_key, step_id, live) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)")?;
        for (e, &activity) in analysis.episodes.iter().zip(&activity_ids) {
            insert.execute(params![
                activity,
                e.start,
                e.end,
                e.app,
                e.key,
                id(e.step),
                e.live
            ])?;
        }
        let mut insert = tx.prepare("INSERT INTO steps(id, key, name) VALUES (?1, ?2, ?3)")?;
        for (i, s) in analysis.steps.iter().enumerate() {
            insert.execute(params![id(i), s.key, s.name])?;
        }
        let mut insert = tx.prepare(
            "INSERT INTO entities(id, kind, value, episode_count) VALUES (?1, ?2, ?3, ?4)",
        )?;
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
                mention.execute(params![id(i), activity_ids[*episode]])?;
            }
        }
        let mut insert = tx.prepare(
            "INSERT INTO tasks(id, label, first_ts, last_ts, active_s) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        let mut episode_link =
            tx.prepare("INSERT INTO task_episodes(task_id, episode_id) VALUES (?1, ?2)")?;
        for t in &analysis.tasks {
            let task_id = activity_ids[t.episodes[0]];
            insert.execute(params![task_id, t.label(), t.first, t.last, t.active_s])?;
            for &episode in &t.episodes {
                episode_link.execute(params![task_id, activity_ids[episode]])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

impl Store {
    /// Rebuilds episodes, steps, entities and tasks from the last 90 days. Only events up to
    /// `when.now` count.
    pub fn refresh(&mut self, when: Local) -> Result<RefreshStats> {
        let events = load_events(&self.conn, when.now - WINDOW_DAYS * DAY, when.now)?;
        let analysis = analyze(&events, when.now);
        persist(&mut self.conn, &analysis)?;
        Ok(RefreshStats {
            episodes: analysis.episodes.len(),
            steps: analysis.steps.len(),
            entities: analysis.entities.len(),
            tasks: analysis.tasks.len(),
        })
    }
}
