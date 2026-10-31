use sextant_core::agent::{AgentName, Idea, parse_ideas};
use sextant_core::fixtures::*;

#[test]
fn brief_holds_the_step_graph_and_no_raw_text() {
    let (_db, store) = refreshed("brief");
    let brief = store.agent_brief().expect("brief");
    let value: serde_json::Value = serde_json::from_str(&brief).expect("brief is JSON");
    let workflows = value["workflows"].as_array().expect("workflows");
    assert_eq!(
        workflows.len(),
        store.workflows().expect("test setup").len()
    );
    assert!(
        workflows[0]["steps"]
            .as_array()
            .is_some_and(|s| s.len() >= 3)
    );
    assert!(
        workflows[0]["links"]
            .as_array()
            .is_some_and(|l| !l.is_empty())
    );
    for id in INVOICE_IDS {
        assert!(
            !brief.contains(id),
            "invoice number {id} leaked into the brief"
        );
    }
    assert!(!brief.contains('@'), "no email addresses");
}

#[test]
fn brief_holds_apps_not_window_title_words() {
    let db = TempDb::new("brief-words");
    let mut store = db.open().expect("test setup");
    let mut fixture = week();
    for event in &mut fixture.events {
        if event.json["app"] == "Google Chrome" {
            event.json["title"] = "Message from Jonas Mueller about INV-2026-0412".into();
        }
    }
    load(&mut store, &fixture, WEEK_NOW).expect("test setup");
    store.refresh(local(WEEK_NOW)).expect("test setup");
    let names: Vec<String> = store
        .workflows()
        .expect("test setup")
        .iter()
        .flat_map(|w| w.steps.iter().map(|s| s.name.to_lowercase()))
        .collect();
    assert!(
        names.iter().any(|name| name.contains("jonas")),
        "the fixture must put the name into a step name"
    );

    let brief = store.agent_brief().expect("brief").to_lowercase();
    for word in ["jonas", "mueller", "0412", "inv-", "message"] {
        assert!(!brief.contains(word), "{word} leaked into the brief");
    }
    assert!(brief.contains("\"app\":\"mail\""));
}

#[test]
fn parse_keeps_known_workflows_and_clamps_numbers() {
    let answer = r#"Here you go:
```json
{"ideas": [
  {"workflow_id": 7, "title": "  Draft invoice replies ", "how": "Claude reads the mail and fills the sheet.", "hours_week": 99, "confidence": 3},
  {"workflow_id": 8, "title": "Unknown", "how": "x", "hours_week": 1, "confidence": 0.5},
  {"workflow_id": 7, "title": "", "how": "no title", "hours_week": 1, "confidence": 0.5}
]}
```"#;
    let ideas = parse_ideas(answer, &[(7, 2.5)]).expect("parse");
    assert_eq!(
        ideas,
        [Idea {
            workflow_id: 7,
            title: "Draft invoice replies".into(),
            how: "Claude reads the mail and fills the sheet.".into(),
            hours_week: 2.5,
            confidence: 1.0,
        }]
    );
}

#[test]
fn parse_rejects_an_answer_without_json() {
    assert!(parse_ideas("I can't help with that.", &[(7, 1.0)]).is_err());
}

#[test]
fn saved_ideas_replace_the_last_run() {
    let (_db, mut store) = refreshed("ideas");
    let id = store.workflows().expect("test setup")[0].id;
    let idea = |title: &str, hours: f64| Idea {
        workflow_id: id,
        title: title.into(),
        how: "how".into(),
        hours_week: hours,
        confidence: 0.8,
    };
    store
        .save_ideas(WEEK_NOW - 60, AgentName::Codex, &[idea("old", 1.0)])
        .expect("save");
    store
        .save_ideas(
            WEEK_NOW,
            AgentName::Claude,
            &[idea("small", 0.5), idea("big", 2.0)],
        )
        .expect("save");
    let run = store.agent_run().expect("read").expect("a run");
    assert_eq!((run.ts, run.agent), (WEEK_NOW, AgentName::Claude));
    let titles: Vec<&str> = run.ideas.iter().map(|i| i.idea.title.as_str()).collect();
    assert_eq!(titles, ["big", "small"]);
    assert!(run.ideas.iter().all(|i| !i.workflow.is_empty()));
}

#[test]
fn agent_ideas_text_is_redacted() {
    let (_db, mut store) = refreshed("ideas-redacted");
    let id = store.workflows().expect("test setup")[0].id;
    let idea = Idea {
        workflow_id: id,
        title: "Mail anna.keller@acme-gmbh.de".into(),
        how: "Pay DE89370400440532013000 and card 4111111111111111".into(),
        hours_week: 1.0,
        confidence: 0.8,
    };
    store
        .save_ideas(100, AgentName::Claude, &[idea])
        .expect("save");
    let saved = store.agent_run().expect("run").expect("a run").ideas;
    let text = format!("{} {}", saved[0].idea.title, saved[0].idea.how);
    for secret in [
        "anna.keller@acme-gmbh.de",
        "DE89370400440532013000",
        "4111111111111111",
    ] {
        assert!(!text.contains(secret), "{secret} was stored: {text}");
    }
}
