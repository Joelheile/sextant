//! §7.7 Commitments: promises in my own transcripts and open reminders, linked to tasks.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use super::entities::{EntityKind, find};
use super::episodes::{Episode, EventRow};
use super::tasks::Task;
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
    /// Index of the linked task.
    pub task: Option<usize>,
    pub overdue: bool,
}

pub(crate) fn is_promise(text: &str) -> bool {
    PROMISE.is_match(text)
}

/// Links a piece of text to a task: by anchor first, else by token overlap of 0.34 or more.
pub(crate) struct Linker {
    by_anchor: HashMap<String, usize>,
    task_tokens: Vec<(usize, Vec<String>)>,
    last_activity: HashMap<usize, i64>,
}

impl Linker {
    pub fn new(tasks: &[Task], episodes: &[Episode]) -> Self {
        Self {
            by_anchor: tasks
                .iter()
                .enumerate()
                .flat_map(|(t, task)| task.anchors.iter().map(move |a| (a.clone(), t)))
                .collect(),
            task_tokens: tasks
                .iter()
                .enumerate()
                .map(|(t, task)| (t, task_tokens(task, episodes)))
                .collect(),
            last_activity: tasks
                .iter()
                .enumerate()
                .map(|(t, task)| (t, task.last))
                .collect(),
        }
    }

    /// Index of the linked task.
    pub fn link(&self, text: &str) -> Option<usize> {
        let by_anchor = find(text)
            .into_iter()
            .filter(|(kind, _)| matches!(kind, EntityKind::Invoice | EntityKind::Ticket))
            .filter_map(|(_, value)| self.by_anchor.get(&value).copied())
            .min();
        if by_anchor.is_some() {
            return by_anchor;
        }
        let words = tokens(text);
        if words.is_empty() {
            return None;
        }
        let overlap = |task: &Vec<String>| {
            words.iter().filter(|w| task.contains(w)).count() as f64 / words.len() as f64
        };
        self.task_tokens
            .iter()
            .map(|(task, words)| (overlap(words), *task))
            .filter(|(score, _)| *score >= LINK_OVERLAP)
            .max_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)))
            .map(|(_, task)| task)
    }
}

/// `spoken` are (ts, text) of mic utterances. Reminders come from `reminder` events and count while not done.
pub(crate) fn build(
    events: &[EventRow],
    spoken: &[(i64, String)],
    linker: &Linker,
    now: i64,
) -> Vec<Commitment> {
    let link = |text: &str| linker.link(&PROMISE.replace_all(text, " "));
    let mut out = Vec::new();
    for (ts, text) in spoken.iter().filter(|(_, text)| is_promise(text)) {
        let task = link(text);
        let untouched = task
            .and_then(|t| linker.last_activity.get(&t))
            .is_some_and(|last| last <= ts);
        out.push(Commitment {
            ts: *ts,
            source: "utterance",
            text: text.clone(),
            due: None,
            task,
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
            task: link(&text),
            overdue: due.is_some_and(|due| due < now),
            due,
            text,
        });
    }
    out
}

fn task_tokens(task: &Task, episodes: &[Episode]) -> Vec<String> {
    let mut words: Vec<String> = task
        .episodes
        .iter()
        .flat_map(|&e| episodes[e].tokens.iter().cloned())
        .collect();
    for anchor in &task.anchors {
        words.extend(tokens(anchor));
    }
    words.sort_unstable();
    words.dedup();
    words
}
