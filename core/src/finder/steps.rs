//! §7.2 Steps: episodes grouped by object key, merged within a scope at Jaccard >= 0.5.

use std::collections::{BTreeMap, HashMap};

use super::episodes::Episode;
use super::find;
use crate::text::{jaccard, text_enum};

text_enum!(Category {
    Meeting => "meeting",
    Email => "email",
    Spreadsheet => "spreadsheet",
    Crm => "crm",
    Finance => "finance",
    Chat => "chat",
    Planning => "planning",
    Coding => "coding",
    Design => "design",
    Writing => "writing",
    Slides => "slides",
    Calendar => "calendar",
    Social => "social",
    Media => "media",
    Web => "web",
    Other => "other",
});

impl Category {
    pub fn prior(self) -> f64 {
        match self {
            Self::Crm => 0.75,
            Self::Spreadsheet | Self::Calendar => 0.7,
            Self::Finance => 0.65,
            Self::Email => 0.6,
            Self::Planning | Self::Coding => 0.55,
            Self::Web => 0.5,
            Self::Writing => 0.45,
            Self::Slides => 0.4,
            Self::Social | Self::Other => 0.35,
            Self::Chat => 0.3,
            Self::Design => 0.2,
            Self::Meeting => 0.1,
            Self::Media => 0.05,
        }
    }
}

/// First match wins. A needle starting with `^` must match the start of the key.
const RULES: &[(Category, &[&str])] = &[
    (
        Category::Meeting,
        &[
            "zoom",
            "teams",
            "meet.google",
            "facetime",
            "webex",
            "meeting",
        ],
    ),
    (Category::Calendar, &["calendar", "kalender", "fantastical"]),
    (Category::Email, &["mail", "outlook", "superhuman"]),
    (
        Category::Spreadsheet,
        &["spreadsheet", "excel", "xlsx", "csv", "airtable"],
    ),
    (
        Category::Crm,
        &["salesforce", "hubspot", "pipedrive", "zoho"],
    ),
    (
        Category::Finance,
        &[
            "lexoffice",
            "datev",
            "qonto",
            "stripe",
            "paypal",
            "quickbooks",
            "sevdesk",
        ],
    ),
    (
        Category::Chat,
        &["slack", "discord", "whatsapp", "telegram", "signal"],
    ),
    (
        Category::Planning,
        &[
            "linear",
            "jira",
            "trello",
            "asana",
            "todoist",
            "reminders",
            "clickup",
        ],
    ),
    (
        Category::Coding,
        &[
            "github",
            "gitlab",
            "stackoverflow",
            "terminal",
            "iterm",
            "xcode",
            "visual studio",
            "cargo",
        ],
    ),
    (Category::Design, &["figma", "sketch", "canva", "framer"]),
    (Category::Slides, &["keynote", "powerpoint", "presentation"]),
    (
        Category::Writing,
        &[
            "docs.google",
            "microsoft word",
            "docx",
            "pages",
            "notion",
            "obsidian",
            "textedit",
        ],
    ),
    (
        Category::Social,
        &[
            "twitter",
            "^x.com",
            "linkedin",
            "facebook",
            "instagram",
            "reddit",
            "mastodon",
        ],
    ),
    (
        Category::Media,
        &[
            "youtube", "netflix", "spotify", "twitch", "vimeo", "podcast",
        ],
    ),
];

pub(crate) fn categorize(key: &str, web: bool) -> Category {
    let key = key.to_lowercase();
    for (category, needles) in RULES {
        let hit = needles.iter().any(|needle| match needle.strip_prefix('^') {
            Some(prefix) => key.starts_with(prefix),
            None => key.contains(needle),
        });
        if hit {
            return *category;
        }
    }
    if web { Category::Web } else { Category::Other }
}

pub(crate) struct Step {
    pub key: String,
    pub name: String,
    pub category: Category,
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

/// Groups episodes into steps and sets `Episode::step`. `merges` maps a step key to the key it joins.
/// `names` overrides step names by key. Steps are ordered by key.
pub(crate) fn build(
    episodes: &mut [Episode],
    merges: &HashMap<String, String>,
    names: &HashMap<String, String>,
) -> Vec<Step> {
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

    let mut steps: Vec<Step> = groups
        .into_iter()
        .map(|group| make_step(group, episodes))
        .collect();
    steps.sort_by(|a, b| a.key.cmp(&b.key));
    dedupe_keys(&mut steps);

    for (from, to) in merges {
        let from_at = steps.iter().position(|s| &s.key == from);
        let to_at = steps.iter().position(|s| &s.key == to);
        if let (Some(from_at), Some(to_at)) = (from_at, to_at)
            && from_at != to_at
        {
            let moved = std::mem::take(&mut steps[from_at].episodes);
            steps[to_at].episodes.extend(moved);
            steps[to_at].episodes.sort_unstable();
        }
    }
    steps.retain(|s| !s.episodes.is_empty());
    for (index, step) in steps.iter_mut().enumerate() {
        if let Some(name) = names.get(&step.key) {
            step.name.clone_from(name);
        }
        for &episode in &step.episodes {
            episodes[episode].step = index;
        }
    }
    steps
}

/// Name and key from the tokens shared by at least half of the group's episodes.
fn make_step(group: Group, episodes: &[Episode]) -> Step {
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
    let first = group
        .members
        .iter()
        .flat_map(|m| m.episodes.iter().copied())
        .min()
        .unwrap_or(0);
    let category = categorize(main.map_or("", |m| m.key.as_str()), episodes[first].web);
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
        category,
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
