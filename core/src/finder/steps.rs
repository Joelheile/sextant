//! §7.2 Steps: episodes grouped by object key, merged within a scope at Jaccard >= 0.5.

use std::collections::{BTreeMap, HashMap};

use super::episodes::Episode;
use super::find;
use crate::text::jaccard;

pub(crate) struct Step {
    pub key: String,
    pub name: String,
    pub episodes: Vec<usize>,
}

struct KeyInfo {
    key: String,
    tokens: Vec<String>,
    episodes: Vec<usize>,
}

struct Group {
    scope: String,
    members: Vec<KeyInfo>,
}

/// Groups episodes into steps and sets `Episode::step`. Steps are ordered by key.
pub(crate) fn build(episodes: &mut [Episode]) -> Vec<Step> {
    let mut by_scope: BTreeMap<&str, BTreeMap<&str, KeyInfo>> = BTreeMap::new();
    for (index, episode) in episodes.iter().enumerate() {
        let info = by_scope
            .entry(&episode.scope)
            .or_default()
            .entry(&episode.key)
            .or_insert_with(|| KeyInfo {
                key: episode.key.clone(),
                tokens: episode.tokens.clone(),
                episodes: Vec::new(),
            });
        info.episodes.push(index);
    }

    let mut groups: Vec<Group> = Vec::new();
    for (scope, keys) in by_scope {
        let keys: Vec<KeyInfo> = keys.into_values().collect();
        let mut parent: Vec<usize> = (0..keys.len()).collect();
        for i in 0..keys.len() {
            for j in i + 1..keys.len() {
                if jaccard(&keys[i].tokens, &keys[j].tokens) >= 0.5 {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    parent[b.max(a)] = a.min(b);
                }
            }
        }
        let mut roots: BTreeMap<usize, Group> = BTreeMap::new();
        for (i, info) in keys.into_iter().enumerate() {
            let root = find(&mut parent, i);
            roots
                .entry(root)
                .or_insert_with(|| Group {
                    scope: scope.to_string(),
                    members: Vec::new(),
                })
                .members
                .push(info);
        }
        groups.extend(roots.into_values());
    }

    let mut steps: Vec<Step> = groups.into_iter().map(make_step).collect();
    steps.sort_by(|a, b| a.key.cmp(&b.key));
    dedupe_keys(&mut steps);

    steps.retain(|s| !s.episodes.is_empty());
    for (index, step) in steps.iter_mut().enumerate() {
        for &episode in &step.episodes {
            episodes[episode].step = index;
        }
    }
    steps
}

/// Name and key from the tokens shared by at least half of the group's episodes.
fn make_step(group: Group) -> Step {
    let total: usize = group.members.iter().map(|m| m.episodes.len()).sum();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for member in &group.members {
        for token in &member.tokens {
            *counts.entry(token).or_default() += member.episodes.len();
        }
    }
    let mut shared: Vec<&str> = counts
        .iter()
        .filter(|(_, n)| **n * 2 >= total)
        .map(|(t, _)| *t)
        .collect();
    let main = group
        .members
        .iter()
        .max_by_key(|m| (m.episodes.len(), std::cmp::Reverse(&m.key)));
    if shared.is_empty() {
        shared = main
            .map(|m| m.tokens.iter().map(String::as_str).collect())
            .unwrap_or_default();
        shared.sort_unstable();
    }
    let words = shared.join(" ");
    let name = if words.is_empty() {
        group.scope.clone()
    } else {
        format!("{}: {words}", group.scope)
    };
    let mut eps: Vec<usize> = group.members.into_iter().flat_map(|m| m.episodes).collect();
    eps.sort_unstable();
    Step {
        key: format!("{}|{words}", group.scope),
        name,
        episodes: eps,
    }
}

fn dedupe_keys(steps: &mut [Step]) {
    let mut seen: HashMap<String, usize> = HashMap::new();
    for step in steps {
        let count = seen.entry(step.key.clone()).or_default();
        *count += 1;
        if *count > 1 {
            step.key = format!("{}#{count}", step.key);
        }
    }
}
