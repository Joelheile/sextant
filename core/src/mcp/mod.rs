//! MCP server over stdio (plan.md §8): JSON-RPC 2.0, one message per line. The app supplies the
//! pipes, the key and the clock; everything else is here so tests can run it in memory.

mod tools;

use std::cell::RefCell;
use std::io::{BufRead, Write};

use serde_json::{Value, json};

use self::tools::{Ctx, Group, TOOLS};
pub use self::tools::{DEFAULT_LIMIT, FIND_ROUTINES};
use crate::settings::Settings;
use crate::store::Store;
use crate::time::Local;

const LATEST: &str = "2025-06-18";
const SUPPORTED: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const INSTRUCTIONS: &str = "Sextant holds what the user did on this Mac: activities, tasks, the text they saw on screen and what they said. To find work an agent could take over, read week first, then day and task, and save each routine you find with save_routine; routines lists the saved ones, and the find_routines prompt walks through it. Use search, context_at and timeline to look back. list_workflows, get_workflow and get_skill show the workflows Sextant's rules found. Results are capped: pass next_offset as offset for the next page.";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

pub struct Server {
    store: Store,
    settings: Box<dyn Fn() -> Settings>,
    clock: Box<dyn Fn() -> Local>,
    /// The client's name from `initialize`, for the sent log.
    client: RefCell<String>,
}

impl Server {
    /// `store` should be a read-only reader. `settings` is read for every request, so switches
    /// flipped in the app apply to a running server. `clock` gives the time and UTC offset of each
    /// request.
    pub fn new(
        store: Store,
        settings: impl Fn() -> Settings + 'static,
        clock: impl Fn() -> Local + 'static,
    ) -> Self {
        Self {
            store,
            settings: Box::new(settings),
            clock: Box::new(clock),
            client: RefCell::new("unknown".into()),
        }
    }

    /// Reads requests until the input ends, writing one response line per request.
    pub fn serve(&self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Some(response) = self.handle(&line) {
                writeln!(output, "{response}")?;
                output.flush()?;
            }
        }
        Ok(())
    }

    /// The response line for one incoming line, or `None` for notifications and responses.
    pub fn handle(&self, line: &str) -> Option<String> {
        let message: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => return Some(error(Value::Null, PARSE_ERROR, "invalid JSON").to_string()),
        };
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            let is_response = message.get("result").is_some() || message.get("error").is_some();
            return (!is_response)
                .then(|| error(Value::Null, INVALID_REQUEST, "not a request").to_string());
        };
        let id = message.get("id").filter(|id| !id.is_null())?.clone();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let settings = (self.settings)();
        let reply = match method {
            "initialize" => {
                if let Some(name) = params.pointer("/clientInfo/name").and_then(Value::as_str) {
                    *self.client.borrow_mut() = name.to_owned();
                }
                Ok(initialize(&params))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(list(&settings)),
            "tools/call" => self.call(&params, &settings),
            "prompts/list" => Ok(tools::prompts(&settings)),
            "prompts/get" => tools::prompt(&params, &settings).map_err(|m| (INVALID_PARAMS, m)),
            _ => Err((METHOD_NOT_FOUND, format!("unknown method {method}"))),
        };
        Some(
            match reply {
                Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                Err((code, message)) => error(id, code, &message),
            }
            .to_string(),
        )
    }

    fn call(&self, params: &Value, settings: &Settings) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((INVALID_PARAMS, "name is required".to_string()))?;
        let tool = TOOLS
            .iter()
            .find(|tool| tool.name == name)
            .ok_or((INVALID_PARAMS, format!("unknown tool {name}")))?;
        let when = (self.clock)();
        let outcome = if enabled(tool.group, settings) {
            let ctx = Ctx {
                store: &self.store,
                settings,
                when,
            };
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            tools::call(&ctx, name, &args)
        } else {
            Err(format!(
                "{name} is switched off in Sextant's privacy settings"
            ))
        };
        let (text, is_error) = match outcome {
            Ok(text) => (text, false),
            Err(message) => (message, true),
        };
        if !is_error {
            // Nothing leaves without a row in the sent log.
            self.store
                .record_sent(when.now, &self.client.borrow(), name, text.len())
                .map_err(|err| (INTERNAL_ERROR, format!("logging the answer failed: {err}")))?;
        }
        Ok(json!({"content": [{"type": "text", "text": text}], "isError": is_error}))
    }
}

fn enabled(group: Group, settings: &Settings) -> bool {
    match group {
        Group::Rewind => settings.mcp_rewind,
        Group::Workflows => settings.mcp_workflows,
    }
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = asked.filter(|v| SUPPORTED.contains(v)).unwrap_or(LATEST);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}, "prompts": {"listChanged": false}},
        "serverInfo": {"name": "sextant", "title": "Sextant", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
    })
}

fn list(settings: &Settings) -> Value {
    let tools: Vec<Value> = TOOLS
        .iter()
        .filter(|tool| enabled(tool.group, settings))
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": (tool.schema)(),
                "annotations": {
                    "readOnlyHint": tool.read_only,
                    "destructiveHint": false,
                    "openWorldHint": false,
                },
            })
        })
        .collect();
    json!({"tools": tools})
}
