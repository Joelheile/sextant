//! Steps: activities grouped by object key, merged within a scope at Jaccard >= 0.5.

use std::collections::{BTreeMap, HashMap};

use super::activities::Activity;
use super::find;
use crate::text::jaccard;

pub(crate) struct Step {
    pub key: String,
    pub name: String,
    pub activities: Vec<usize>,
}

struct KeyInfo {
    key: String,
    tokens: Vec<String>,
    activities: Vec<usize>,
}

struct Group {
    scope: String,
    members: Vec<KeyInfo>,
}

/// Groups activities into steps and sets `Activity::step`. Steps are ordered by key.
pub(crate) fn build(activities: &mut [Activity]) -> Vec<Step> {
    let mut by_scope: BTreeMap<&str, BTreeMap<&str, KeyInfo>> = BTreeMap::new();
    for (index, activity) in activities.iter().enumerate() {
        let info = by_scope
            .entry(&activity.scope)
            .or_default()
            .entry(&activity.key)
            .or_insert_with(|| KeyInfo {
                key: activity.key.clone(),
                tokens: activity.tokens.clone(),
                activities: Vec::new(),
            });
        info.activities.push(index);
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

    steps.retain(|s| !s.activities.is_empty());
    for (index, step) in steps.iter_mut().enumerate() {
        for &activity in &step.activities {
            activities[activity].step = index;
        }
    }
    steps
}

/// Name and key from the tokens shared by at least half of the group's activities.
fn make_step(group: Group) -> Step {
    let total: usize = group.members.iter().map(|m| m.activities.len()).sum();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for member in &group.members {
        for token in &member.tokens {
            *counts.entry(token).or_default() += member.activities.len();
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
        .max_by_key(|m| (m.activities.len(), std::cmp::Reverse(&m.key)));
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
    let mut eps: Vec<usize> = group
        .members
        .into_iter()
        .flat_map(|m| m.activities)
        .collect();
    eps.sort_unstable();
    Step {
        key: format!("{}|{words}", group.scope),
        name,
        activities: eps,
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
