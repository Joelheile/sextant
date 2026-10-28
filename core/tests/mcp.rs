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
use sextant_core::{Event, EventKind, Item, Settings, Source, Store, Writer};

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
            server: Server::new(reader, move || shared.borrow().clone(), || WEEK_NOW),
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
    store.refresh(WEEK_NOW).expect("test setup");
    let reader = store.reader().expect("test setup");
    (db, Client::new(reader))
}

fn first_workflow_id(client: &mut Client) -> i64 {
    let list = client.tool("list_workflows", json!({})).expect("list");
    list["workflows"][0]["id"].as_i64().expect("a workflow")
}

#[test]
fn the_handshake_lists_seven_tools() {
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
            "context_at",
            "get_skill",
            "get_workflow",
            "list_workflows",
            "report_handoff",
            "search",
            "timeline"
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
fn the_timeline_lists_upcoming_meetings_at_their_real_time() {
    let db = TempDb::new("mcp-upcoming");
    let mut store = db.open().expect("test setup");
    load(&mut store, &week(), WEEK_NOW).expect("test setup");
    let start = WEEK_NOW + 2 * DAY;
    let mut writer = Writer::new();
    writer.push(Item::Event(Event {
        ts: start,
        kind: EventKind::Calendar,
        source: Source::Backfill,
        json: json!({"title": "Planning soon", "end_ts": start + 3600}),
    }));
    writer.flush(&mut store, WEEK_NOW).expect("test setup");
    store.refresh(WEEK_NOW).expect("test setup");
    let mut client = Client::new(store.reader().expect("test setup"));

    let ahead = client
        .tool(
            "timeline",
            json!({"from": "2026-03-08", "to": "2026-03-10"}),
        )
        .expect("timeline");
    let items = ahead["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{ahead}");
    assert_eq!(items[0]["what"], "Planning soon");
    assert!(
        items[0]["start"]
            .as_str()
            .expect("start")
            .starts_with("2026-03-09T08:00")
    );
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
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "list_workflows", "arguments": {}}}),
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
    assert!(
        kinds.contains("episode") && kinds.contains("meeting"),
        "{kinds:?}"
    );
    assert!(week["items"].to_string().contains("Weekly sync"));

    let list = client.tool("list_workflows", json!({})).expect("list");
    assert!(list["total"].as_i64().expect("total") >= 1);
    assert!(list["hours_back"].as_f64().expect("hours") >= 0.0);
    let id = list["workflows"][0]["id"].as_i64().expect("id");
    assert!(
        list["workflows"][0]["steps"]
            .as_array()
            .expect("steps")
            .len()
            >= 2
    );

    let detail = client
        .tool("get_workflow", json!({"id": id}))
        .expect("get_workflow");
    assert_eq!(detail["id"], id);
    assert!(detail["steps"][0]["name"].is_string());
    assert!(detail["follows"].is_array() && detail["examples"].is_array());

    let skill = client
        .tool("get_skill", json!({"id": id}))
        .expect("get_skill");
    let skill = skill.as_str().expect("markdown");
    assert!(
        skill.starts_with("---") && skill.contains("## Steps"),
        "{skill}"
    );

    let first = client
        .tool(
            "report_handoff",
            json!({"id": id, "status": "started", "note": "Drafts the invoice"}),
        )
        .expect("report_handoff");
    let second = client
        .tool("report_handoff", json!({"id": id, "status": "done"}))
        .expect("report_handoff");
    assert_eq!(first["recorded"], true);
    assert_eq!(
        second["handoff_id"].as_i64(),
        first["handoff_id"].as_i64().map(|n| n + 1)
    );
    assert!(
        client
            .tool("report_handoff", json!({"id": 999_999, "status": "done"}))
            .is_err()
    );
    assert!(client.tool("get_workflow", json!({"id": 999_999})).is_err());
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
    let one = client
        .tool("list_workflows", json!({"limit": 1}))
        .expect("list");
    assert_eq!(one["workflows"].as_array().expect("rows").len(), 1);
}

#[test]
fn excluded_apps_never_reach_the_output() {
    let (_db, mut client) = fixture_client("mcp-excluded");
    let before = client
        .tool("search", json!({"query": "invoice"}))
        .expect("search");
    assert!(before.to_string().contains("\"app\":\"Mail\""));
    let ids = |client: &mut Client| -> BTreeSet<i64> {
        let list = client
            .tool("list_workflows", json!({"limit": 200}))
            .expect("list");
        list["workflows"]
            .as_array()
            .expect("rows")
            .iter()
            .map(|w| w["id"].as_i64().expect("id"))
            .collect()
    };
    let all_ids = ids(&mut client);

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

    let visible = ids(&mut client);
    let hidden: Vec<i64> = all_ids.difference(&visible).copied().collect();
    assert!(!hidden.is_empty(), "the invoice workflow has a Mail step");
    for id in hidden {
        assert!(client.tool("get_workflow", json!({"id": id})).is_err());
        assert!(client.tool("get_skill", json!({"id": id})).is_err());
        assert!(
            client
                .tool("report_handoff", json!({"id": id, "status": "done"}))
                .is_err()
        );
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
fn rewind_and_workflow_tools_switch_off_separately() {
    let (_db, mut client) = fixture_client("mcp-switches");
    let id = first_workflow_id(&mut client);

    client.settings.borrow_mut().mcp_rewind = false;
    let names = client.tool_names();
    assert!(
        !names.contains("search") && !names.contains("context_at") && !names.contains("timeline")
    );
    assert!(names.contains("list_workflows"));
    assert!(client.tool("search", json!({"query": "invoice"})).is_err());
    assert!(client.tool("get_workflow", json!({"id": id})).is_ok());

    *client.settings.borrow_mut() = Settings {
        mcp_workflows: false,
        ..Settings::default()
    };
    let names = client.tool_names();
    assert_eq!(names.len(), 3, "{names:?}");
    assert!(client.tool("list_workflows", json!({})).is_err());
    assert!(
        client
            .tool("report_handoff", json!({"id": id, "status": "done"}))
            .is_err()
    );
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
    store.refresh(now).expect("test setup");
    let mut client = Client::new(store.reader().expect("test setup"));
    let id = first_workflow_id(&mut client);

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
        ("list_workflows", json!({})),
        ("get_workflow", json!({"id": id})),
        ("get_skill", json!({"id": id})),
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
    let writing = (0..50)
        .map(|_| {
            let started = Instant::now();
            client
                .tool("report_handoff", json!({"id": id, "status": "started"}))
                .expect("handoff");
            started.elapsed()
        })
        .collect::<Vec<_>>();
    let mut writing = writing;
    let p = p95(&mut writing);
    println!("mcp {:15} {:60} p95 {p:?}", "report_handoff", "");
    worst = worst.max(p);
    assert!(
        worst < MCP_P95,
        "worst tool p95 {worst:?} is over the {MCP_P95:?} budget"
    );
}
