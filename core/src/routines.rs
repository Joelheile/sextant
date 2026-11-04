//! Routines: your agent proposes a routine and cites the tasks and repeated
//! activities it read. Code checks every citation, measures the hours and stores the time spans.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use regex::Regex;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::pages::{covered_s, midnight, minutes};
use crate::redact::redact;
use crate::settings::Settings;
use crate::store::Store;
use crate::text::{pattern, round2};
use crate::time::{day, iso, parse};

const MIN_ACTIVITIES: usize = 3;
const MIN_DAYS: usize = 2;
const TITLE_CHARS: usize = 120;
const TEXT_CHARS: usize = 1000;
/// Hours a week count work days, as the finder's do.
const WORK_DAYS: f64 = 5.0;

/// A numbered step in a `SKILL.md`, such as "1. Open the sheet".
static STEP: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\s*\d+[.)]\s"));
/// A step's citation, such as "(task 42: Mail · AXButton · send)".
static CITATION: LazyLock<Regex> = LazyLock::new(|| pattern(r"\(task (-?\d+): ([^()]+)\)"));

/// Work outside tasks, cited the way `day` folds it: an app, a label and the dates it ran.
#[derive(Debug, Deserialize, Serialize)]
pub struct Repeat {
    pub app: String,
    pub label: String,
    pub dates: Vec<String>,
}

/// What the agent proposes. It gives no hours: code measures them from the citations.
#[derive(Debug, Deserialize)]
pub struct Routine {
    pub title: String,
    #[serde(default)]
    pub task_ids: Vec<i64>,
    #[serde(default)]
    pub repeats: Vec<Repeat>,
    pub how: String,
    #[serde(default)]
    pub trigger: String,
    #[serde(default)]
    pub approval: String,
    pub why: String,
}

impl Store {
    /// Checks and stores `routine` on a short-lived write connection, so a read-only reader can
    /// save. Returns its id and the measured hours. An error says what to fix.
    pub fn save_routine(
        &self,
        settings: &Settings,
        routine: &Routine,
        now: i64,
        utc_offset_s: i32,
    ) -> Result<Value> {
        if routine.title.trim().is_empty() {
            return Err(Error::Invalid("title is required".into()));
        }
        let spans: Vec<(i64, i64)> = self
            .cited(settings, routine, utc_offset_s)?
            .into_values()
            .collect();
        let days: BTreeSet<i64> = spans
            .iter()
            .map(|(start, _)| day(*start, utc_offset_s))
            .collect();
        if spans.len() < MIN_ACTIVITIES || days.len() < MIN_DAYS {
            return Err(Error::Invalid(format!(
                "a routine needs {MIN_ACTIVITIES} or more activities on {MIN_DAYS} or more days, and these citations hold {} on {}: cite more tasks or repeats",
                spans.len(),
                days.len()
            )));
        }
        let active = covered_s(spans.clone());
        if let Some((id, title)) = self.overlapping(&spans, active)? {
            return Err(Error::Invalid(format!(
                "routine {id} \"{title}\" already holds more than half of this time: cite other work"
            )));
        }
        let hours_week = round2(active as f64 / 3600.0 / self.weeks_of_data(utc_offset_s)?);

        let clean = |text: &str, chars: usize| {
            redact(
                &text.trim().chars().take(chars).collect::<String>(),
                &self.secret,
            )
        };
        let cites = json!({"task_ids": routine.task_ids, "repeats": routine.repeats});
        let mut conn = self.write_conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO routines(ts, title, how, trigger, approval, why, cites, hours_week)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                now,
                clean(&routine.title, TITLE_CHARS),
                clean(&routine.how, TEXT_CHARS),
                clean(&routine.trigger, TEXT_CHARS),
                clean(&routine.approval, TEXT_CHARS),
                clean(&routine.why, TEXT_CHARS),
                redact(&cites.to_string(), &self.secret),
                hours_week
            ],
        )?;
        let id = tx.last_insert_rowid();
        {
            let mut insert = tx.prepare(
                "INSERT INTO routine_spans(routine_id, start_ts, end_ts) VALUES (?1, ?2, ?3)",
            )?;
            for (start, end) in &spans {
                insert.execute(params![id, start, end])?;
            }
        }
        tx.commit()?;
        Ok(json!({
            "id": id,
            "hours_week": hours_week,
            "active_minutes": minutes(active),
            "activities": spans.len(),
            "days": days.len(),
        }))
    }

    /// Saved routines, most hours first, with what they cite and the time their evidence spans.
    pub fn routines(&self, utc_offset_s: i32) -> Result<Value> {
        let mut statement = self.conn.prepare(
            "SELECT r.id, r.ts, r.title, r.how, r.trigger, r.approval, r.why, r.cites, r.hours_week,
                    COUNT(s.routine_id), MIN(s.start_ts), MAX(s.end_ts)
             FROM routines r LEFT JOIN routine_spans s ON s.routine_id = r.id
             GROUP BY r.id ORDER BY r.hours_week DESC, r.id",
        )?;
        let mut rows = statement.query([])?;
        let mut routines = Vec::new();
        while let Some(row) = rows.next()? {
            let at = |ts: Option<i64>| ts.map(|ts| iso(ts, utc_offset_s));
            routines.push(json!({
                "id": row.get::<_, i64>(0)?,
                "saved": iso(row.get(1)?, utc_offset_s),
                "title": row.get::<_, String>(2)?,
                "how": row.get::<_, String>(3)?,
                "trigger": row.get::<_, String>(4)?,
                "approval": row.get::<_, String>(5)?,
                "why": row.get::<_, String>(6)?,
                "cites": serde_json::from_str::<Value>(&row.get::<_, String>(7)?)?,
                "hours_week": row.get::<_, f64>(8)?,
                "activities": row.get::<_, i64>(9)?,
                "first": at(row.get(10)?),
                "last": at(row.get(11)?),
            }));
        }
        Ok(json!({"routines": routines}))
    }

    /// Checks a draft `SKILL.md` for routine `id`: every numbered step cites a task the routine
    /// cites and an action that task holds, as `(task <id>: <action>)`. Errors name every
    /// problem, so the agent can fix them in one pass.
    pub fn check_skill(&self, settings: &Settings, id: i64, skill: &str) -> Result<Value> {
        let tasks = self.routine_tasks(id)?;
        if tasks.is_empty() {
            return Err(Error::Invalid(format!(
                "routine {id} cites no tasks, so its steps have nothing to cite"
            )));
        }
        let mut actions: BTreeMap<i64, BTreeSet<String>> = BTreeMap::new();
        for &task in &tasks {
            let visible = self
                .task_actions(task)?
                .into_iter()
                .map(|a| a.symbol)
                .filter(|symbol| !settings.is_excluded(symbol.split(" · ").next().unwrap_or("")));
            actions.insert(task, visible.collect());
        }

        let steps: Vec<&str> = skill.lines().filter(|line| STEP.is_match(line)).collect();
        if steps.is_empty() {
            return Err(Error::Invalid(
                "the skill has no numbered steps: write them as 1. 2. 3.".into(),
            ));
        }
        let mut problems = Vec::new();
        let mut citations = 0;
        for (index, step) in steps.iter().enumerate() {
            let n = index + 1;
            let cited: Vec<_> = CITATION.captures_iter(step).collect();
            if cited.is_empty() {
                problems.push(format!(
                    "step {n} cites nothing: end it with (task <id>: <action>)"
                ));
            }
            for citation in cited {
                citations += 1;
                let (task, action) = (&citation[1], citation[2].trim());
                let Some(known) = task.parse().ok().and_then(|task: i64| actions.get(&task)) else {
                    problems.push(format!(
                        "step {n} cites task {task}, which routine {id} does not cite: use one of {tasks:?}"
                    ));
                    continue;
                };
                if !known.contains(action) {
                    problems.push(format!(
                        "step {n}: task {task} has no action \"{action}\": copy one from its actions in task"
                    ));
                }
            }
        }
        if !problems.is_empty() {
            return Err(Error::Invalid(problems.join("; ")));
        }
        Ok(json!({"ok": true, "steps": steps.len(), "citations": citations}))
    }

    /// The task ids routine `id` cites.
    fn routine_tasks(&self, id: i64) -> Result<Vec<i64>> {
        let cites: String = self
            .conn
            .query_row(
                "SELECT cites FROM routines WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::Invalid(format!("no routine {id}: use an id from routines")))?;
        let cites: Value = serde_json::from_str(&cites)?;
        Ok(cites["task_ids"]
            .as_array()
            .map(|ids| ids.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default())
    }

    /// Every cited activity's time span by activity id. An unknown task or repeat is an error
    /// that names it.
    fn cited(
        &self,
        settings: &Settings,
        routine: &Routine,
        utc_offset_s: i32,
    ) -> Result<BTreeMap<i64, (i64, i64)>> {
        let mut cited = BTreeMap::new();
        for &id in &routine.task_ids {
            let (.., activities) = self.task_activities(settings, id)?;
            cited.extend(activities.iter().map(|a| (a.id, (a.start, a.end))));
        }
        for repeat in &routine.repeats {
            for date in &repeat.dates {
                let unknown = || {
                    Error::Invalid(format!(
                        "no {} activity \"{}\" on {date}: copy the app, label and dates from week or day",
                        repeat.app, repeat.label
                    ))
                };
                let today = day(
                    parse(date, utc_offset_s, false).ok_or_else(unknown)?,
                    utc_offset_s,
                );
                let before = cited.len();
                let activities = self.activities(
                    settings,
                    midnight(today, utc_offset_s),
                    midnight(today + 1, utc_offset_s),
                )?;
                cited.extend(
                    activities
                        .iter()
                        .filter(|a| a.app == repeat.app && a.label == repeat.label)
                        .map(|a| (a.id, (a.start, a.end))),
                );
                if cited.len() == before {
                    return Err(unknown());
                }
            }
        }
        Ok(cited)
    }

    /// The first saved routine that shares more than half of the `active` seconds in `spans`.
    fn overlapping(&self, spans: &[(i64, i64)], active: i64) -> Result<Option<(i64, String)>> {
        let mut statement = self.conn.prepare(
            "SELECT r.id, r.title, s.start_ts, s.end_ts
             FROM routines r JOIN routine_spans s ON s.routine_id = r.id ORDER BY r.id",
        )?;
        let mut saved: BTreeMap<i64, (String, Vec<(i64, i64)>)> = BTreeMap::new();
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let title: String = row.get(1)?;
            saved
                .entry(row.get(0)?)
                .or_insert_with(|| (title, Vec::new()))
                .1
                .push((row.get(2)?, row.get(3)?));
        }
        for (id, (title, theirs)) in saved {
            let both = covered_s(spans.iter().chain(&theirs).copied().collect());
            let shared = active + covered_s(theirs) - both;
            if shared * 2 > active {
                return Ok(Some((id, title)));
            }
        }
        Ok(None)
    }

    /// Weeks of five work days among the days that hold any activity, at least one day.
    fn weeks_of_data(&self, utc_offset_s: i32) -> Result<f64> {
        let days: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT (start_ts + ?1) / 86400) FROM activities",
            params![utc_offset_s],
            |row| row.get(0),
        )?;
        Ok(days.max(1) as f64 / WORK_DAYS)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{Repeat, Routine};
    use crate::fixtures::{TempDb, UTC_OFFSET_S, WEEK_NOW, WEEK_START, actions_week, load, local};
    use crate::settings::Settings;
    use crate::store::Store;
    use crate::time::{date, day};

    fn invoice_week(label: &str) -> (TempDb, Store) {
        let (fixture, _) = actions_week();
        let db = TempDb::new(label);
        let mut store = db.open().expect("test setup");
        load(&mut store, &fixture, WEEK_NOW).expect("test setup");
        store.refresh(local(WEEK_NOW)).expect("test setup");
        (db, store)
    }

    /// The task ids of one repeated task kind on the fixture week, such as "inv #".
    fn task_ids(store: &Store, kind: &str) -> Vec<i64> {
        let week = store
            .week_page(&Settings::default(), WEEK_START, UTC_OFFSET_S)
            .expect("week");
        let found = week["repeated_tasks"]
            .as_array()
            .expect("kinds")
            .iter()
            .find(|k| k["label"] == kind)
            .unwrap_or_else(|| panic!("no {kind} in {week}"));
        found["task_ids"]
            .as_array()
            .expect("ids")
            .iter()
            .map(|id| id.as_i64().expect("id"))
            .collect()
    }

    fn routine(task_ids: Vec<i64>, repeats: Vec<Repeat>) -> Routine {
        Routine {
            title: "Send invoices".into(),
            task_ids,
            repeats,
            how: "Draft the invoice email from the sheet and attach the PDF.".into(),
            trigger: "A row is marked done in the sheet.".into(),
            approval: "Mail · AXButton · send".into(),
            why: "The same five steps each time.".into(),
        }
    }

    fn save(store: &Store, routine: &Routine) -> Result<Value, String> {
        store
            .save_routine(&Settings::default(), routine, WEEK_NOW, UTC_OFFSET_S)
            .map_err(|e| e.to_string())
    }

    #[test]
    fn an_unknown_task_or_repeat_is_refused_by_name() {
        let (_db, store) = invoice_week("routine-unknown");
        let mut ids = task_ids(&store, "inv #");
        ids.push(999_999);
        let refused = save(&store, &routine(ids, vec![])).expect_err("unknown task");
        assert!(refused.contains("999999"), "{refused}");

        let repeat = Repeat {
            app: "Mail".into(),
            label: "Nothing like this".into(),
            dates: vec![date(day(WEEK_START, UTC_OFFSET_S))],
        };
        let refused = save(&store, &routine(vec![], vec![repeat])).expect_err("unknown repeat");
        assert!(refused.contains("Nothing like this"), "{refused}");
    }

    #[test]
    fn hours_come_from_the_cited_activities_not_the_agent() {
        let (_db, store) = invoice_week("routine-measured");
        let ids = task_ids(&store, "inv #");
        let saved = save(&store, &routine(ids.clone(), vec![])).expect("saved");

        let settings = Settings::default();
        let task_minutes: f64 = ids
            .iter()
            .map(|&id| {
                store.task_page(&settings, id, UTC_OFFSET_S).expect("task")["active_minutes"]
                    .as_f64()
                    .expect("minutes")
            })
            .sum();
        let observed_days: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(DISTINCT (start_ts + ?1) / 86400) FROM activities",
                [UTC_OFFSET_S],
                |row| row.get(0),
            )
            .expect("days");
        let expected = task_minutes / 60.0 / (observed_days as f64 / 5.0);
        let hours = saved["hours_week"].as_f64().expect("hours");
        assert!((hours - expected).abs() < 0.02, "{hours} vs {expected}");

        let listed = store.routines(UTC_OFFSET_S).expect("routines");
        let first = &listed["routines"][0];
        assert_eq!(first["id"], saved["id"]);
        assert_eq!(first["hours_week"], saved["hours_week"]);
        assert_eq!(first["activities"], saved["activities"]);
        assert_eq!(
            first["cites"]["task_ids"].as_array().expect("ids").len(),
            ids.len()
        );
    }

    #[test]
    fn one_day_of_evidence_is_not_a_routine() {
        let (_db, store) = invoice_week("routine-thin");
        let day_page = store
            .day_page(&Settings::default(), WEEK_START, UTC_OFFSET_S)
            .expect("day");
        let fold = &day_page["other"][0];
        let repeat = Repeat {
            app: fold["app"].as_str().expect("app").into(),
            label: fold["label"].as_str().expect("label").into(),
            dates: vec![date(day(WEEK_START, UTC_OFFSET_S))],
        };
        let refused = save(&store, &routine(vec![], vec![repeat])).expect_err("thin");
        assert!(refused.contains("on 1: cite more"), "{refused}");
    }

    #[test]
    fn a_duplicate_is_refused_with_the_saved_id() {
        let (_db, store) = invoice_week("routine-duplicate");
        let invoices = task_ids(&store, "inv #");
        let saved = save(&store, &routine(invoices.clone(), vec![])).expect("saved");
        let refused = save(&store, &routine(invoices, vec![])).expect_err("duplicate");
        assert!(
            refused.contains(&format!("routine {}", saved["id"])),
            "{refused}"
        );

        let receipts = task_ids(&store, "re #");
        save(&store, &routine(receipts, vec![])).expect("other work saves");
    }

    #[test]
    fn a_skill_passes_only_when_every_step_cites_a_real_action_of_the_routine() {
        let (_db, store) = invoice_week("routine-skill");
        let invoices = task_ids(&store, "inv #");
        let saved = save(&store, &routine(invoices.clone(), vec![])).expect("saved");
        let id = saved["id"].as_i64().expect("id");
        let settings = Settings::default();
        let task = store
            .task_page(&settings, invoices[0], UTC_OFFSET_S)
            .expect("task");
        let actions: Vec<&str> = task["actions"]
            .as_array()
            .expect("actions")
            .iter()
            .map(|a| a["action"].as_str().expect("symbol"))
            .collect();
        assert!(actions.len() >= 2, "{task}");
        let check = |skill: &str| {
            store
                .check_skill(&settings, id, skill)
                .map_err(|e| e.to_string())
        };

        let good = format!(
            "---\nname: send-invoices\n---\n\nSteps:\n\n1. Start here (task {0}: {1})\n2. Then this (task {0}: {2})\n",
            invoices[0], actions[0], actions[1]
        );
        assert_eq!(check(&good).expect("passes")["citations"], 2);

        let receipts = task_ids(&store, "re #");
        let bad = format!(
            "1. Guess\n2. Other work (task {}: {})\n3. Made up (task {}: Mail · AXButton · teleport)\n",
            receipts[0], actions[0], invoices[0]
        );
        let refused = check(&bad).expect_err("bad citations");
        assert!(refused.contains("step 1 cites nothing"), "{refused}");
        assert!(
            refused.contains(&format!("cites task {}", receipts[0])),
            "{refused}"
        );
        assert!(refused.contains("teleport"), "{refused}");
        assert!(check("No steps at all.").is_err());
        assert!(
            store
                .check_skill(&settings, id + 1, &good)
                .is_err_and(|e| e.to_string().contains("no routine"))
        );
    }
}
