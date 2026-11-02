//! Pages (routines-plan.md E1): the week, a day and a task, compiled by code from the graph so an
//! agent can read it top down. The schema still calls activities `episodes` until E4.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::params;
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::settings::Settings;
use crate::store::Store;
use crate::time::{DAY, WEEK_DAYS, date, day, iso, weekday};

/// Values shown per run of the same action in `task`.
const MAX_VALUES: usize = 3;
/// Tasks per day in `week`.
const TOP_TASKS: usize = 3;
/// Repeated activity labels in `week`.
const TOP_REPEATS: usize = 25;
/// Folded labels outside tasks in `day`, by minutes. The rest is summed.
const TOP_OTHER: usize = 40;
/// A label must show up on this many days to count as repeated.
const MIN_DAYS: usize = 2;

/// Activities outside tasks with the same app and label, folded for `day`.
#[derive(Default)]
struct Fold {
    first: i64,
    times: usize,
    seconds: i64,
    actions: usize,
}

pub(crate) struct Activity {
    pub(crate) id: i64,
    pub(crate) start: i64,
    pub(crate) end: i64,
    pub(crate) app: String,
    pub(crate) label: String,
    meeting: bool,
    task: Option<(i64, String)>,
}

pub(crate) fn minutes(seconds: i64) -> f64 {
    (seconds as f64 / 6.0).round() / 10.0
}

/// `09:40` local.
fn clock(ts: i64, utc_offset_s: i32) -> String {
    iso(ts, utc_offset_s)[11..16].to_string()
}

/// Seconds covered by at least one activity, so a meeting and the screen during it count once.
fn active_s(activities: &[&Activity]) -> i64 {
    covered_s(activities.iter().map(|a| (a.start, a.end)).collect())
}

/// Seconds covered by at least one of `spans`.
pub(crate) fn covered_s(mut spans: Vec<(i64, i64)>) -> i64 {
    spans.sort_unstable();
    let mut total = 0;
    let mut reached = i64::MIN;
    for (start, end) in spans {
        let start = start.max(reached);
        if end > start {
            total += end - start;
            reached = end;
        }
    }
    total
}

pub(crate) fn midnight(day: i64, utc_offset_s: i32) -> i64 {
    day * DAY - i64::from(utc_offset_s)
}

impl Store {
    /// Visible activities that start in `[from, to)`, oldest first.
    pub(crate) fn activities(
        &self,
        settings: &Settings,
        from: i64,
        to: i64,
    ) -> Result<Vec<Activity>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT e.id, e.start_ts, e.end_ts, e.app, e.object_key, s.key, s.name, t.id, t.label
             FROM episodes e JOIN steps s ON s.id = e.step_id
             LEFT JOIN task_episodes te ON te.episode_id = e.id
             LEFT JOIN tasks t ON t.id = te.task_id
             WHERE e.start_ts >= ?1 AND e.start_ts < ?2 ORDER BY e.start_ts, e.id",
        )?;
        let rows = statement.query_map(params![from, to], |row| {
            let object: String = row.get(4)?;
            let step_key: String = row.get(5)?;
            let task = match row.get::<_, Option<i64>>(7)? {
                Some(id) => Some((id, row.get(8)?)),
                None => None,
            };
            Ok((
                step_key,
                Activity {
                    id: row.get(0)?,
                    start: row.get(1)?,
                    end: row.get(2)?,
                    app: row.get(3)?,
                    label: row.get(6)?,
                    meeting: object.starts_with("meeting|"),
                    task,
                },
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (step_key, activity) = row?;
            let host_or_app = step_key.split('|').next().unwrap_or("");
            if !settings.is_excluded(&activity.app) && !settings.is_excluded(host_or_app) {
                out.push(activity);
            }
        }
        Ok(out)
    }

    /// Press and edit times in `[from, to)`, sorted.
    fn action_times(&self, from: i64, to: i64) -> Result<Vec<i64>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT ts FROM events WHERE ts >= ?1 AND ts < ?2 AND kind IN ('press', 'edit') ORDER BY ts",
        )?;
        let times = statement
            .query_map(params![from, to], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(times)
    }

    /// The day that holds `at`: the day's tasks by minutes, their activities in time order, and
    /// every other activity folded by app and label with its first time.
    pub fn day_page(&self, settings: &Settings, at: i64, utc_offset_s: i32) -> Result<Value> {
        let today = day(at, utc_offset_s);
        let (from, to) = (
            midnight(today, utc_offset_s),
            midnight(today + 1, utc_offset_s),
        );
        let activities = self.activities(settings, from, to)?;
        let times = self.action_times(from, to + DAY)?;
        let all: Vec<&Activity> = activities.iter().collect();

        let mut tasks: BTreeMap<i64, (String, i64, usize)> = BTreeMap::new();
        for activity in &activities {
            if let Some((id, label)) = &activity.task {
                let entry = tasks.entry(*id).or_insert((label.clone(), 0, 0));
                entry.1 += activity.end - activity.start;
                entry.2 += 1;
            }
        }
        let mut tasks: Vec<_> = tasks.into_iter().collect();
        tasks.sort_by_key(|(id, (_, seconds, _))| (-seconds, *id));

        let actions = |a: &Activity| {
            if a.meeting {
                return 0;
            }
            times.partition_point(|&t| t < a.end) - times.partition_point(|&t| t < a.start)
        };
        let (in_tasks, other): (Vec<&Activity>, Vec<&Activity>) =
            activities.iter().partition(|a| a.task.is_some());
        let rows: Vec<Value> = in_tasks
            .iter()
            .map(|a| {
                let mut row = json!({
                    "id": a.id,
                    "time": clock(a.start, utc_offset_s),
                    "minutes": minutes(a.end - a.start),
                    "app": a.app,
                    "label": a.label,
                });
                if let Some((id, _)) = a.task {
                    row["task"] = json!(id);
                }
                let count = actions(a);
                if count > 0 {
                    row["actions"] = json!(count);
                }
                row
            })
            .collect();

        let mut folds: BTreeMap<(&str, &str), Fold> = BTreeMap::new();
        for a in &other {
            let fold = folds.entry((&a.app, &a.label)).or_insert(Fold {
                first: a.start,
                ..Fold::default()
            });
            fold.times += 1;
            fold.seconds += a.end - a.start;
            fold.actions += actions(a);
        }
        let mut folds: Vec<_> = folds.into_iter().collect();
        folds.sort_by_key(|(key, fold)| (-fold.seconds, *key));
        let rest = folds.split_off(folds.len().min(TOP_OTHER));
        folds.sort_by_key(|(key, fold)| (fold.first, *key));
        let mut page = json!({
            "date": date(today),
            "weekday": weekday(today),
            "active_minutes": minutes(active_s(&all)),
            "tasks": tasks.iter().map(|(id, (label, seconds, count))| json!({
                "id": id, "label": label, "minutes": minutes(*seconds), "activities": count,
            })).collect::<Vec<_>>(),
            "activities": rows,
            "other": folds.iter().map(|((app, label), fold)| {
                let mut row = json!({
                    "app": app,
                    "label": label,
                    "first": clock(fold.first, utc_offset_s),
                    "times": fold.times,
                    "minutes": minutes(fold.seconds),
                });
                if fold.actions > 0 {
                    row["actions"] = json!(fold.actions);
                }
                row
            }).collect::<Vec<_>>(),
        });
        if !rest.is_empty() {
            page["other_rest"] = json!({
                "labels": rest.len(),
                "times": rest.iter().map(|(_, f)| f.times).sum::<usize>(),
                "minutes": minutes(rest.iter().map(|(_, f)| f.seconds).sum()),
            });
        }
        Ok(page)
    }

    /// Seven days from the day that holds `start`: minutes and top tasks per day, and the task
    /// kinds and activity labels that repeat on 2 or more days.
    pub fn week_page(&self, settings: &Settings, start: i64, utc_offset_s: i32) -> Result<Value> {
        let first = day(start, utc_offset_s);
        let from = midnight(first, utc_offset_s);
        let activities =
            self.activities(settings, from, midnight(first + WEEK_DAYS, utc_offset_s))?;

        let mut by_day: BTreeMap<i64, Vec<&Activity>> = BTreeMap::new();
        for activity in &activities {
            by_day
                .entry(day(activity.start, utc_offset_s))
                .or_default()
                .push(activity);
        }
        let days: Vec<Value> = (first..first + WEEK_DAYS)
            .map(|d| {
                let list = by_day.get(&d).map(Vec::as_slice).unwrap_or(&[]);
                let mut tasks: BTreeMap<i64, (&str, i64)> = BTreeMap::new();
                for a in list {
                    if let Some((id, label)) = &a.task {
                        tasks.entry(*id).or_insert((label, 0)).1 += a.end - a.start;
                    }
                }
                let mut tasks: Vec<_> = tasks.into_iter().collect();
                tasks.sort_by_key(|(id, (_, seconds))| (-seconds, *id));
                json!({
                    "date": date(d),
                    "weekday": weekday(d),
                    "active_minutes": minutes(active_s(list)),
                    "top_tasks": tasks.iter().take(TOP_TASKS).map(|(id, (label, seconds))| json!({
                        "id": id, "label": label, "minutes": minutes(*seconds),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();

        Ok(json!({
            "from": date(first),
            "to": date(first + WEEK_DAYS - 1),
            "active_minutes": minutes(active_s(&activities.iter().collect::<Vec<_>>())),
            "days": days,
            "repeated_tasks": repeated_tasks(&activities, utc_offset_s),
            "repeated_activities": repeated_activities(&activities, utc_offset_s),
        }))
    }

    /// The task's label, first and last time, and its visible activities in time order.
    pub(crate) fn task_activities(
        &self,
        settings: &Settings,
        id: i64,
    ) -> Result<(String, i64, i64, Vec<Activity>)> {
        let no_task = || Error::Invalid(format!("no task {id}: use an id from day or week"));
        let (label, first, last) = self
            .conn
            .query_row(
                "SELECT label, first_ts, last_ts FROM tasks WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => no_task(),
                err => err.into(),
            })?;
        let activities: Vec<Activity> = self
            .activities(settings, first, last + 1)?
            .into_iter()
            .filter(|a| a.task.as_ref().is_some_and(|(task, _)| *task == id))
            .collect();
        if activities.is_empty() {
            return Err(no_task());
        }
        Ok((label, first, last, activities))
    }

    /// One task: its activities and its actions, consecutive repeats folded. Values show only
    /// when `settings.mcp_rewind` lets screen text out.
    pub fn task_page(&self, settings: &Settings, id: i64, utc_offset_s: i32) -> Result<Value> {
        let (label, first, last, activities) = self.task_activities(settings, id)?;
        let visible =
            |symbol: &str| !settings.is_excluded(symbol.split(" · ").next().unwrap_or(""));

        let mut runs: Vec<(String, Option<&'static str>, usize, BTreeSet<String>)> = Vec::new();
        for action in self
            .task_actions(id)?
            .into_iter()
            .filter(|a| visible(&a.symbol))
        {
            let kind = action.value_type.map(|v| v.as_str());
            let same = runs
                .last()
                .is_some_and(|(symbol, value, ..)| *symbol == action.symbol && *value == kind);
            if !same {
                runs.push((action.symbol, kind, 0, BTreeSet::new()));
            }
            if let Some(run) = runs.last_mut() {
                run.2 += 1;
                if let Some(value) = action
                    .value
                    .filter(|_| settings.mcp_rewind && run.3.len() < MAX_VALUES)
                {
                    run.3.insert(value);
                }
            }
        }

        let days: BTreeSet<i64> = activities
            .iter()
            .map(|a| day(a.start, utc_offset_s))
            .collect();
        let all: Vec<&Activity> = activities.iter().collect();
        Ok(json!({
            "id": id,
            "label": label,
            "first": iso(first, utc_offset_s),
            "last": iso(last, utc_offset_s),
            "active_minutes": minutes(active_s(&all)),
            "days": days.len(),
            "activities": activities.iter().map(|a| json!({
                "id": a.id,
                "start": iso(a.start, utc_offset_s),
                "minutes": minutes(a.end - a.start),
                "app": a.app,
                "label": a.label,
            })).collect::<Vec<_>>(),
            "actions": runs.iter().map(|(symbol, kind, times, values)| {
                let mut row = json!({"action": symbol});
                if *times > 1 {
                    row["times"] = json!(times);
                }
                if let Some(kind) = kind {
                    row["value"] = json!(kind);
                }
                if !values.is_empty() {
                    row["values"] = json!(values);
                }
                row
            }).collect::<Vec<_>>(),
        }))
    }
}

/// Task labels (numbers already templated, so "inv #") held by 2 or more tasks on 2 or more days.
fn repeated_tasks(activities: &[Activity], utc_offset_s: i32) -> Vec<Value> {
    let mut kinds: BTreeMap<&str, (BTreeSet<i64>, BTreeSet<i64>, i64)> = BTreeMap::new();
    for a in activities {
        if let Some((id, label)) = &a.task {
            let kind = kinds.entry(label).or_default();
            kind.0.insert(*id);
            kind.1.insert(day(a.start, utc_offset_s));
            kind.2 += a.end - a.start;
        }
    }
    let mut kinds: Vec<_> = kinds
        .into_iter()
        .filter(|(_, (tasks, days, _))| tasks.len() >= 2 && days.len() >= MIN_DAYS)
        .collect();
    kinds.sort_by_key(|(label, (_, _, seconds))| (-seconds, *label));
    kinds
        .iter()
        .map(|(label, (tasks, days, seconds))| {
            json!({"label": label, "tasks": tasks.len(), "days": days.len(),
                   "minutes": minutes(*seconds), "task_ids": tasks})
        })
        .collect()
}

/// Activity labels seen on 2 or more days, most minutes first.
fn repeated_activities(activities: &[Activity], utc_offset_s: i32) -> Vec<Value> {
    let mut labels: BTreeMap<(&str, &str), (BTreeSet<i64>, usize, i64)> = BTreeMap::new();
    for a in activities {
        let label = labels.entry((&a.app, &a.label)).or_default();
        label.0.insert(day(a.start, utc_offset_s));
        label.1 += 1;
        label.2 += a.end - a.start;
    }
    let mut labels: Vec<_> = labels
        .into_iter()
        .filter(|(_, (days, _, _))| days.len() >= MIN_DAYS)
        .collect();
    labels.sort_by_key(|(key, (_, _, seconds))| (-seconds, *key));
    labels
        .iter()
        .take(TOP_REPEATS)
        .map(|((app, label), (days, times, seconds))| {
            json!({"app": app, "label": label, "days": days.len(), "times": times,
                   "minutes": minutes(*seconds)})
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::Value;

    use crate::fixtures::{TempDb, UTC_OFFSET_S, WEEK_NOW, WEEK_START, actions_week, load, local};
    use crate::settings::Settings;
    use crate::store::Store;
    use crate::time::{DAY, day};

    /// The interleaved invoice week, refreshed, and the invoice id of each planned visit by start.
    fn invoice_week(label: &str) -> (TempDb, Store, Vec<(i64, String)>) {
        let (fixture, truth) = actions_week();
        let db = TempDb::new(label);
        let mut store = db.open().expect("test setup");
        load(&mut store, &fixture, WEEK_NOW).expect("test setup");
        store.refresh(local(WEEK_NOW)).expect("test setup");
        (db, store, truth)
    }

    fn week(store: &Store, settings: &Settings) -> Value {
        store
            .week_page(settings, WEEK_START, UTC_OFFSET_S)
            .expect("week")
    }

    fn kind<'a>(week: &'a Value, label: &str) -> &'a Value {
        week["repeated_tasks"]
            .as_array()
            .expect("kinds")
            .iter()
            .find(|k| k["label"] == label)
            .unwrap_or_else(|| panic!("no {label} in {week}"))
    }

    #[test]
    fn the_week_lists_each_invoice_kind_with_its_tasks_and_days() {
        let (_db, store, truth) = invoice_week("pages-week");
        let page = week(&store, &Settings::default());
        for (prefix, label) in [("INV-", "inv #"), ("RE-", "re #")] {
            let planned: Vec<&(i64, String)> = truth
                .iter()
                .filter(|(_, id)| id.starts_with(prefix))
                .collect();
            let invoices: BTreeSet<&str> = planned.iter().map(|(_, id)| id.as_str()).collect();
            let days: BTreeSet<i64> = planned
                .iter()
                .map(|(start, _)| day(*start, UTC_OFFSET_S))
                .collect();
            let found = kind(&page, label);
            assert_eq!(found["tasks"], invoices.len(), "{found}");
            assert_eq!(found["days"], days.len(), "{found}");
        }
    }

    #[test]
    fn a_day_lists_every_activity_in_order_and_its_task_ids_resolve() {
        let (_db, store, _) = invoice_week("pages-day");
        let settings = Settings::default();
        let page = store
            .day_page(&settings, WEEK_START + DAY, UTC_OFFSET_S)
            .expect("day");
        let started: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM episodes WHERE start_ts >= ?1 AND start_ts < ?1 + 86400",
                [WEEK_START + DAY],
                |row| row.get(0),
            )
            .expect("count");
        let activities = page["activities"].as_array().expect("activities");
        let folded: i64 = page["other"]
            .as_array()
            .expect("other")
            .iter()
            .map(|fold| fold["times"].as_i64().expect("times"))
            .sum();
        assert!(activities.iter().all(|a| a["task"].is_i64()));
        assert_eq!(activities.len() as i64 + folded, started);
        let times: Vec<&str> = activities
            .iter()
            .map(|a| a["time"].as_str().expect("time"))
            .collect();
        assert!(times.is_sorted(), "{times:?}");

        for listed in page["tasks"].as_array().expect("tasks") {
            let id = listed["id"].as_i64().expect("id");
            let task = store.task_page(&settings, id, UTC_OFFSET_S).expect("task");
            let in_task: BTreeSet<i64> = task["activities"]
                .as_array()
                .expect("activities")
                .iter()
                .map(|a| a["id"].as_i64().expect("id"))
                .collect();
            for activity in activities.iter().filter(|a| a["task"] == id) {
                assert!(in_task.contains(&activity["id"].as_i64().expect("id")));
            }
        }
        assert!(store.task_page(&settings, 1, UTC_OFFSET_S).is_err());
    }

    #[test]
    fn a_task_folds_repeated_actions_and_shows_values_only_with_screen_text_on() {
        let (_db, store, _) = invoice_week("pages-task");
        let mut settings = Settings::default();
        let id = kind(&week(&store, &settings), "inv #")["task_ids"][0]
            .as_i64()
            .expect("id");
        let task = store.task_page(&settings, id, UTC_OFFSET_S).expect("task");
        let text = task.to_string();
        assert!(
            text.contains("\"action\":\"Mail · AXButton · send\""),
            "{text}"
        );
        assert!(text.contains("€1,250.00"), "{text}");

        settings.mcp_rewind = false;
        let task = store.task_page(&settings, id, UTC_OFFSET_S).expect("task");
        let text = task.to_string();
        assert!(!text.contains("values") && !text.contains("€"), "{text}");
        assert!(text.contains("\"value\":\"amount\""), "{text}");
    }

    #[test]
    fn excluded_apps_leave_every_page() {
        let (_db, store, _) = invoice_week("pages-excluded");
        let settings = Settings {
            excluded_apps: vec!["mail".into()],
            ..Settings::default()
        };
        let page = week(&store, &settings);
        let id = kind(&page, "inv #")["task_ids"][0].as_i64().expect("id");
        let pages = [
            page.to_string(),
            store
                .day_page(&settings, WEEK_START, UTC_OFFSET_S)
                .expect("day")
                .to_string(),
            store
                .task_page(&settings, id, UTC_OFFSET_S)
                .expect("task")
                .to_string(),
        ];
        for text in pages {
            assert!(!text.contains("Mail"), "{text}");
        }
    }
}
