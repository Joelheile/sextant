//! Rewind search history: the questions asked in the search panel and their answers.

use rusqlite::params;

use crate::error::Result;
use crate::redact::redact;
use crate::store::Store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub id: i64,
    pub ts: i64,
    pub question: String,
    pub answer: String,
}

impl Store {
    /// Saves a question and its answer, both redacted, and returns them as stored.
    pub fn add_turn(&mut self, ts: i64, question: &str, answer: &str) -> Result<Turn> {
        let question = redact(question, &self.secret);
        let answer = redact(answer, &self.secret);
        self.conn.execute(
            "INSERT INTO turns (ts, question, answer) VALUES (?1, ?2, ?3)",
            params![ts, question, answer],
        )?;
        Ok(Turn {
            id: self.conn.last_insert_rowid(),
            ts,
            question,
            answer,
        })
    }

    /// Up to `limit` turns older than turn `before` (or the newest), newest first.
    pub fn turns(&self, before: Option<i64>, limit: usize) -> Result<Vec<Turn>> {
        let mut query = self.conn.prepare(
            "SELECT id, ts, question, answer FROM turns WHERE id < ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = query.query_map(params![before.unwrap_or(i64::MAX), limit as i64], |row| {
            Ok(Turn {
                id: row.get(0)?,
                ts: row.get(1)?,
                question: row.get(2)?,
                answer: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}
