//! MCP tool logic (plan.md §8): pure functions over a read-only `Store`. Results are capped,
//! paginated, free of excluded apps and redacted again on the way out.

use std::collections::HashSet;

use rusqlite::params;
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::finder::Workflow;
use crate::model::Speaker;
use crate::redact::{redact, redact_json};
use crate::search::HitSource;
use crate::settings::Settings;
use crate::store::Store;
use crate::text::round2;
use crate::time::{self, DAY, Local, WEEK_DAYS, iso};
use crate::writer::unpack;

pub const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;
const DEFAULT_WINDOW_S: i64 = 120;
const MAX_WINDOW_S: i64 = 3600;
/// Most frames `context_at` looks at, nearest first.
const MAX_FRAMES: i64 = 300;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Rewind,
    Workflows,
}

pub struct Tool {
    pub name: &'static str,
    pub group: Group,
    pub description: &'static str,
    pub read_only: bool,
    pub schema: fn() -> Value,
}

macro_rules! time_note {
    () => {
        "Times are ISO 8601 such as 2026-03-02T09:40 (local time unless an offset is given), a bare date such as 2026-03-02, or Unix seconds."
    };
}

fn page_schema(mut properties: Value, required: &[&str]) -> Value {
    properties["limit"] = json!({"type": "integer", "description": "Lines or rows per page, default 50, at most 200."});
    properties["offset"] = json!({"type": "integer", "description": "Skip this many results. Use next_offset from the previous page."});
    json!({"type": "object", "properties": properties, "required": required})
}

pub const TOOLS: &[Tool] = &[
    Tool {
        name: "search",
        group: Group::Rewind,
        description: concat!(
            "Find text the user saw on screen or said, newest first, grouped into sessions with time, app and window. Every word must appear in the same line. Use it for questions like what did Anna say about pricing, or find the invoice number I saw yesterday. ",
            time_note!()
        ),
        read_only: true,
        schema: || {
            page_schema(
                json!({
                    "query": {"type": "string", "description": "Words to find."},
                    "from": {"type": "string", "description": "Earliest time."},
                    "to": {"type": "string", "description": "Latest time. A bare date includes that whole day."},
                    "app": {"type": "string", "description": "Only screen text from this app, such as Mail."},
                }),
                &["query"],
            )
        },
    },
    Tool {
        name: "context_at",
        group: Group::Rewind,
        description: concat!(
            "The screen text and speech around one moment, nearest lines first in each group. Use it after search to read what surrounded a hit. ",
            time_note!()
        ),
        read_only: true,
        schema: || {
            json!({"type": "object", "required": ["time"], "properties": {
                "time": {"type": "string", "description": "The moment."},
                "window": {"type": "integer", "description": "Seconds on each side, default 120, at most 3600."},
                "limit": {"type": "integer", "description": "Most lines, default 50, at most 200."},
            }})
        },
    },
    Tool {
        name: "timeline",
        group: Group::Rewind,
        description: concat!(
            "What the user worked on in a time range: activities per app, meetings, reminders, commits and files, oldest first. Activities are rebuilt every few minutes, so the last minutes can be missing. ",
            time_note!()
        ),
        read_only: true,
        schema: || {
            page_schema(
                json!({
                    "from": {"type": "string", "description": "Start of the range."},
                    "to": {"type": "string", "description": "End of the range. A bare date includes that whole day."},
                }),
                &["from", "to"],
            )
        },
    },
    Tool {
        name: "week",
        group: Group::Workflows,
        description: concat!(
            "Where the user's time went over 7 days, compiled by Sextant from what they did on their Mac: active minutes and top tasks per day, task kinds that repeat (numbers become #, so \"inv #\" is invoices) and activities that repeat on 2 or more days. Start here to find work an agent could take over, then open a day or a task. Without start you get the 7 days ending today. ",
            time_note!()
        ),
        read_only: true,
        schema: || {
            json!({"type": "object", "properties": {
                "start": {"type": "string", "description": "First day, such as 2026-03-02."},
            }})
        },
    },
    Tool {
        name: "day",
        group: Group::Workflows,
        description: concat!(
            "One day. An activity is one focus span on one window, document or page. Lists the day's tasks by minutes, then their activities in time order (id, start time, minutes, app, label, task, clicks and edits), then all other activities folded by app and label with the first time, count and minutes. date defaults to today. ",
            time_note!()
        ),
        read_only: true,
        schema: || {
            json!({"type": "object", "properties": {
                "date": {"type": "string", "description": "The day, such as 2026-03-02."},
            }})
        },
    },
    Tool {
        name: "task",
        group: Group::Workflows,
        description: "One task: activities joined by a shared identifier such as an invoice or ticket number, across apps and days. Lists its activities and its actions in order (app · control role · label), repeats folded, with the kind of value typed (email, id, amount, date, text). The typed values show only when screen text is switched on.",
        read_only: true,
        schema: || {
            json!({"type": "object", "required": ["id"], "properties": {
                "id": {"type": "integer", "description": "Task id from week or day."},
            }})
        },
    },
    Tool {
        name: "list_workflows",
        group: Group::Workflows,
        description: "The user's recurring workflows across apps, ranked by estimated hours a week an agent could save. Each has hours, automation potential, rough (backfill only) and the reasons behind its scores.",
        read_only: true,
        schema: || page_schema(json!({}), &[]),
    },
    Tool {
        name: "get_workflow",
        group: Group::Workflows,
        description: "One workflow in detail: steps, how steps follow each other, data copied by hand, example cases and open promises.",
        read_only: true,
        schema: || {
            json!({"type": "object", "required": ["id"], "properties": {
                "id": {"type": "integer", "description": "Workflow id from list_workflows."},
            }})
        },
    },
    Tool {
        name: "get_skill",
        group: Group::Workflows,
        description: "A ready SKILL.md in Markdown that tells an agent how to run one workflow.",
        read_only: true,
        schema: || {
            json!({"type": "object", "required": ["id"], "properties": {
                "id": {"type": "integer", "description": "Workflow id from list_workflows."},
            }})
        },
    },
    Tool {
        name: "report_handoff",
        group: Group::Workflows,
        description: "Record that an agent took over a workflow, so Sextant can compare the hours before and after. This is the only tool that writes.",
        read_only: false,
        schema: || {
            json!({"type": "object", "required": ["id", "status"], "properties": {
                "id": {"type": "integer", "description": "Workflow id from list_workflows."},
                "status": {"type": "string", "description": "For example started, done or abandoned."},
                "note": {"type": "string", "description": "One sentence on what the agent now does."},
            }})
        },
    },
];

pub struct Ctx<'a> {
    pub store: &'a Store,
    pub settings: &'a Settings,
    pub when: Local,
}

/// Runs one tool. `Err` text goes back to the model as a tool error.
pub fn call(ctx: &Ctx, name: &str, args: &Value) -> std::result::Result<String, String> {
    let run = match name {
        "search" => search(ctx, args),
        "context_at" => context_at(ctx, args),
        "timeline" => timeline(ctx, args),
        "week" => week(ctx, args),
        "day" => time_arg(ctx, args, "date", false).and_then(|at| {
            ctx.store.day_page(
                ctx.settings,
                at.unwrap_or(ctx.when.now),
                ctx.when.utc_offset_s,
            )
        }),
        "task" => int(args, "id").and_then(|id| {
            let id = id.ok_or_else(|| bad("id is required"))?;
            ctx.store.task_page(ctx.settings, id, ctx.when.utc_offset_s)
        }),
        "list_workflows" => list_workflows(ctx, args),
        "get_workflow" => get_workflow(ctx, args),
        "get_skill" => return get_skill(ctx, args).map_err(|e| e.to_string()),
        "report_handoff" => report_handoff(ctx, args),
        _ => return Err(format!("unknown tool {name}")),
    };
    run.map(|mut value| {
        redact_json(&mut value, &ctx.store.secret);
        value.to_string()
    })
    .map_err(|e| e.to_string())
}

fn bad(message: &'static str) -> Error {
    Error::Invalid(message.to_string())
}

/// A whole number argument. Missing or null is `None`; a float or a string is an error.
fn int(args: &Value, key: &str) -> Result<Option<i64>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| Error::Invalid(format!("{key} must be a whole number"))),
    }
}

fn page(args: &Value) -> Result<(usize, usize)> {
    let limit = int(args, "limit")?.unwrap_or(DEFAULT_LIMIT as i64);
    let offset = int(args, "offset")?.unwrap_or(0);
    if limit < 1 {
        return Err(bad("limit must be at least 1"));
    }
    if offset < 0 {
        return Err(bad("offset must not be negative"));
    }
    Ok((limit.min(MAX_LIMIT as i64) as usize, offset as usize))
}

fn time_arg(ctx: &Ctx, args: &Value, key: &str, end_of_day: bool) -> Result<Option<i64>> {
    let Some(value) = args.get(key).filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let parsed = match value {
        Value::Number(n) => n
            .as_i64()
            .map(|s| if s > 100_000_000_000 { s / 1000 } else { s }),
        Value::String(text) => time::parse(text, ctx.when.utc_offset_s, end_of_day),
        _ => None,
    };
    parsed.map(Some).ok_or_else(|| {
        Error::Invalid(format!(
            "{key} is not a time: use 2026-03-02T09:40, 2026-03-02 or Unix seconds"
        ))
    })
}

fn text_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn at(ctx: &Ctx, ts: i64) -> String {
    iso(ts, ctx.when.utc_offset_s)
}

fn next_offset(full: bool, offset: usize, taken: usize) -> Value {
    if full {
        json!(offset + taken)
    } else {
        Value::Null
    }
}

fn search(ctx: &Ctx, args: &Value) -> Result<Value> {
    let query = text_arg(args, "query").ok_or_else(|| bad("query is required"))?;
    let from = time_arg(ctx, args, "from", false)?;
    let to = time_arg(ctx, args, "to", true)?;
    let app = text_arg(args, "app");
    let (limit, offset) = page(args)?;
    let page = ctx
        .store
        .search_sessions(ctx.settings, query, (from, to), app, limit, offset)?;
    let sessions: Vec<Value> = page
        .sessions
        .iter()
        .map(|s| {
            let source = match s.source {
                HitSource::Screen => "screen",
                HitSource::Mic => "me",
                HitSource::System => "them",
            };
            json!({
                "start": at(ctx, s.start),
                "end": at(ctx, s.end),
                "source": source,
                "app": s.app,
                "window": s.title,
                "url": s.url,
                "lines": s.lines.iter().map(|(ts, text)| json!({"time": at(ctx, *ts), "text": text})).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!({
        "lines": page.lines,
        "sessions": sessions,
        "next_offset": page.next_offset,
    }))
}

fn context_at(ctx: &Ctx, args: &Value) -> Result<Value> {
    let center = time_arg(ctx, args, "time", false)?.ok_or_else(|| bad("time is required"))?;
    let window = int(args, "window")?
        .unwrap_or(DEFAULT_WINDOW_S)
        .clamp(1, MAX_WINDOW_S);
    let (limit, _) = page(args)?;
    let (low, high) = (center - window, center + window);

    let mut speech = ctx.store.conn.prepare_cached(
        "SELECT start_ts, source, text FROM utterances WHERE start_ts BETWEEN ?1 AND ?2
         ORDER BY ABS(start_ts - ?3), id LIMIT ?4",
    )?;
    let speech_cap = (limit / 2).max(1);
    let mut said: Vec<(i64, String, String)> = speech
        .query_map(params![low, high, center, speech_cap as i64 + 1], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let mut truncated = said.len() > speech_cap;
    said.truncate(speech_cap);
    said.sort_by_key(|(ts, _, _)| *ts);

    let screen_cap = limit - said.len();
    let mut frames = ctx.store.conn.prepare_cached(
        "SELECT f.ts, f.packed, w.app, w.title, w.url_host_path FROM frames f
         JOIN windows w ON w.id = f.window_id WHERE f.ts BETWEEN ?1 AND ?2
         ORDER BY ABS(f.ts - ?3), f.id LIMIT ?4",
    )?;
    let mut rows = frames.query(params![low, high, center, MAX_FRAMES])?;
    let mut text = ctx
        .store
        .conn
        .prepare_cached("SELECT text FROM lines WHERE id = ?1")?;
    let mut seen = HashSet::new();
    let mut screen: Vec<(i64, String, String, String, Vec<String>)> = Vec::new();
    let mut taken = 0;
    'frames: while let Some(row) = rows.next()? {
        let app: String = row.get(2)?;
        if ctx.settings.is_excluded(&app) {
            continue;
        }
        let mut lines = Vec::new();
        for (line_id, _) in unpack(&row.get::<_, Vec<u8>>(1)?) {
            if !seen.insert(line_id) {
                continue;
            }
            if taken == screen_cap {
                truncated = true;
                break 'frames;
            }
            lines.push(text.query_row(params![line_id], |r| r.get(0))?);
            taken += 1;
        }
        if !lines.is_empty() {
            screen.push((row.get(0)?, app, row.get(3)?, row.get(4)?, lines));
        }
    }
    screen.sort_by_key(|(ts, ..)| *ts);

    Ok(json!({
        "at": at(ctx, center),
        "window": window,
        "screen": screen.iter().map(|(ts, app, title, url, lines)| json!({
            "time": at(ctx, *ts), "app": app, "window": title, "url": url, "lines": lines,
        })).collect::<Vec<_>>(),
        "speech": said.iter().map(|(ts, source, text)| json!({
            "time": at(ctx, *ts), "who": if Speaker::parse(source) == Some(Speaker::Mic) { "me" } else { "them" }, "text": text,
        })).collect::<Vec<_>>(),
        "truncated": truncated,
    }))
}

fn week(ctx: &Ctx, args: &Value) -> Result<Value> {
    let last_week = ctx.when.now - (WEEK_DAYS - 1) * DAY;
    let start = time_arg(ctx, args, "start", false)?.unwrap_or(last_week);
    ctx.store
        .week_page(ctx.settings, start, ctx.when.utc_offset_s)
}

struct Item {
    start: i64,
    end: i64,
    kind: &'static str,
    app: String,
    what: String,
}

fn timeline(ctx: &Ctx, args: &Value) -> Result<Value> {
    let from = time_arg(ctx, args, "from", false)?.ok_or_else(|| bad("from is required"))?;
    let to = time_arg(ctx, args, "to", true)?.ok_or_else(|| bad("to is required"))?;
    let (limit, offset) = page(args)?;
    let want = (limit + offset) as i64;

    let mut items: Vec<Item> = Vec::new();
    let mut activities = ctx.store.conn.prepare_cached(
        "SELECT e.start_ts, e.end_ts, e.app, s.name FROM episodes e JOIN steps s ON s.id = e.step_id
         WHERE e.start_ts BETWEEN ?1 AND ?2 AND e.object_key NOT LIKE 'meeting|%'
         ORDER BY e.start_ts, e.id LIMIT ?3",
    )?;
    for row in activities.query_map(params![from, to, want], |row| {
        Ok(Item {
            start: row.get(0)?,
            end: row.get(1)?,
            kind: "activity",
            app: row.get(2)?,
            what: row.get(3)?,
        })
    })? {
        items.push(row?);
    }
    let mut events = ctx.store.conn.prepare_cached(
        "SELECT ts, kind, json FROM events WHERE ts BETWEEN ?1 AND ?2
         AND kind IN ('calendar', 'reminder', 'commit', 'file') ORDER BY ts, id LIMIT ?3",
    )?;
    let mut rows = events.query(params![from, to, want])?;
    while let Some(row) = rows.next()? {
        let kind: String = row.get(1)?;
        let payload: Value = serde_json::from_str(&row.get::<_, String>(2)?)?;
        let field = |key: &str| payload[key].as_str().unwrap_or("").to_string();
        let ts: i64 = row.get(0)?;
        items.push(match kind.as_str() {
            "calendar" => Item {
                start: ts,
                end: payload["end_ts"].as_i64().unwrap_or(ts),
                kind: "meeting",
                app: "Calendar".into(),
                what: field("title"),
            },
            "reminder" => Item {
                start: ts,
                end: ts,
                kind: "reminder",
                app: "Reminders".into(),
                what: if payload["done"].as_bool().unwrap_or(false) {
                    format!("{} (done)", field("title"))
                } else {
                    field("title")
                },
            },
            "commit" => Item {
                start: ts,
                end: ts,
                kind: "commit",
                app: "git".into(),
                what: format!("{}: {}", field("repo"), field("subject")),
            },
            _ => Item {
                start: ts,
                end: ts,
                kind: "file",
                app: "Files".into(),
                what: field("name"),
            },
        });
    }
    items.retain(|item| !ctx.settings.is_excluded(&item.app));
    items.sort_by_key(|item| (item.start, item.end));
    let page: Vec<&Item> = items.iter().skip(offset).take(limit).collect();
    Ok(json!({
        "from": at(ctx, from),
        "to": at(ctx, to),
        "items": page.iter().map(|i| json!({
            "start": at(ctx, i.start), "end": at(ctx, i.end), "kind": i.kind, "app": i.app, "what": i.what,
        })).collect::<Vec<_>>(),
        "next_offset": next_offset(items.len() > offset + limit, offset, limit),
    }))
}

/// Workflows with a step in an excluded app are hidden. A step key starts with its app, or
/// with the host for web pages.
fn shown(ctx: &Ctx, workflow: &Workflow) -> bool {
    !workflow.steps.iter().any(|step| {
        ctx.settings
            .is_excluded(step.key.split('|').next().unwrap_or(""))
    })
}

fn workflow_row(workflow: &Workflow) -> Value {
    json!({
        "id": workflow.id,
        "name": workflow.name,
        "hours_week": round2(workflow.hours_week),
        "potential": round2(workflow.potential),
        "benefit": round2(workflow.benefit),
        "kind": workflow.kind.as_str(),
        "action": workflow.action.as_str(),
        "enjoyment": workflow.enjoyment.as_str(),
        "rough": workflow.rough,
        "cases": workflow.task_count,
        "first_step": workflow.first_step,
        "steps": workflow.steps.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        "reasons": workflow.reasons,
    })
}

fn list_workflows(ctx: &Ctx, args: &Value) -> Result<Value> {
    let (limit, offset) = page(args)?;
    let all: Vec<Workflow> = ctx
        .store
        .workflows()?
        .into_iter()
        .filter(|w| shown(ctx, w))
        .collect();
    Ok(json!({
        "total": all.len(),
        "hours_back": round2(ctx.store.hours_back()?),
        "workflows": all.iter().skip(offset).take(limit).map(workflow_row).collect::<Vec<_>>(),
        "next_offset": next_offset(all.len() > offset + limit, offset, limit),
    }))
}

fn workflow_id(args: &Value) -> Result<i64> {
    int(args, "id")?.ok_or_else(|| bad("id is required"))
}

/// The workflow if it exists and shows no excluded app.
fn visible(ctx: &Ctx, id: i64) -> Result<()> {
    match ctx.store.workflow(id)? {
        Some(workflow) if shown(ctx, &workflow) => Ok(()),
        _ => Err(Error::NoWorkflow(id)),
    }
}

fn get_workflow(ctx: &Ctx, args: &Value) -> Result<Value> {
    let id = workflow_id(args)?;
    visible(ctx, id)?;
    let detail = ctx
        .store
        .workflow_detail(id)?
        .ok_or(Error::NoWorkflow(id))?;
    let mut row = workflow_row(&detail.workflow);
    row["steps"] = detail
        .workflow
        .steps
        .iter()
        .map(|s| json!({"name": s.name, "category": s.category.as_str(), "minutes": round2(s.minutes), "potential": round2(s.potential)}))
        .collect();

    row["follows"] = ctx
        .store
        .step_links(&detail.workflow)?
        .iter()
        .map(|l| json!({"from": l.from, "to": l.to, "count": l.count, "median_gap_s": l.median_gap_s, "copies": l.copies, "days": l.days}))
        .collect();
    row["copy_points"] = detail
        .copy_points
        .iter()
        .map(|c| json!({"from": c.from, "to": c.to, "copies": c.copies, "retypes": c.retypes}))
        .collect();
    row["examples"] = detail
        .examples
        .iter()
        .map(|c| json!({"case": c.entity, "from": at(ctx, c.first_ts), "to": at(ctx, c.last_ts), "path": c.path}))
        .collect();
    row["commitments"] = detail
        .commitments
        .iter()
        .map(|c| {
            json!({"time": at(ctx, c.ts), "source": c.source, "text": c.text,
                        "due": c.due_ts.map(|d| at(ctx, d)), "overdue": c.overdue})
        })
        .collect();
    Ok(row)
}

fn get_skill(ctx: &Ctx, args: &Value) -> Result<String> {
    let id = workflow_id(args)?;
    visible(ctx, id)?;
    Ok(redact(
        &ctx.store.skill_md(id, ctx.when.utc_offset_s)?,
        &ctx.store.secret,
    ))
}

fn report_handoff(ctx: &Ctx, args: &Value) -> Result<Value> {
    let id = workflow_id(args)?;
    let status = text_arg(args, "status").ok_or_else(|| bad("status is required"))?;
    let note = text_arg(args, "note").unwrap_or("");
    visible(ctx, id)?;
    let handoff = ctx.store.record_handoff(
        id,
        ctx.when.now,
        &status.chars().take(40).collect::<String>(),
        &note.chars().take(500).collect::<String>(),
    )?;
    Ok(json!({"recorded": true, "handoff_id": handoff}))
}
