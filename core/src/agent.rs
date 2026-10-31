//! The agent bridge's data side. Sextant hands your own Claude Code or Codex the step graph of
//! your workflows and gets back automation ideas. The brief holds each step's app, category and
//! minutes, and how steps follow each other. It never holds window titles, workflow or step
//! names, screen text, transcripts, entities, example tasks or commitments.

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::redact::redact;
use crate::store::Store;
use crate::text::round2;

/// What the agent is asked. The brief follows it.
pub const PROMPT: &str = "You help one person find work on their Mac that an AI agent could take \
over. Below is the step graph of their recurring workflows, measured on their Mac: each workflow \
has numbered steps (app, category, minutes, a 0 to 1 share an agent could do) and links between \
step numbers (how often one step follows another, the median gap and how often text was copied \
between them). Suggest at most 8 concrete automations, best first. For each give the workflow_id, \
a short title (under 60 characters), how an agent would do it (one or two sentences: which tools, \
what it reads, what it writes) and the hours a week it would give back, at most the workflow's \
hours_week. Set confidence from 0 to 1. Skip workflows where an agent would not help. Answer \
with JSON only, matching the schema.";

/// The JSON Schema of the answer, for `--json-schema` and `--output-schema`.
pub const SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"required":["ideas"],"properties":{"ideas":{"type":"array","maxItems":8,"items":{"type":"object","additionalProperties":false,"required":["workflow_id","title","how","hours_week","confidence"],"properties":{"workflow_id":{"type":"integer"},"title":{"type":"string"},"how":{"type":"string"},"hours_week":{"type":"number"},"confidence":{"type":"number"}}}}}}"#;

const MAX_IDEAS: usize = 8;
const MAX_TITLE: usize = 80;
const MAX_HOW: usize = 400;

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

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Idea {
    pub workflow_id: i64,
    pub title: String,
    pub how: String,
    pub hours_week: f64,
    pub confidence: f64,
}

/// An idea with the name of its workflow, as the menu bar shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedIdea {
    pub idea: Idea,
    pub workflow: String,
}

/// The last run: when, which agent and what it found.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub ts: i64,
    pub agent: AgentName,
    pub ideas: Vec<SavedIdea>,
}

#[derive(Deserialize)]
struct Answer {
    ideas: Vec<Idea>,
}

/// Reads the agent's answer. `workflows` holds the id and hours a week of every workflow in the
/// brief: ideas for other ids are dropped, and no idea saves more than its workflow takes.
pub fn parse_ideas(answer: &str, workflows: &[(i64, f64)]) -> Result<Vec<Idea>> {
    let json = match (answer.find('{'), answer.rfind('}')) {
        (Some(start), Some(end)) if start < end => &answer[start..=end],
        _ => return Err(Error::Invalid("the answer holds no JSON".into())),
    };
    let answer: Answer = serde_json::from_str(json)
        .map_err(|err| Error::Invalid(format!("unreadable answer: {err}")))?;
    Ok(answer
        .ideas
        .into_iter()
        .filter_map(|idea| {
            let (_, hours) = workflows.iter().find(|(id, _)| *id == idea.workflow_id)?;
            let title = clip(&idea.title, MAX_TITLE);
            (!title.is_empty()).then(|| Idea {
                workflow_id: idea.workflow_id,
                title,
                how: clip(&idea.how, MAX_HOW),
                hours_week: idea.hours_week.clamp(0.0, *hours),
                confidence: idea.confidence.clamp(0.0, 1.0),
            })
        })
        .take(MAX_IDEAS)
        .collect())
}

fn clip(text: &str, max: usize) -> String {
    text.trim().chars().take(max).collect()
}

impl Store {
    /// The step graph the agent gets, as JSON.
    pub fn agent_brief(&self) -> Result<String> {
        let secret = self.secret();
        let mut workflows = Vec::new();
        for workflow in self.workflows()? {
            let step_number = |key: &str| {
                workflow
                    .steps
                    .iter()
                    .position(|step| step.key == key)
                    .map_or(0, |index| index + 1)
            };
            let links: Vec<Value> = self
                .step_links(&workflow)?
                .into_iter()
                .map(|link| {
                    json!({
                        "from": step_number(&link.from_key),
                        "to": step_number(&link.to_key),
                        "count": link.count,
                        "median_gap_s": link.median_gap_s,
                        "copies": link.copies,
                    })
                })
                .collect();
            let steps: Vec<Value> = workflow
                .steps
                .iter()
                .enumerate()
                .map(|(index, step)| {
                    json!({
                        "step": index + 1,
                        "app": redact(app_of(&step.name), &secret),
                        "category": step.category.as_str(),
                        "minutes": round2(step.minutes),
                        "agent_share": round2(step.potential),
                    })
                })
                .collect();
            workflows.push(json!({
                "workflow_id": workflow.id,
                "hours_week": round2(workflow.hours_week),
                "tasks": workflow.task_count,
                "kind": workflow.kind.as_str(),
                "steps": steps,
                "links": links,
            }));
        }
        Ok(json!({ "workflows": workflows }).to_string())
    }

    /// Ids and hours a week of the workflows in the brief, for [`parse_ideas`].
    pub fn agent_workflows(&self) -> Result<Vec<(i64, f64)>> {
        Ok(self
            .workflows()?
            .into_iter()
            .map(|w| (w.id, w.hours_week))
            .collect())
    }

    /// Replaces the last run with this one.
    pub fn save_ideas(&mut self, ts: i64, agent: AgentName, ideas: &[Idea]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM agent_ideas", [])?;
        tx.execute(
            "INSERT OR REPLACE INTO agent_runs(id, ts, agent) VALUES (1, ?1, ?2)",
            params![ts, agent.as_str()],
        )?;
        for idea in ideas {
            tx.execute(
                "INSERT INTO agent_ideas(workflow_id, title, how, hours_week, confidence)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    idea.workflow_id,
                    redact(&idea.title, &self.secret),
                    redact(&idea.how, &self.secret),
                    idea.hours_week,
                    idea.confidence
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The last run, biggest ideas first. Ideas whose workflow is gone are left out.
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
        let mut statement = self.conn.prepare(
            "SELECT i.workflow_id, i.title, i.how, i.hours_week, i.confidence, w.name
             FROM agent_ideas i JOIN workflows w ON w.id = i.workflow_id
             ORDER BY i.hours_week DESC, i.id",
        )?;
        let ideas = statement
            .query_map([], |row| {
                Ok(SavedIdea {
                    idea: Idea {
                        workflow_id: row.get(0)?,
                        title: row.get(1)?,
                        how: row.get(2)?,
                        hours_week: row.get(3)?,
                        confidence: row.get(4)?,
                    },
                    workflow: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Some(Run { ts, agent, ideas }))
    }
}

/// A step name is `app: words`. Only the app leaves the Mac.
fn app_of(step_name: &str) -> &str {
    step_name.split(": ").next().unwrap_or("")
}
