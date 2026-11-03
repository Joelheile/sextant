//! Tasks (routines-plan.md T2): activities joined in time order by shared anchors, where time goes.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use rusqlite::params;

use super::actions::{Act, act, identifier};
use super::activities::Activity;
use super::entities::{Entity, EntityKind, ValueType};
use super::{find, stable_id};
use crate::error::Result;
use crate::model::Payload;
use crate::store::Store;
use crate::text::template;
use crate::time::{DAY, date, day};

const JOIN_WINDOW_S: i64 = 7 * DAY;
const SAME_OBJECT_S: i64 = 120;
const MERGE_GAP_S: i64 = 14 * DAY;

/// Activities joined by shared anchors. Anchors are invoice and ticket ids, and edit values that are ids.
pub(crate) struct Task {
    pub anchors: BTreeSet<String>,
    /// Activity indexes in time order.
    pub activities: Vec<usize>,
    pub first: i64,
    pub last: i64,
    pub active_s: i64,
}

impl Task {
    /// The first anchor in sorted order with its numbers templated: "INV-2026-0412" becomes "inv #".
    pub fn label(&self) -> String {
        template(self.anchors.iter().next().map_or("", String::as_str))
    }

    fn absorb(&mut self, other: Task) {
        self.anchors.extend(other.anchors);
        self.activities.extend(other.activities);
        self.first = self.first.min(other.first);
        self.last = self.last.max(other.last);
        self.active_s += other.active_s;
    }
}

/// Joins activities in time order (routines-plan.md T2). An activity with an anchor joins the open task
/// it shares the most anchors with, seen in the last 7 days. An activity with no anchor joins only the
/// task of an activity in the same object that ended within 2 minutes. Otherwise it stays unassigned.
/// A final pass merges tasks that share an anchor when they are at most 14 days apart.
pub(crate) fn build(activities: &[Activity], entities: &[Entity], acts: &[Vec<Act>]) -> Vec<Task> {
    let mut anchors: Vec<BTreeSet<String>> = vec![BTreeSet::new(); activities.len()];
    for entity in entities
        .iter()
        .filter(|e| matches!(e.kind, EntityKind::Invoice | EntityKind::Ticket))
    {
        for &activity in &entity.activities {
            anchors[activity].insert(entity.value.clone());
        }
    }
    for (index, list) in acts.iter().enumerate() {
        for action in list {
            if let (Some(ValueType::Id), Some(value)) = (action.value_type, &action.value) {
                anchors[index].insert(value.to_uppercase());
            }
        }
    }

    let mut open: Vec<Task> = Vec::new();
    let mut of: Vec<Option<usize>> = vec![None; activities.len()];
    for (index, activity) in activities.iter().enumerate() {
        let joined = if anchors[index].is_empty() {
            (0..index)
                .rev()
                .take_while(|&j| activity.start - activities[j].end <= SAME_OBJECT_S)
                .find(|&j| activities[j].key == activity.key)
                .and_then(|j| of[j])
        } else {
            open.iter()
                .enumerate()
                .filter(|(_, task)| task.last >= activity.start - JOIN_WINDOW_S)
                .map(|(t, task)| {
                    let shared = anchors[index].intersection(&task.anchors).count();
                    (shared, task.last, t)
                })
                .filter(|(shared, _, _)| *shared > 0)
                .max()
                .map(|(_, _, t)| t)
        };
        let task = match (joined, anchors[index].is_empty()) {
            (Some(task), _) => task,
            (None, true) => continue,
            (None, false) => {
                open.push(Task {
                    anchors: BTreeSet::new(),
                    activities: Vec::new(),
                    first: activity.start,
                    last: activity.start,
                    active_s: 0,
                });
                open.len() - 1
            }
        };
        let target = &mut open[task];
        target.anchors.extend(anchors[index].iter().cloned());
        target.activities.push(index);
        target.last = target.last.max(activity.end);
        target.active_s += activity.end - activity.start;
        of[index] = Some(task);
    }

    let mut parent: Vec<usize> = (0..open.len()).collect();
    let mut by_anchor: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (t, task) in open.iter().enumerate() {
        for anchor in &task.anchors {
            by_anchor.entry(anchor.as_str()).or_default().push(t);
        }
    }
    for list in by_anchor.values_mut() {
        list.sort_by_key(|&t| open[t].first);
        for pair in list.windows(2) {
            if open[pair[1]].first - open[pair[0]].last <= MERGE_GAP_S {
                let (a, b) = (find(&mut parent, pair[0]), find(&mut parent, pair[1]));
                parent[b] = a;
            }
        }
    }

    let mut merged: BTreeMap<usize, Task> = BTreeMap::new();
    for (t, task) in open.into_iter().enumerate() {
        match merged.entry(find(&mut parent, t)) {
            Entry::Vacant(slot) => {
                slot.insert(task);
            }
            Entry::Occupied(mut slot) => slot.get_mut().absorb(task),
        }
    }
    let mut tasks: Vec<Task> = merged.into_values().collect();
    tasks.sort_by_key(|task| task.first);
    for task in &mut tasks {
        task.activities.sort_unstable();
    }
    tasks
}

/// A task as the app lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSummary {
    pub id: i64,
    pub label: String,
    pub first_ts: i64,
    pub last_ts: i64,
    pub active_s: i64,
}

/// One press or edit inside a task, oldest first. The value is read from `events` at query time,
/// since the task tables hold no values.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskAction {
    pub ts: i64,
    /// Stable for the symbol, and fits a JS number.
    pub symbol_id: i64,
    /// App, role and label with numbers replaced, such as "Mail · AXButton · send".
    pub symbol: String,
    pub value_type: Option<ValueType>,
    /// The stored value, redacted at write time. None for a press.
    pub value: Option<String>,
    /// The value when it is an identifier: type id, email or amount.
    pub identifiers: Vec<String>,
}

/// Active minutes of one task on one day, where the time went.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskMinutes {
    pub task_id: i64,
    pub label: String,
    /// `YYYY-MM-DD`, in the local offset. An activity counts on the day it starts.
    pub date: String,
    pub minutes: f64,
}

impl Store {
    pub fn tasks(&self) -> Result<Vec<TaskSummary>> {
        let mut statement = self.conn.prepare(
            "SELECT id, label, first_ts, last_ts, active_s FROM tasks ORDER BY first_ts, id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(TaskSummary {
                id: row.get(0)?,
                label: row.get(1)?,
                first_ts: row.get(2)?,
                last_ts: row.get(3)?,
                active_s: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn task_actions(&self, task_id: i64) -> Result<Vec<TaskAction>> {
        let mut statement = self.conn.prepare(
            "SELECT ev.ts, ev.json FROM task_activities te
             JOIN activities e ON e.id = te.activity_id
             JOIN events ev ON ev.ts BETWEEN e.start_ts AND e.end_ts AND ev.kind IN ('press', 'edit')
             WHERE te.task_id = ?1 ORDER BY ev.ts, ev.id",
        )?;
        let rows = statement.query_map(params![task_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (ts, json) = row?;
            let payload: Payload = serde_json::from_str(&json).unwrap_or_default();
            let action = act(&payload);
            let identifiers = identifier(&action).map(str::to_owned).into_iter().collect();
            out.push(TaskAction {
                ts,
                symbol_id: stable_id(&action.symbol),
                symbol: action.symbol,
                value_type: action.value_type,
                value: action.value,
                identifiers,
            });
        }
        Ok(out)
    }

    /// Active minutes per task and day. Each activity counts once, on the day it starts.
    pub fn where_time_goes(&self, utc_offset_s: i32) -> Result<Vec<TaskMinutes>> {
        let mut statement = self.conn.prepare(
            "SELECT te.task_id, t.label, e.start_ts, e.end_ts FROM task_activities te
             JOIN tasks t ON t.id = te.task_id
             JOIN activities e ON e.id = te.activity_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        let mut per_day: BTreeMap<(i64, i64), (String, f64)> = BTreeMap::new();
        for row in rows {
            let (task_id, label, start, end) = row?;
            let (_, minutes) = per_day
                .entry((day(start, utc_offset_s), task_id))
                .or_insert((label, 0.0));
            *minutes += (end - start) as f64 / 60.0;
        }
        Ok(per_day
            .into_iter()
            .map(|((days, task_id), (label, minutes))| TaskMinutes {
                task_id,
                label,
                date: date(days),
                minutes,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::fixtures::{TempDb, WEEK_NOW, WEEK_START, actions_week, at, load, local};
    use crate::time::DAY;

    /// Adjusted Rand index of two labelings of the same items: 1.0 when they match, about 0 by chance.
    fn adjusted_rand(truth: &[usize], found: &[usize]) -> f64 {
        let pairs = |n: usize| (n * n.saturating_sub(1)) as f64 / 2.0;
        let mut cells: HashMap<(usize, usize), usize> = HashMap::new();
        let mut rows: HashMap<usize, usize> = HashMap::new();
        let mut cols: HashMap<usize, usize> = HashMap::new();
        for (&t, &f) in truth.iter().zip(found) {
            *cells.entry((t, f)).or_default() += 1;
            *rows.entry(t).or_default() += 1;
            *cols.entry(f).or_default() += 1;
        }
        let index: f64 = cells.values().map(|&n| pairs(n)).sum();
        let row: f64 = rows.values().map(|&n| pairs(n)).sum();
        let col: f64 = cols.values().map(|&n| pairs(n)).sum();
        let expected = row * col / pairs(truth.len());
        let max = (row + col) / 2.0;
        if (max - expected).abs() < f64::EPSILON {
            return 1.0;
        }
        (index - expected) / (max - expected)
    }

    /// Numbers each distinct label in order of first use.
    fn intern(labels: impl Iterator<Item = String>) -> Vec<usize> {
        let mut ids: HashMap<String, usize> = HashMap::new();
        labels
            .map(|label| {
                let next = ids.len();
                *ids.entry(label).or_insert(next)
            })
            .collect()
    }

    #[test]
    fn adjusted_rand_is_one_for_the_same_grouping_under_new_names() {
        assert!((adjusted_rand(&[0, 0, 1, 1], &[5, 5, 9, 9]) - 1.0).abs() < 1e-9);
        assert!(adjusted_rand(&[0, 1, 0, 1], &[0, 0, 1, 1]) < 0.1);
    }

    fn ids_by_start(store: &crate::store::Store, sql: &str) -> HashMap<i64, i64> {
        let mut statement = store.conn.prepare(sql).expect("test setup");
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("test setup")
            .collect::<std::result::Result<_, _>>()
            .expect("test setup")
    }

    #[test]
    fn activity_and_task_ids_survive_a_refresh_whose_window_moved() {
        let (fixture, _) = actions_week();
        let db = TempDb::new("stable-ids");
        let mut store = db.open().expect("test setup");
        load(&mut store, &fixture, WEEK_NOW).expect("test setup");
        let activities = "SELECT start_ts, id FROM activities";
        let tasks = "SELECT first_ts, id FROM tasks";

        store.refresh(local(WEEK_NOW)).expect("test setup");
        let (activities_before, tasks_before) = (
            ids_by_start(&store, activities),
            ids_by_start(&store, tasks),
        );
        // 90 days after Tuesday noon, so Monday and Tuesday morning fall out of the window.
        store
            .refresh(local(WEEK_START + DAY + 12 * 3600 + 90 * DAY))
            .expect("test setup");
        let (activities_after, tasks_after) = (
            ids_by_start(&store, activities),
            ids_by_start(&store, tasks),
        );

        assert!(activities_after.len() < activities_before.len());
        let kept: Vec<_> = activities_after
            .iter()
            .filter(|(start, _)| activities_before.contains_key(start))
            .collect();
        assert!(!kept.is_empty());
        for (start, id) in kept {
            assert_eq!(activities_before[start], *id, "activity at {start}");
        }
        let kept_tasks: Vec<_> = tasks_after
            .iter()
            .filter(|(first, _)| tasks_before.contains_key(first))
            .collect();
        assert!(!kept_tasks.is_empty());
        for (first, id) in kept_tasks {
            assert_eq!(tasks_before[first], *id, "task starting at {first}");
        }
    }

    #[test]
    fn tasks_agree_with_the_interleaved_truth_at_least_0_9() {
        let (fixture, truth) = actions_week();
        let db = TempDb::new("tasks-pairs");
        let mut store = db.open().expect("test setup");
        load(&mut store, &fixture, WEEK_NOW).expect("test setup");
        store.refresh(local(WEEK_NOW)).expect("test setup");

        let planned: HashMap<i64, String> = truth.into_iter().collect();
        let rows: Vec<(i64, Option<i64>)> = {
            let mut statement = store
                .conn
                .prepare(
                    "SELECT e.start_ts, te.task_id FROM activities e
                     LEFT JOIN task_activities te ON te.activity_id = e.id ORDER BY e.id",
                )
                .expect("test setup");
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .expect("test setup")
                .collect::<std::result::Result<_, _>>()
                .expect("test setup")
        };

        // Every activity counts, so an activity that joins the wrong task is a disagreement.
        // Activities nobody planned are noise: each is its own group on both sides.
        let expected = intern(rows.iter().enumerate().map(|(index, (start, _))| {
            planned
                .get(start)
                .cloned()
                .unwrap_or_else(|| format!("noise {index}"))
        }));
        let found = intern(rows.iter().enumerate().map(|(index, (_, task))| {
            task.map_or_else(|| format!("alone {index}"), |task| format!("task {task}"))
        }));
        let score = adjusted_rand(&expected, &found);
        assert!(
            score >= 0.9,
            "pair agreement {score:.3} over {} activities",
            rows.len()
        );

        let lunch = at(WEEK_START + DAY, 15, 20);
        let lunch_task = rows
            .iter()
            .find(|(start, _)| *start == lunch)
            .and_then(|(_, task)| *task);
        assert_eq!(lunch_task, None, "the unrelated chat stays unassigned");
    }
}
