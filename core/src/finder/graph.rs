//! §7.4 follows edges and §7.6 workflows.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use petgraph::Direction::{Incoming, Outgoing};
use petgraph::graph::{DiGraph, NodeIndex};

use super::episodes::Episode;
use super::steps::Step;
use super::tasks::Task;
use crate::time::day;

pub(crate) const FOLLOW_WINDOW_S: i64 = 600;
const ROUTINE_COUNT: usize = 5;
const ROUTINE_DAYS: usize = 3;
const MIN_TASKS: usize = 3;

pub(crate) struct Follow {
    pub from: usize,
    pub to: usize,
    pub count: usize,
    pub median_gap_s: i64,
    pub copies: usize,
    /// B shows an entity of A in another app and nothing was copied: it was typed again.
    pub retypes: usize,
    pub days: usize,
}

#[derive(Default)]
struct Edge {
    gaps: Vec<i64>,
    copies: usize,
    retypes: usize,
    days: HashSet<i64>,
}

/// B follows A when B is the next episode and starts within 10 minutes of A's end.
/// `copies` are (ts, app) of copy events; one counts for A's edge when it happens in A's app before B starts.
/// `entities` are the entity indexes of each episode, sorted.
pub(crate) fn follows(
    episodes: &[Episode],
    copies: &[(i64, &str)],
    entities: &[Vec<usize>],
    utc_offset_s: i32,
) -> Vec<Follow> {
    let mut edges: BTreeMap<(usize, usize), Edge> = BTreeMap::new();
    for (index, pair) in episodes.windows(2).enumerate() {
        let (a, b) = (&pair[0], &pair[1]);
        let gap = b.start - a.end;
        if a.step == b.step || !(0..=FOLLOW_WINDOW_S).contains(&gap) {
            continue;
        }
        let edge = edges.entry((a.step, b.step)).or_default();
        edge.gaps.push(gap);
        edge.days.insert(day(a.start, utc_offset_s));
        let first = copies.partition_point(|(ts, _)| *ts < a.start);
        let copied = copies[first..]
            .iter()
            .take_while(|(ts, _)| *ts < b.start)
            .filter(|(_, app)| *app == a.app)
            .count();
        edge.copies += copied;
        if copied == 0 && a.app != b.app && shares(&entities[index], &entities[index + 1]) {
            edge.retypes += 1;
        }
    }
    edges
        .into_iter()
        .map(|((from, to), mut edge)| {
            edge.gaps.sort_unstable();
            Follow {
                from,
                to,
                count: edge.gaps.len(),
                median_gap_s: edge.gaps[edge.gaps.len() / 2],
                copies: edge.copies,
                retypes: edge.retypes,
                days: edge.days.len(),
            }
        })
        .collect()
}

fn shares(a: &[usize], b: &[usize]) -> bool {
    a.iter().any(|x| b.binary_search(x).is_ok())
}

pub(crate) enum Basis {
    Tasks,
    Chain,
}

pub(crate) struct Workflow {
    pub variant: String,
    pub basis: Basis,
    pub sequence: Vec<usize>,
    pub tasks: Vec<usize>,
    /// Episode indexes in time order.
    pub episodes: Vec<usize>,
    pub count: usize,
}

pub(crate) fn variant_of(sequence: &[usize], steps: &[Step]) -> String {
    sequence
        .iter()
        .map(|&s| steps[s].key.as_str())
        .collect::<Vec<_>>()
        .join(" > ")
}

/// Tasks with the same step sequence (edit distance <= 1) form one workflow when there are 3+ of them.
/// A variant in `split` only takes identical sequences. Sets `Task::workflow`.
pub(crate) fn from_tasks(
    tasks: &mut [Task],
    steps: &[Step],
    split: &HashSet<String>,
) -> Vec<Workflow> {
    let mut by_sequence: BTreeMap<Vec<usize>, Vec<usize>> = BTreeMap::new();
    for (index, task) in tasks.iter().enumerate() {
        by_sequence.entry(task.seq.clone()).or_default().push(index);
    }
    let mut distinct: Vec<(Vec<usize>, Vec<usize>)> = by_sequence.into_iter().collect();
    distinct.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));

    let mut clusters: Vec<(Vec<usize>, usize, Vec<usize>)> = Vec::new();
    for (sequence, members) in distinct {
        let slot = clusters.iter().position(|(rep, limit, _)| {
            sequence.len().abs_diff(rep.len()) <= *limit && edit_distance(&sequence, rep) <= *limit
        });
        match slot {
            Some(slot) => clusters[slot].2.extend(members),
            None => {
                let limit = usize::from(!split.contains(&variant_of(&sequence, steps)));
                clusters.push((sequence, limit, members));
            }
        }
    }

    let mut workflows = Vec::new();
    for (sequence, _, mut members) in clusters {
        if members.len() < MIN_TASKS || sequence.len() < 2 {
            continue;
        }
        members.sort_unstable();
        let mut episodes: Vec<usize> = members
            .iter()
            .flat_map(|&c| tasks[c].episodes.iter().copied())
            .collect();
        episodes.sort_unstable();
        for &member in &members {
            tasks[member].workflow = Some(workflows.len());
        }
        workflows.push(Workflow {
            variant: variant_of(&sequence, steps),
            basis: Basis::Tasks,
            sequence,
            count: members.len(),
            tasks: members,
            episodes,
        });
    }
    workflows
}

fn edit_distance(a: &[usize], b: &[usize]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(x != y));
            diagonal = above;
        }
    }
    row[b.len()]
}

/// Linear chains of steps whose follows edges happen 5+ times on 3+ days. Branches and loops are skipped.
pub(crate) fn chains(follows: &[Follow]) -> Vec<Vec<usize>> {
    let mut graph: DiGraph<usize, ()> = DiGraph::new();
    let mut nodes: HashMap<usize, NodeIndex> = HashMap::new();
    for follow in follows
        .iter()
        .filter(|f| f.count >= ROUTINE_COUNT && f.days >= ROUTINE_DAYS)
    {
        let from = *nodes
            .entry(follow.from)
            .or_insert_with(|| graph.add_node(follow.from));
        let to = *nodes
            .entry(follow.to)
            .or_insert_with(|| graph.add_node(follow.to));
        graph.add_edge(from, to, ());
    }
    let degree = |node, direction| graph.neighbors_directed(node, direction).count();
    let mut visited: HashSet<NodeIndex> = HashSet::new();
    let mut found = Vec::new();
    for node in graph.node_indices() {
        let single_parent = graph
            .neighbors_directed(node, Incoming)
            .next()
            .filter(|_| degree(node, Incoming) == 1);
        let starts = single_parent.is_none_or(|parent| degree(parent, Outgoing) != 1);
        if !starts || degree(node, Outgoing) == 0 {
            continue;
        }
        let mut path = vec![node];
        visited.insert(node);
        let mut current = node;
        while degree(current, Outgoing) == 1 {
            let Some(next) = graph.neighbors_directed(current, Outgoing).next() else {
                break;
            };
            if degree(next, Incoming) != 1 || !visited.insert(next) {
                break;
            }
            path.push(next);
            current = next;
        }
        if path.len() >= 2 {
            found.push(path.into_iter().map(|n| graph[n]).collect::<Vec<usize>>());
        }
    }
    found.sort();
    found
}

/// Routine workflows: chains that no task workflow already contains.
pub(crate) fn from_chains(
    chains: Vec<Vec<usize>>,
    follows: &[Follow],
    episodes: &[Episode],
    steps: &[Step],
    covered: &[Workflow],
) -> Vec<Workflow> {
    let mut workflows = Vec::new();
    for chain in chains {
        if covered
            .iter()
            .any(|w| chain.iter().all(|s| w.sequence.contains(s)))
        {
            continue;
        }
        let links: HashSet<(usize, usize)> = chain.windows(2).map(|p| (p[0], p[1])).collect();
        let mut members: BTreeSet<usize> = BTreeSet::new();
        for (index, pair) in episodes.windows(2).enumerate() {
            let gap = pair[1].start - pair[0].end;
            if links.contains(&(pair[0].step, pair[1].step)) && (0..=FOLLOW_WINDOW_S).contains(&gap)
            {
                members.insert(index);
                members.insert(index + 1);
            }
        }
        let count = follows
            .iter()
            .filter(|f| links.contains(&(f.from, f.to)))
            .map(|f| f.count)
            .min()
            .unwrap_or(0);
        workflows.push(Workflow {
            variant: variant_of(&chain, steps),
            basis: Basis::Chain,
            sequence: chain,
            tasks: Vec::new(),
            episodes: members.into_iter().collect(),
            count,
        });
    }
    workflows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_distance_counts_single_changes() {
        assert_eq!(edit_distance(&[1, 2, 3], &[1, 2, 3]), 0);
        assert_eq!(edit_distance(&[1, 2, 3], &[1, 2]), 1);
        assert_eq!(edit_distance(&[1, 9, 3], &[1, 2, 3]), 1);
        assert_eq!(edit_distance(&[1, 2, 3], &[3, 2, 1]), 2);
    }

    #[test]
    fn chains_follow_unique_links_only() {
        let edge = |from, to| Follow {
            from,
            to,
            count: 5,
            median_gap_s: 10,
            copies: 0,
            retypes: 0,
            days: 3,
        };
        // 0 -> 1 -> 2 is a chain; 5 -> 6 and 5 -> 7 branch; 8 <-> 9 loop.
        let follows = [
            edge(0, 1),
            edge(1, 2),
            edge(5, 6),
            edge(5, 7),
            edge(8, 9),
            edge(9, 8),
        ];
        assert_eq!(chains(&follows), vec![vec![0, 1, 2]]);
    }
}
