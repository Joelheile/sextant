//! §7.7 Commitments: promises in my own transcripts and open reminders, linked to cases.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use super::entities::{Case, Entity, EntityKind, find};
use super::episodes::{Episode, EventRow};
use crate::text::{pattern, tokens};
use crate::time::DAY;

const LINK_OVERLAP: f64 = 0.34;
const STALE_AFTER_S: i64 = 2 * DAY;

static PROMISE: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"(?i)\b(?:i['’]ll|i will|let me|ich werde|ich schicke|ich sende|mach ich)\b")
});

pub(crate) struct Commitment {
    pub ts: i64,
    pub source: &'static str,
    pub text: String,
    pub due: Option<i64>,
    /// Entity index of the linked case.
    pub entity: Option<usize>,
    pub overdue: bool,
}

pub(crate) fn is_promise(text: &str) -> bool {
    PROMISE.is_match(text)
}

/// `spoken` are (ts, text) of mic utterances. Reminders come from `reminder` events and count while not done.
pub(crate) fn build(
    events: &[EventRow],
    spoken: &[(i64, String)],
    entities: &[Entity],
    cases: &[Case],
    episodes: &[Episode],
    now: i64,
) -> Vec<Commitment> {
    let by_value: HashMap<(EntityKind, &str), usize> = cases
        .iter()
        .map(|c| {
            (
                (entities[c.entity].kind, entities[c.entity].value.as_str()),
                c.entity,
            )
        })
        .collect();
    let case_tokens: Vec<(usize, Vec<String>)> = cases
        .iter()
        .map(|c| (c.entity, case_tokens(c, entities, episodes)))
        .collect();
    let last_activity: HashMap<usize, i64> = cases.iter().map(|c| (c.entity, c.last)).collect();

    let link = |text: &str| -> Option<usize> {
        let by_entity = find(text)
            .iter()
            .filter_map(|(kind, value)| by_value.get(&(*kind, value.as_str())).copied())
            .min();
        if by_entity.is_some() {
            return by_entity;
        }
        let words = tokens(&PROMISE.replace_all(text, " "));
        if words.is_empty() {
            return None;
        }
        let overlap = |case: &Vec<String>| {
            words.iter().filter(|w| case.contains(w)).count() as f64 / words.len() as f64
        };
        case_tokens
            .iter()
            .map(|(entity, case)| (overlap(case), *entity))
            .filter(|(score, _)| *score >= LINK_OVERLAP)
            .max_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)))
            .map(|(_, entity)| entity)
    };

    let mut out = Vec::new();
    for (ts, text) in spoken.iter().filter(|(_, text)| is_promise(text)) {
        let entity = link(text);
        let untouched = entity
            .and_then(|e| last_activity.get(&e))
            .is_some_and(|last| last <= ts);
        out.push(Commitment {
            ts: *ts,
            source: "utterance",
            text: text.clone(),
            due: None,
            entity,
            overdue: untouched && now - ts > STALE_AFTER_S,
        });
    }
    for event in events
        .iter()
        .filter(|e| e.kind == "reminder" && !e.payload.done)
    {
        let due = event.payload.due_ts;
        let text = event.payload.title.clone();
        out.push(Commitment {
            ts: event.ts,
            source: "reminder",
            entity: link(&text),
            overdue: due.is_some_and(|due| due < now),
            due,
            text,
        });
    }
    out
}

fn case_tokens(case: &Case, entities: &[Entity], episodes: &[Episode]) -> Vec<String> {
    let mut words: Vec<String> = case
        .episodes
        .iter()
        .flat_map(|&e| episodes[e].tokens.iter().cloned())
        .collect();
    words.extend(tokens(&entities[case.entity].value));
    words.sort_unstable();
    words.dedup();
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promises_match_english_and_german_phrases() {
        for text in [
            "I'll send it tomorrow",
            "Let me check",
            "Ich werde das morgen schicken",
            "Mach ich",
        ] {
            assert!(is_promise(text), "{text}");
        }
        assert!(!is_promise("She said the price is high"));
    }
}
