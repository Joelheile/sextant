//! The on-device model's view of how much of a step an agent could do. The app asks the model
//! about steps it has not judged yet and stores the answer here; refresh reads it back.

use std::collections::HashMap;

use rusqlite::{Connection, params};

use crate::error::Result;
use crate::redact::redact;
use crate::store::Store;

/// Object keys per step shown to the model.
const SAMPLE_KEYS: usize = 5;

/// What the model sees: no screen text, only the step's name, category, apps and templated keys.
#[derive(Debug, Clone, PartialEq)]
pub struct StepSample {
    pub key: String,
    pub name: String,
    pub category: String,
    pub apps: Vec<String>,
    pub objects: Vec<String>,
    pub minutes: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Judgment {
    pub share: f64,
    pub reason: String,
}

pub(crate) fn load(conn: &Connection) -> Result<HashMap<String, Judgment>> {
    let mut statement = conn.prepare("SELECT key, share, reason FROM judgments")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get(0)?,
            Judgment {
                share: row.get(1)?,
                reason: row.get(2)?,
            },
        ))
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

impl Store {
    /// Up to `limit` steps without a judgment, the most time first.
    pub fn unjudged_steps(&self, limit: usize) -> Result<Vec<StepSample>> {
        let mut steps = self.conn.prepare(
            "SELECT s.id, s.key, s.name, s.category, s.minutes FROM steps s
             LEFT JOIN judgments j ON j.key = s.key
             WHERE j.key IS NULL ORDER BY s.minutes DESC, s.key LIMIT ?1",
        )?;
        let rows = steps.query_map(params![limit as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                StepSample {
                    key: row.get(1)?,
                    name: row.get(2)?,
                    category: row.get(3)?,
                    apps: Vec::new(),
                    objects: Vec::new(),
                    minutes: row.get(4)?,
                },
            ))
        })?;
        let mut episodes = self.conn.prepare_cached(
            "SELECT app, object_key, COUNT(*) AS n FROM episodes WHERE step_id = ?1
             GROUP BY app, object_key ORDER BY n DESC, object_key",
        )?;
        let mut out = Vec::new();
        for row in rows {
            let (id, mut sample) = row?;
            let found = episodes.query_map(params![id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for pair in found {
                let (app, object) = pair?;
                if !sample.apps.contains(&app) {
                    sample.apps.push(app);
                }
                if sample.objects.len() < SAMPLE_KEYS {
                    sample.objects.push(redact(&object, &self.secret));
                }
            }
            out.push(sample);
        }
        Ok(out)
    }

    /// Stores the model's judgment of step `key`. `share` is clamped to 0..=1 and the reason is
    /// redacted. Takes effect at the next `refresh`.
    pub fn add_judgment(&mut self, ts: i64, key: &str, share: f64, reason: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO judgments (key, share, reason, ts) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(key) DO UPDATE SET share = ?2, reason = ?3, ts = ?4",
            params![key, share.clamp(0.0, 1.0), redact(reason, &self.secret), ts],
        )?;
        Ok(())
    }
}
