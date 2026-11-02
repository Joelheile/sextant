//! The agent bridge's data side: which agent CLI runs, and when it last ran. The agent itself
//! reads and saves through the MCP tools.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::store::Store;

/// The agent CLIs the bridge can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentName {
    Claude,
    Codex,
}

impl AgentName {
    pub const ALL: [Self; 2] = [Self::Claude, Self::Codex];

    /// The CLI's name, which is also what the database stores.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|agent| agent.as_str() == text)
    }
}

impl std::fmt::Display for AgentName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The last run: when and which agent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Run {
    pub ts: i64,
    pub agent: AgentName,
}

impl Store {
    /// Replaces the last run with this one.
    pub fn save_run(&mut self, ts: i64, agent: AgentName) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO agent_runs(id, ts, agent) VALUES (1, ?1, ?2)",
            params![ts, agent.as_str()],
        )?;
        Ok(())
    }

    pub fn agent_run(&self) -> Result<Option<Run>> {
        let run = self
            .conn
            .query_row("SELECT ts, agent FROM agent_runs", [], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            });
        let (ts, agent) = match run {
            Ok(run) => run,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        let agent = AgentName::parse(&agent)
            .ok_or_else(|| Error::Invalid(format!("unknown agent {agent} in the last run")))?;
        Ok(Some(Run { ts, agent }))
    }
}
