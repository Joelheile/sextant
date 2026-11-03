//! The log of what left the database for an agent (plan.md §5, §10): each MCP tool call, the
//! agent bridge's included, with its size. A row is written before the answer goes out, so
//! nothing leaves without one.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::store::Store;

/// One answer an agent received.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sent {
    pub ts: i64,
    /// The MCP client's name from `initialize`.
    pub client: String,
    pub tool: String,
    pub bytes: usize,
}

impl Store {
    /// Appends one row on a short-lived write connection, so the read-only MCP server can log.
    pub fn record_sent(&self, ts: i64, client: &str, tool: &str, bytes: usize) -> Result<()> {
        self.write_conn()?.execute(
            "INSERT INTO sent(ts, client, tool, bytes) VALUES (?1, ?2, ?3, ?4)",
            params![ts, client, tool, bytes as i64],
        )?;
        Ok(())
    }

    /// The newest `limit` rows, newest first.
    pub fn sent(&self, limit: usize) -> Result<Vec<Sent>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT ts, client, tool, bytes FROM sent ORDER BY ts DESC, id DESC LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit as i64], |row| {
            Ok(Sent {
                ts: row.get(0)?,
                client: row.get(1)?,
                tool: row.get(2)?,
                bytes: row.get::<_, i64>(3)? as usize,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use crate::fixtures::TempDb;

    #[test]
    fn a_reader_logs_and_the_newest_row_comes_first() {
        let db = TempDb::new("sent");
        let store = db.open().expect("test setup");
        let reader = store.reader().expect("test setup");

        reader
            .record_sent(10, "claude-ai", "search", 120)
            .expect("log");
        reader
            .record_sent(20, "claude-code", "week", 900)
            .expect("log");

        let sent = store.sent(10).expect("sent");
        assert_eq!(sent.len(), 2);
        assert_eq!((sent[0].tool.as_str(), sent[0].bytes), ("week", 900));
        assert_eq!(sent[1].client, "claude-ai");
    }
}
