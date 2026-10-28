//! Before and after (plan.md §8, §9): the hours a week a workflow takes on the days before a
//! handoff against the days after it. Each side needs 3 days with data; until then the result
//! says "measuring". The handoff day and today are left out, because both are partial.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::params;

use crate::error::Result;
use crate::store::Store;
use crate::time::{DAY, day};

/// Days with data needed on each side of a handoff.
pub const MIN_DAYS: usize = 3;
/// Most days counted on each side, nearest the handoff first.
const MAX_DAYS: usize = 14;
/// How far before the first handoff episodes are read, so 14 days with data fit around weekends.
const LOOKBACK_DAYS: i64 = 30;

/// One workflow that was handed to an agent. A workflow reported twice counts once, from its
/// first report, with the latest status and note.
#[derive(Debug, Clone, PartialEq)]
pub struct Measurement {
    pub handoff_id: i64,
    pub workflow_id: i64,
    pub name: String,
    pub ts: i64,
    pub status: String,
    pub note: String,
    pub days_before: usize,
    pub days_after: usize,
    /// Hours a week before the handoff, once `days_before` reaches [`MIN_DAYS`].
    pub hours_before: Option<f64>,
    /// Hours a week after the handoff, once `days_after` reaches [`MIN_DAYS`].
    pub hours_after: Option<f64>,
}

impl Measurement {
    /// Hours a week given back. Only when both sides are measured; never below zero.
    pub fn saved(&self) -> Option<f64> {
        Some((self.hours_before? - self.hours_after?).max(0.0))
    }

    /// Days with data on the thinner side, at most [`MIN_DAYS`].
    pub fn progress(&self) -> usize {
        self.days_before.min(self.days_after).min(MIN_DAYS)
    }
}

struct Handoff {
    id: i64,
    workflow_id: i64,
    ts: i64,
    name: String,
    sequence: HashSet<String>,
    status: String,
    note: String,
}

/// Seconds per step key, for each local day that has any episode.
type Days = BTreeMap<i64, HashMap<String, i64>>;

fn side<'a>(
    days: impl Iterator<Item = &'a HashMap<String, i64>>,
    sequence: &HashSet<String>,
) -> (usize, Option<f64>) {
    let (mut count, mut seconds) = (0, 0);
    for steps in days.take(MAX_DAYS) {
        count += 1;
        seconds += steps
            .iter()
            .filter(|(key, _)| sequence.contains(*key))
            .map(|(_, s)| s)
            .sum::<i64>();
    }
    let hours = (count >= MIN_DAYS).then(|| seconds as f64 / 3600.0 / count as f64 * 5.0);
    (count, hours)
}

impl Store {
    /// Every reported handoff, newest first, measured as of `now`.
    pub fn results(&self, now: i64) -> Result<Vec<Measurement>> {
        let handoffs = self.handoffs()?;
        let Some(first) = handoffs.iter().map(|h| h.ts).min() else {
            return Ok(Vec::new());
        };
        let days = self.episode_days(first - LOOKBACK_DAYS * DAY, now)?;
        let today = day(now, self.utc_offset_s);
        Ok(handoffs
            .into_iter()
            .map(|h| {
                let cut = day(h.ts, self.utc_offset_s);
                let (days_before, hours_before) =
                    side(days.range(..cut).rev().map(|(_, s)| s), &h.sequence);
                let (days_after, hours_after) =
                    side(days.range(cut + 1..today).map(|(_, s)| s), &h.sequence);
                Measurement {
                    handoff_id: h.id,
                    workflow_id: h.workflow_id,
                    name: h.name,
                    ts: h.ts,
                    status: h.status,
                    note: h.note,
                    days_before,
                    days_after,
                    hours_before,
                    hours_after,
                }
            })
            .collect())
    }

    /// Whether the workflow has been reported as handed off.
    pub fn is_handed_off(&self, workflow_id: i64) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM handoffs WHERE workflow_id = ?1)",
            params![workflow_id],
            |row| row.get(0),
        )?)
    }

    fn handoffs(&self) -> Result<Vec<Handoff>> {
        let mut statement = self.conn.prepare(
            "SELECT id, workflow_id, ts, status, note, name, sequence FROM handoffs ORDER BY ts, id",
        )?;
        let mut rows = statement.query([])?;
        let mut by_workflow: Vec<Handoff> = Vec::new();
        while let Some(row) = rows.next()? {
            let workflow_id: i64 = row.get(1)?;
            let (status, note): (String, String) = (row.get(3)?, row.get(4)?);
            if let Some(known) = by_workflow
                .iter_mut()
                .find(|h| h.workflow_id == workflow_id)
            {
                known.status = status;
                known.note = note;
                continue;
            }
            let keys: Vec<String> = serde_json::from_str(&row.get::<_, String>(6)?)?;
            by_workflow.push(Handoff {
                id: row.get(0)?,
                workflow_id,
                ts: row.get(2)?,
                name: row.get(5)?,
                sequence: keys.into_iter().collect(),
                status,
                note,
            });
        }
        by_workflow.reverse();
        Ok(by_workflow)
    }

    fn episode_days(&self, from: i64, to: i64) -> Result<Days> {
        let mut statement = self.conn.prepare(
            "SELECT e.start_ts, e.end_ts, s.key FROM episodes e JOIN steps s ON s.id = e.step_id
             WHERE e.start_ts BETWEEN ?1 AND ?2",
        )?;
        let mut rows = statement.query(params![from, to])?;
        let mut days = Days::new();
        while let Some(row) = rows.next()? {
            let (start, end): (i64, i64) = (row.get(0)?, row.get(1)?);
            *days
                .entry(day(start, self.utc_offset_s))
                .or_default()
                .entry(row.get(2)?)
                .or_default() += end - start;
        }
        Ok(days)
    }
}
