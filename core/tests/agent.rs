use sextant_core::agent::{AgentName, Run};
use sextant_core::fixtures::*;

#[test]
fn a_run_replaces_the_last_one() {
    let db = TempDb::new("agent-run");
    let mut store = db.open().expect("test setup");
    assert_eq!(store.agent_run().expect("read"), None);
    store
        .save_run(WEEK_NOW - 60, AgentName::Codex)
        .expect("save");
    store.save_run(WEEK_NOW, AgentName::Claude).expect("save");
    assert_eq!(
        store.agent_run().expect("read"),
        Some(Run {
            ts: WEEK_NOW,
            agent: AgentName::Claude,
        })
    );
}
