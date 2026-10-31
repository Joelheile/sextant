//! `sextant mcp` as a real child process: JSON-RPC over its stdin and stdout, against a fixture
//! database in a temp folder. The key comes from SEXTANT_DEV_KEY, so the Keychain stays untouched.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};
use sextant_core::Store;
use sextant_core::fixtures::{TEST_SECRET, WEEK_NOW, load, local, week};

const KEY_HEX: &str = "0707070707070707070707070707070707070707070707070707070707070707";
const KEY: [u8; 32] = [7; 32];

fn temp(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sextant-stdio-{}-{label}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("test setup");
    dir
}

fn spawn(dir: &PathBuf) -> Child {
    Command::new(env!("CARGO_BIN_EXE_sextant"))
        .arg("mcp")
        .env("SEXTANT_HOME", dir)
        .env("SEXTANT_DEV_KEY", KEY_HEX)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("sextant starts")
}

struct Session {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    id: i64,
}

impl Session {
    fn call(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let line = json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params});
        writeln!(self.stdin, "{line}").expect("write request");
        let mut reply = String::new();
        self.stdout.read_line(&mut reply).expect("read reply");
        let reply: Value = serde_json::from_str(&reply).expect("json reply");
        assert_eq!(reply["id"], self.id);
        reply
    }
}

#[test]
fn serves_every_tool_group_over_stdio() {
    let dir = temp("serve");
    let mut store = Store::open(&dir.join("sextant.db"), &KEY, &TEST_SECRET).expect("test setup");
    load(&mut store, &week(), WEEK_NOW).expect("test setup");
    store.refresh(local(WEEK_NOW)).expect("test setup");
    drop(store);

    let mut child = spawn(&dir);
    let mut session = Session {
        stdin: child.stdin.take().expect("stdin"),
        stdout: BufReader::new(child.stdout.take().expect("stdout")),
        id: 0,
    };

    let init = session.call("initialize", json!({"protocolVersion": "2025-06-18"}));
    assert_eq!(init["result"]["serverInfo"]["name"], "sextant");
    writeln!(
        session.stdin,
        "{}",
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
    )
    .expect("notify");

    let tools = session.call("tools/list", json!({}));
    assert_eq!(tools["result"]["tools"].as_array().expect("tools").len(), 7);

    let list = session.call(
        "tools/call",
        json!({"name": "list_workflows", "arguments": {}}),
    );
    assert_eq!(list["result"]["isError"], false);
    let rows: Value =
        serde_json::from_str(list["result"]["content"][0]["text"].as_str().expect("text"))
            .expect("json");
    assert!(rows["total"].as_i64().expect("total") >= 1);

    let found = session.call(
        "tools/call",
        json!({"name": "search", "arguments": {"query": "pricing"}}),
    );
    assert_eq!(found["result"]["isError"], false);
    assert!(
        found["result"]["content"][0]["text"]
            .as_str()
            .expect("text")
            .contains("Anna Keller")
    );

    fs::write(dir.join("settings.json"), r#"{"mcp_rewind": false}"#).expect("switch off");
    let blocked = session.call(
        "tools/call",
        json!({"name": "search", "arguments": {"query": "pricing"}}),
    );
    assert_eq!(
        blocked["result"]["isError"], true,
        "the running server re-reads settings.json"
    );

    drop(session.stdin);
    assert!(
        child.wait().expect("exits").success(),
        "the server stops when stdin closes"
    );
    fs::remove_dir_all(dir).expect("cleanup");
}

#[test]
fn explains_when_there_is_no_database_yet() {
    let dir = temp("empty");
    let mut child = spawn(&dir);
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("exits");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("open the app once first"));
    fs::remove_dir_all(dir).expect("cleanup");
}
