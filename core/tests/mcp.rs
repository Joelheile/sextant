//! MCP client test (plan.md §8, §13 M2): drives the server through its JSON-RPC lines against the
//! fixture week and times every tool. Budget numbers: `cargo test -p sextant-core --release --test mcp -- --nocapture`.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::io::Cursor;
use std::rc::Rc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sextant_core::fixtures::*;
use sextant_core::mcp::Server;
use sextant_core::{Settings, Store};

const MCP_P95: Duration = Duration::from_millis(200);
const DAY: i64 = 86_400;

struct Client {
    server: Server,
    settings: Rc<RefCell<Settings>>,
    next_id: i64,
}

impl Client {
    fn new(reader: Store) -> Self {
        let settings = Rc::new(RefCell::new(Settings::default()));
        let shared = Rc::clone(&settings);
        Self {
            server: Server::new(reader, move || shared.borrow().clone(), || local(WEEK_NOW)),
            settings,
            next_id: 0,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let line =
            json!({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params});
        let reply = self.server.handle(&line.to_string()).expect("a reply");
        let reply: Value = serde_json::from_str(&reply).expect("json reply");
        assert_eq!(reply["id"], self.next_id);
        assert_eq!(reply["jsonrpc"], "2.0");
        reply
    }

    /// The tool's text, parsed as JSON when it is JSON. `Err` when the tool reports an error.
    fn tool(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        let reply = self.request("tools/call", json!({"name": name, "arguments": arguments}));
        let result = &reply["result"];
        let text = result["content"][0]["text"]
            .as_str()
            .expect("text content")
            .to_string();
        if result["isError"] == true {
            return Err(text);
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }

    fn tool_names(&mut self) -> BTreeSet<String> {
        let reply = self.request("tools/list", json!({}));
        reply["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .map(|t| t["name"].as_str().expect("name").to_string())
            .collect()
    }
}

fn fixture_client(label: &str) -> (TempDb, Client) {
    let db = TempDb::new(label);
    let mut store = db.open().expect("test setup");
    load(&mut store, &week(), WEEK_NOW).expect("test setup");
    store.refresh(local(WEEK_NOW)).expect("test setup");
    let reader = store.reader().expect("test setup");
    (db, Client::new(reader))
}

#[test]
fn the_handshake_lists_every_tool() {
    let (_db, mut client) = fixture_client("mcp-handshake");
    let init = client.request(
        "initialize",
        json!({"protocolVersion": "2024-11-05", "capabilities": {}}),
    );
    assert_eq!(init["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(init["result"]["serverInfo"]["name"], "sextant");
    assert!(init["result"]["capabilities"]["tools"].is_object());

    let initialized = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    assert_eq!(client.server.handle(&initialized.to_string()), None);
    assert_eq!(client.request("ping", json!({}))["result"], json!({}));

    let names: Vec<String> = client.tool_names().into_iter().collect();
    assert_eq!(
        names,
        [
            "check_skill",
            "context_at",
            "day",
            "routines",
            "save_routine",
            "search",
            "task",
            "timeline",
            "week"
        ]
    );
    let listed = client.request("tools/list", json!({}));
    for tool in listed["result"]["tools"].as_array().expect("tools") {
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
        assert!(
            !tool["description"]
                .as_str()
                .expect("description")
                .is_empty()
        );
    }
}

#[test]
fn every_answer_is_logged_with_its_client_and_size_before_it_goes_out() {
    let (db, mut client) = fixture_client("mcp-sent");
    client.request(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "clientInfo": {"name": "claude-code"}}),
    );
    let text = client.tool("week", json!({})).expect("week").to_string();
    assert!(client.tool("task", json!({"id": -1})).is_err());

    let sent = db.open().expect("store").sent(10).expect("sent");
    assert_eq!(sent.len(), 1, "errors send nothing worth logging");
    assert_eq!(
        (sent[0].client.as_str(), sent[0].tool.as_str(), sent[0].ts),
        ("claude-code", "week", WEEK_NOW)
    );
    assert_eq!(sent[0].bytes, text.len());
}

#[test]
fn bad_requests_get_json_rpc_errors() {
    let (_db, mut client) = fixture_client("mcp-errors");
    let parse: Value =
        serde_json::from_str(&client.server.handle("{nope").expect("reply")).expect("json");
    assert_eq!(parse["error"]["code"], -32700);
    assert_eq!(
        client.request("resources/list", json!({}))["error"]["code"],
        -32601
    );
    assert_eq!(
        client.request("tools/call", json!({"name": "nope"}))["error"]["code"],
        -32602
    );
    assert_eq!(
        client.request("tools/call", json!({}))["error"]["code"],
        -32602
    );
    assert!(
        client.tool("search", json!({})).is_err(),
        "query is required"
    );
    assert!(
        client
            .tool(
                "timeline",
                json!({"from": "yesterday-ish", "to": "2026-03-06"})
            )
            .is_err()
    );
}

#[test]
fn the_server_loop_answers_one_line_per_request_over_pipes() {
    let (_db, client) = fixture_client("mcp-pipes");
    let input = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "routines", "arguments": {}}}),
    ]
    .map(|m| m.to_string())
    .join("\n\n");
    let mut output = Vec::new();
    client
        .server
        .serve(Cursor::new(input), &mut output)
        .expect("serve");
    let lines: Vec<Value> = String::from_utf8(output)
        .expect("utf8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("json line"))
        .collect();
    assert_eq!(
        lines.iter().map(|l| l["id"].clone()).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(lines[2]["result"]["isError"], false);
}

#[test]
fn every_tool_answers_from_the_fixture_week() {
    let (_db, mut client) = fixture_client("mcp-every-tool");

    let found = client
        .tool("search", json!({"query": "pricing"}))
        .expect("search");
    let sessions = found["sessions"].as_array().expect("sessions");
    assert!(!sessions.is_empty());
    assert!(found["lines"].as_i64().expect("lines") >= 1);
    assert!(
        sessions
            .iter()
            .any(|s| s["lines"].to_string().contains("Anna Keller")),
        "{found}"
    );
    let only_slack = client
        .tool("search", json!({"query": "pricing", "app": "Slack"}))
        .expect("search");
    assert!(
        only_slack["sessions"]
            .as_array()
            .expect("sessions")
            .iter()
            .all(|s| s["app"] == "Slack")
    );
    let in_range = client
        .tool(
            "search",
            json!({"query": "pricing", "from": "2026-03-03", "to": "2026-03-03"}),
        )
        .expect("search");
    assert!(in_range["lines"].as_i64().expect("lines") <= found["lines"].as_i64().expect("lines"));

    let around = client
        .tool(
            "context_at",
            json!({"time": "2026-03-02T09:40:30", "window": 300}),
        )
        .expect("context_at");
    assert!(
        around["screen"].to_string().contains("INV-2026-0412"),
        "{around}"
    );
    assert_eq!(around["truncated"], false);
    assert!(
        around["at"]
            .as_str()
            .expect("at")
            .starts_with("2026-03-02T09:40:30+01:00")
    );

    let week = client
        .tool(
            "timeline",
            json!({"from": "2026-03-02", "to": "2026-03-06", "limit": 200}),
        )
        .expect("timeline");
    let kinds: BTreeSet<&str> = week["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["kind"].as_str().expect("kind"))
        .collect();
    assert_eq!(kinds, BTreeSet::from(["activity"]));

    let page = client
        .tool("week", json!({"start": "2026-03-02"}))
        .expect("week");
    assert_eq!(page["days"].as_array().expect("days").len(), 7);
    let kind = &page["repeated_tasks"][0];
    let task_id = kind["task_ids"][0].as_i64().expect("task id");
    let task = client.tool("task", json!({"id": task_id})).expect("task");
    assert_eq!(task["label"], kind["label"]);
    let first_day = &task["first"].as_str().expect("first")[..10];
    let day = client.tool("day", json!({"date": first_day})).expect("day");
    assert!(
        day["activities"]
            .as_array()
            .expect("activities")
            .iter()
            .any(|a| a["task"] == task_id),
        "{day}"
    );
    assert_eq!(
        client.tool("week", json!({})).expect("week")["to"],
        "2026-03-07",
        "the default week ends today"
    );
    assert!(client.tool("task", json!({})).is_err());
    assert!(client.tool("day", json!({"date": "yesterday"})).is_err());
}

#[test]
fn only_save_routine_writes() {
    let (_db, mut client) = fixture_client("mcp-read-only");
    let reply = client.request("tools/list", json!({}));
    let writers: Vec<&str> = reply["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter(|tool| tool["annotations"]["readOnlyHint"] == false)
        .map(|tool| tool["name"].as_str().expect("name"))
        .collect();
    assert_eq!(writers, ["save_routine"]);
}

#[test]
fn a_saved_routine_comes_back_measured_and_listed() {
    let (_db, mut client) = fixture_client("mcp-routine");
    let week = client.tool("week", json!({})).expect("week");
    let ids = week["repeated_tasks"][0]["task_ids"].clone();
    assert!(ids.as_array().is_some_and(|ids| ids.len() >= 2), "{week}");
    let routine = json!({
        "title": "Send invoices",
        "task_ids": ids,
        "how": "Draft the email from the sheet.",
        "why": "Every weekday.",
        "hours_week": 40,
    });
    let saved = client.tool("save_routine", routine.clone()).expect("saved");
    let hours = saved["hours_week"].as_f64().expect("measured hours");
    assert!(hours > 0.0 && hours < 40.0, "{saved}");

    let listed = client.tool("routines", json!({})).expect("routines");
    assert_eq!(listed["routines"][0]["id"], saved["id"]);
    let refused = client.tool("save_routine", routine).expect_err("duplicate");
    assert!(
        refused.contains(&format!("routine {}", saved["id"])),
        "{refused}"
    );
    let missing = client
        .tool(
            "save_routine",
            json!({"title": "No evidence", "how": "-", "why": "-", "task_ids": [999_999]}),
        )
        .expect_err("unknown task");
    assert!(missing.contains("999999"), "{missing}");
}

#[test]
fn the_prompts_follow_the_work_patterns_switch() {
    let (_db, mut client) = fixture_client("mcp-prompt");
    let listed = client.request("prompts/list", json!({}));
    assert_eq!(listed["result"]["prompts"][0]["name"], "find_routines");
    let got = client.request("prompts/get", json!({"name": "find_routines"}));
    let text = got["result"]["messages"][0]["content"]["text"]
        .as_str()
        .expect("text");
    assert!(text.contains("save_routine"), "{text}");
    let skill = client.request(
        "prompts/get",
        json!({"name": "write_skill", "arguments": {"routine_id": "7"}}),
    );
    let text = skill["result"]["messages"][0]["content"]["text"]
        .as_str()
        .expect("text");
    assert!(
        text.contains("routine 7") && text.contains("check_skill"),
        "{text}"
    );

    client.settings.borrow_mut().mcp_workflows = false;
    let listed = client.request("prompts/list", json!({}));
    assert_eq!(listed["result"]["prompts"], json!([]));
    let got = client.request("prompts/get", json!({"name": "find_routines"}));
    assert!(got["error"].is_object(), "{got}");
}

#[test]
fn numbers_must_be_whole_numbers() {
    let (_db, mut client) = fixture_client("mcp-strict-ints");
    for (name, arguments) in [
        ("search", json!({"query": "invoice", "limit": 2.5})),
        ("search", json!({"query": "invoice", "limit": "2"})),
        ("search", json!({"query": "invoice", "limit": 0})),
        ("search", json!({"query": "invoice", "offset": -1})),
        ("search", json!({"query": "invoice", "offset": "1"})),
        ("task", json!({"id": 1.5})),
        ("task", json!({"id": "1"})),
        (
            "context_at",
            json!({"time": "2026-03-02T09:40", "window": "60"}),
        ),
    ] {
        let error = client.tool(name, arguments.clone()).expect_err("rejected");
        assert!(error.contains("must"), "{name} {arguments}: {error}");
    }
    assert!(
        client
            .tool(
                "search",
                json!({"query": "invoice", "limit": 2, "offset": 0})
            )
            .is_ok()
    );
    assert!(
        client
            .tool("search", json!({"query": "invoice", "limit": null}))
            .is_ok()
    );
}

#[test]
fn results_are_capped_and_paginated() {
    let (_db, mut client) = fixture_client("mcp-pages");

    let first = client
        .tool("search", json!({"query": "invoice", "limit": 2}))
        .expect("search");
    assert_eq!(first["lines"], 2);
    let next = first["next_offset"].as_i64().expect("a next page");
    let second = client
        .tool(
            "search",
            json!({"query": "invoice", "limit": 2, "offset": next}),
        )
        .expect("search");
    assert_ne!(first["sessions"], second["sessions"]);

    let mut seen = 0;
    let mut offset = 0;
    loop {
        let page = client
            .tool(
                "timeline",
                json!({"from": "2026-03-02", "to": "2026-03-06", "limit": 7, "offset": offset}),
            )
            .expect("timeline");
        let items = page["items"].as_array().expect("items").len();
        assert!(items <= 7);
        seen += items;
        match page["next_offset"].as_i64() {
            Some(next) => offset = next,
            None => break,
        }
    }
    let all = client
        .tool(
            "timeline",
            json!({"from": "2026-03-02", "to": "2026-03-06", "limit": 200}),
        )
        .expect("timeline");
    assert_eq!(seen, all["items"].as_array().expect("items").len());

    let huge = client
        .tool("search", json!({"query": "invoice", "limit": 100_000}))
        .expect("search");
    assert!(huge["lines"].as_i64().expect("lines") <= 200);
}

#[test]
fn excluded_apps_never_reach_the_output() {
    let (_db, mut client) = fixture_client("mcp-excluded");
    let before = client
        .tool("search", json!({"query": "invoice"}))
        .expect("search");
    assert!(before.to_string().contains("\"app\":\"Mail\""));

    client.settings.borrow_mut().excluded_apps = vec!["mail".into()];
    let after = client
        .tool("search", json!({"query": "invoice"}))
        .expect("search");
    assert!(!after.to_string().contains("\"app\":\"Mail\""), "{after}");
    let around = client
        .tool(
            "context_at",
            json!({"time": "2026-03-02T09:40:30", "window": 300}),
        )
        .expect("context_at");
    assert!(!around.to_string().contains("\"app\":\"Mail\""), "{around}");
    for (name, args) in [
        ("week", json!({"start": "2026-03-02"})),
        ("day", json!({"date": "2026-03-02"})),
    ] {
        let page = client.tool(name, args).expect(name);
        assert!(!page.to_string().contains("Mail"), "{page}");
    }
}

#[test]
fn private_data_is_redacted_in_every_answer() {
    let (_db, mut client) = fixture_client("mcp-redaction");
    let found = client
        .tool("search", json!({"query": "IBAN"}))
        .expect("search");
    let text = found.to_string();
    assert!(text.contains("<iban>"), "{text}");
    assert!(!text.contains("DE89 3704"), "{text}");
    let mail = client
        .tool("search", json!({"query": "Keller"}))
        .expect("search");
    assert!(!mail.to_string().contains("anna.keller@acme-gmbh.de"));
}

#[test]
fn rewind_and_routine_tools_switch_off_separately() {
    let (_db, mut client) = fixture_client("mcp-switches");

    client.settings.borrow_mut().mcp_rewind = false;
    let names = client.tool_names();
    assert!(
        !names.contains("search") && !names.contains("context_at") && !names.contains("timeline")
    );
    assert!(names.contains("routines"));
    assert!(client.tool("search", json!({"query": "invoice"})).is_err());
    assert!(client.tool("routines", json!({})).is_ok());

    *client.settings.borrow_mut() = Settings {
        mcp_workflows: false,
        ..Settings::default()
    };
    let names = client.tool_names();
    assert_eq!(names.len(), 3, "{names:?}");
    assert!(client.tool("routines", json!({})).is_err());
    assert!(client.tool("week", json!({})).is_err());
    assert!(client.tool("search", json!({"query": "invoice"})).is_ok());
}

fn p95(times: &mut [Duration]) -> Duration {
    times.sort();
    times[(times.len() * 95).div_ceil(100) - 1]
}

#[test]
#[cfg_attr(debug_assertions, ignore = "budget tests run in release mode")]
fn every_tool_stays_under_200_ms_p95() {
    let db = TempDb::new("budget-mcp");
    let mut store = db.open().expect("test setup");
    let spec = Spec {
        first_monday: WEEK_START,
        weeks: 13,
        noise_per_day: 250,
        frames: true,
    };
    let now = WEEK_START + 90 * DAY;
    load(&mut store, &generate(&spec), now).expect("test setup");
    store.refresh(local(now)).expect("test setup");
    let mut client = Client::new(store.reader().expect("test setup"));
    let week = client
        .tool("week", json!({"start": "2026-03-09"}))
        .expect("week");
    let task = week["repeated_tasks"][0]["task_ids"][0].clone();

    let calls: Vec<(&str, Value)> = vec![
        ("search", json!({"query": "pricing"})),
        (
            "search",
            json!({"query": "invoice", "from": "2026-03-10", "to": "2026-04-10"}),
        ),
        ("context_at", json!({"time": "2026-03-10T09:40:30"})),
        (
            "timeline",
            json!({"from": "2026-03-09", "to": "2026-03-13"}),
        ),
        ("week", json!({"start": "2026-03-09"})),
        ("day", json!({"date": "2026-03-10"})),
        ("task", json!({"id": task})),
        ("routines", json!({})),
    ];
    let mut worst = Duration::ZERO;
    for (name, args) in calls {
        let mut times = Vec::new();
        for _ in 0..50 {
            let started = Instant::now();
            let result = client.tool(name, args.clone());
            times.push(started.elapsed());
            assert!(result.is_ok(), "{name}: {result:?}");
        }
        let p = p95(&mut times);
        println!("mcp {name:15} {args:60} p95 {p:?}");
        worst = worst.max(p);
    }
    assert!(
        worst < MCP_P95,
        "worst tool p95 {worst:?} is over the {MCP_P95:?} budget"
    );
}

#[test]
#[cfg_attr(debug_assertions, ignore = "budget tests run in release mode")]
fn every_day_page_of_90_days_stays_under_8_kb_and_100_ms() {
    const DAY_BYTES: usize = 8_000;
    const DAY_TIME: Duration = Duration::from_millis(100);
    let db = TempDb::new("budget-day-pages");
    let mut store = db.open().expect("test setup");
    let spec = Spec {
        first_monday: WEEK_START,
        weeks: 13,
        noise_per_day: 250,
        frames: false,
    };
    let now = WEEK_START + 90 * DAY;
    load(&mut store, &generate(&spec), now).expect("test setup");
    store.refresh(local(now)).expect("test setup");
    let mut client = Client::new(store.reader().expect("test setup"));

    let (mut largest, mut slowest) = (0, Duration::ZERO);
    for day in 0..90 {
        let noon = WEEK_START + day * DAY + DAY / 2;
        let started = Instant::now();
        let page = client.tool("day", json!({"date": noon})).expect("day");
        slowest = slowest.max(started.elapsed());
        largest = largest.max(page.to_string().len());
    }
    println!("day pages over 90 days: largest {largest} bytes, slowest {slowest:?}");
    assert!(largest < DAY_BYTES, "a day page holds {largest} bytes");
    assert!(slowest < DAY_TIME, "a day page took {slowest:?}");
}
