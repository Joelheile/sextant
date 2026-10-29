//! §7.8 to §7.10 Scores, kind and action. Every number carries its reason.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::commitments::Commitment;
use super::entities::Case;
use super::episodes::Episode;
use super::graph::{Basis, FOLLOW_WINDOW_S, Follow, Workflow};
use super::judgments::Judgment;
use super::steps::{Category, Step, text_enum};
use crate::corrections::Overrides;
use crate::time::DAY;

text_enum!(Kind {
    Friction => "friction",
    PromiseGap => "promise_gap",
    Routine => "routine",
    LongTail => "long_tail",
});

text_enum!(Action {
    Automate => "automate",
    Assist => "assist",
    Protect => "protect",
    Ignore => "ignore",
    Watch => "watch",
});

text_enum!(Enjoyment {
    Hate => "hate",
    Dislike => "dislike",
    Neutral => "neutral",
    Unknown => "unknown",
    Like => "like",
    Love => "love",
});

impl Enjoyment {
    pub fn desire(self) -> f64 {
        match self {
            Self::Hate => 1.0,
            Self::Dislike => 0.9,
            Self::Neutral => 0.7,
            Self::Unknown => 0.6,
            Self::Like => 0.35,
            Self::Love => 0.0,
        }
    }
}

pub(crate) struct StepScore {
    pub minutes: f64,
    pub days: usize,
    pub potential: f64,
    pub reasons: Vec<String>,
}

pub(crate) struct Scored {
    pub name: String,
    pub hours_week: f64,
    pub potential: f64,
    pub desire: f64,
    pub benefit: f64,
    pub procrastination: f64,
    pub kind: Kind,
    pub action: Action,
    pub first_step: String,
    pub copies: Vec<(usize, usize, usize)>,
    pub enjoyment: Enjoyment,
    pub client: bool,
    pub hidden: bool,
    pub rough: bool,
    pub reasons: Vec<String>,
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or(0.0)
}

/// Step potential = prior x length factor x (0.6 + 0.4 x stability) x repetition.
/// Length is the median episode, stability the share of episodes within 0.5x to 2x of it.
/// The prior is the on-device model's judgment of the step when there is one, else the
/// category's.
pub(crate) fn score_steps(
    steps: &[Step],
    episodes: &[Episode],
    judged: &HashMap<String, Judgment>,
    utc_offset_s: i32,
) -> Vec<StepScore> {
    steps
        .iter()
        .map(|step| {
            let mut seconds: Vec<f64> = step.episodes.iter().map(|&e| episodes[e].seconds() as f64).collect();
            let minutes = seconds.iter().sum::<f64>() / 60.0;
            let typical = median(&mut seconds);
            let stable = seconds.iter().filter(|s| **s >= typical * 0.5 && **s <= typical * 2.0).count();
            let stability = stable as f64 / seconds.len().max(1) as f64;
            let days: HashSet<i64> = step.episodes.iter().map(|&e| crate::time::day(episodes[e].start, utc_offset_s)).collect();
            let typical_min = typical / 60.0;
            let length = if typical_min <= 60.0 { 1.0 } else if typical_min <= 180.0 { 0.85 } else { 0.75 };
            let repetition = if days.len() >= 2 { 1.0 } else { 0.8 };
            let (prior, basis) = match judged.get(&step.key) {
                Some(judgment) => (judgment.share, format!("model ({})", judgment.reason)),
                None => (step.category.prior(), step.category.as_str().to_string()),
            };
            let potential = prior * length * (0.6 + 0.4 * stability) * repetition;
            let reasons = vec![format!(
                "potential {potential:.2} = {basis} prior {prior:.2} x length {length:.2} ({typical_min:.0} min typical) x (0.6 + 0.4 x stability {stability:.2}: {stable} of {} episodes near typical) x repetition {repetition:.1} ({} days)",
                seconds.len(),
                days.len(),
            )];
            StepScore { minutes, days: days.len(), potential, reasons }
        })
        .collect()
}

pub(crate) struct Context<'a> {
    pub episodes: &'a [Episode],
    pub steps: &'a [Step],
    pub step_scores: &'a [StepScore],
    pub follows: &'a [Follow],
    pub cases: &'a [Case],
    pub commitments: &'a [Commitment],
    pub overrides: &'a Overrides,
    pub observed_days: usize,
    pub now: i64,
}

pub(crate) fn score_workflow(workflow: &Workflow, cx: &Context) -> Scored {
    let mut reasons = Vec::new();
    let mut minutes_by_step: BTreeMap<usize, f64> = BTreeMap::new();
    for &e in &workflow.episodes {
        *minutes_by_step.entry(cx.episodes[e].step).or_default() +=
            cx.episodes[e].seconds() as f64 / 60.0;
    }
    let minutes: f64 = minutes_by_step.values().sum();
    let hours_week = minutes / 60.0 / cx.observed_days.max(1) as f64 * 5.0;
    reasons.push(format!(
        "hours_week {hours_week:.2}: {minutes:.0} min over {} observed days, scaled to a 5-day week",
        cx.observed_days
    ));

    let copies = copy_points(workflow, cx);
    let copy_total: usize = copies.iter().map(|(_, _, n)| n).sum();
    let switches = workflow
        .episodes
        .iter()
        .filter(|&&e| {
            let next = e + 1;
            workflow.episodes.binary_search(&next).is_ok()
                && cx.episodes[e].step != cx.episodes[next].step
                && (0..=FOLLOW_WINDOW_S).contains(&(cx.episodes[next].start - cx.episodes[e].end))
        })
        .count();
    let switches_per_hour = if minutes > 0.0 {
        switches as f64 / (minutes / 60.0)
    } else {
        0.0
    };

    let enjoyment = enjoyment_of(workflow, cx);
    let not_automatable = cx.overrides.flag(&workflow.variant, "not_automatable");
    let weighted: f64 = minutes_by_step
        .iter()
        .map(|(s, m)| cx.step_scores[*s].potential * m)
        .sum::<f64>()
        / minutes.max(f64::EPSILON);
    let bonus = if copy_total > 0 { 0.1 } else { 0.0 };
    let mut potential = (weighted + bonus).min(0.95);
    reasons.push(format!(
        "potential {potential:.2}: time-weighted mean {weighted:.2} of {} steps{}",
        minutes_by_step.len(),
        if copy_total > 0 {
            ", +0.10 because data is copied by hand"
        } else {
            ""
        },
    ));
    for (from, to, n) in &copies {
        reasons.push(format!(
            "copy: {n} times from {} to {}",
            cx.steps[*from].name, cx.steps[*to].name
        ));
    }
    if not_automatable {
        potential = 0.0;
        reasons.push("potential 0.00: you said this cannot be automated".into());
    }

    let desire = enjoyment.desire();
    reasons.push(format!(
        "desire {desire:.2}: enjoyment {}",
        enjoyment.as_str()
    ));

    let top_category = category_minutes(&minutes_by_step, cx.steps);
    let assist = top_category.get(&Category::Meeting).copied().unwrap_or(0.0) * 2.0 >= minutes;
    let benefit = hours_week * potential * desire * if assist { 0.5 } else { 1.0 };
    reasons.push(format!(
        "benefit {benefit:.2} = {hours_week:.2} h x {potential:.2} x {desire:.2}{}",
        if assist { " x 0.5 (assist)" } else { "" }
    ));

    let (procrastination, detail) = procrastination(workflow, cx);
    reasons.push(format!("procrastination {procrastination:.2}: {detail}"));

    let kind = if copy_total > 0 || switches_per_hour >= 12.0 {
        reasons.push(format!(
            "kind friction: {copy_total} hand copies, {switches_per_hour:.1} app switches per hour"
        ));
        Kind::Friction
    } else if procrastination >= 0.4 {
        reasons.push("kind promise_gap: procrastination is 0.40 or more".into());
        Kind::PromiseGap
    } else if hours_week < 0.5 {
        reasons.push(format!(
            "kind long_tail: {hours_week:.2} h per week is under 0.5"
        ));
        Kind::LongTail
    } else {
        reasons.push(format!("kind routine: {} repetitions", workflow.count));
        Kind::Routine
    };

    let media = top_category.get(&Category::Media).copied().unwrap_or(0.0) * 2.0 >= minutes;
    let action = if enjoyment == Enjoyment::Love {
        reasons.push("action protect: you love this work".into());
        Action::Protect
    } else if media || not_automatable {
        reasons.push(format!(
            "action ignore: {}",
            if media {
                "mostly media"
            } else {
                "cannot be automated"
            }
        ));
        Action::Ignore
    } else if assist {
        reasons.push("action assist: mostly meetings".into());
        Action::Assist
    } else if potential >= 0.6 && hours_week >= 0.5 {
        reasons.push("action automate: potential 0.60 or more and 0.5 h per week or more".into());
        Action::Automate
    } else {
        reasons.push("action watch: below the automate thresholds".into());
        Action::Watch
    };

    let best = minutes_by_step
        .iter()
        .max_by(|a, b| {
            (cx.step_scores[*a.0].potential * a.1)
                .total_cmp(&(cx.step_scores[*b.0].potential * b.1))
                .then(b.0.cmp(a.0))
        })
        .map_or(0, |(s, _)| *s);
    reasons.push(format!(
        "first step: {} has the highest potential x time",
        cx.steps[best].name
    ));

    let client = cx.overrides.flag(&workflow.variant, "client");
    if client {
        reasons.push("client: you said this is client work".into());
    }
    let rough = workflow.episodes.iter().all(|&e| !cx.episodes[e].live);
    if rough {
        reasons.push("rough: built from backfill only, no live data yet".into());
    }
    Scored {
        name: cx
            .overrides
            .get(&workflow.variant, "name")
            .map_or_else(|| default_name(workflow, cx.steps), str::to_string),
        hours_week,
        potential,
        desire,
        benefit,
        procrastination,
        kind,
        action,
        first_step: cx.steps[best].name.clone(),
        copies,
        enjoyment,
        client,
        hidden: cx.overrides.flag(&workflow.variant, "hidden"),
        rough,
        reasons,
    }
}

/// "Mail > Excel > app.lexoffice.de": the scope of each step, repeats collapsed.
fn default_name(workflow: &Workflow, steps: &[Step]) -> String {
    let mut scopes: Vec<&str> = Vec::new();
    for &s in &workflow.sequence {
        let scope = steps[s].name.split(": ").next().unwrap_or("");
        if scopes.last() != Some(&scope) {
            scopes.push(scope);
        }
    }
    scopes.join(" > ")
}

fn category_minutes(
    minutes_by_step: &BTreeMap<usize, f64>,
    steps: &[Step],
) -> HashMap<Category, f64> {
    let mut out: HashMap<Category, f64> = HashMap::new();
    for (s, m) in minutes_by_step {
        *out.entry(steps[*s].category).or_default() += m;
    }
    out
}

/// The workflow's own override, else the first step in the sequence that has one.
fn enjoyment_of(workflow: &Workflow, cx: &Context) -> Enjoyment {
    let of = |key: &str| {
        cx.overrides
            .get(key, "enjoyment")
            .and_then(Enjoyment::parse)
    };
    of(&workflow.variant)
        .or_else(|| workflow.sequence.iter().find_map(|&s| of(&cx.steps[s].key)))
        .unwrap_or(Enjoyment::Unknown)
}

/// (from, to, copy events) for step pairs that are next to each other in the sequence or in one of its cases.
fn copy_points(workflow: &Workflow, cx: &Context) -> Vec<(usize, usize, usize)> {
    let mut pairs: HashSet<(usize, usize)> =
        workflow.sequence.windows(2).map(|p| (p[0], p[1])).collect();
    for &c in &workflow.cases {
        pairs.extend(cx.cases[c].seq.windows(2).map(|p| (p[0], p[1])));
    }
    let mut out: Vec<(usize, usize, usize)> = cx
        .follows
        .iter()
        .filter(|f| f.copies > 0 && pairs.contains(&(f.from, f.to)))
        .map(|f| (f.from, f.to, f.copies))
        .collect();
    out.sort_unstable();
    out
}

/// Overdue promises x 0.35 + abandoned opens x 0.15 + 0.1 per waiting day over 2, capped at 1.
fn procrastination(workflow: &Workflow, cx: &Context) -> (f64, String) {
    let entities: HashSet<usize> = workflow.cases.iter().map(|&c| cx.cases[c].entity).collect();
    let overdue = cx
        .commitments
        .iter()
        .filter(|c| c.overdue && c.entity.is_some_and(|e| entities.contains(&e)))
        .count();
    let is_basis_cases = matches!(workflow.basis, Basis::Cases);
    let abandoned = if is_basis_cases {
        workflow
            .cases
            .iter()
            .filter(|&&c| {
                let seq = &cx.cases[c].seq;
                seq.len() < workflow.sequence.len()
                    && workflow.sequence.starts_with(seq)
                    && cx.now - cx.cases[c].last > 2 * DAY
            })
            .count()
    } else {
        0
    };
    let mut waits: Vec<f64> = workflow
        .cases
        .iter()
        .map(|&c| {
            let case = &cx.cases[c];
            let active: i64 = case
                .episodes
                .iter()
                .map(|&e| cx.episodes[e].seconds())
                .sum();
            ((case.last - case.first - active) as f64 / DAY as f64).max(0.0)
        })
        .collect();
    let waiting_days = median(&mut waits);
    let waiting = (waiting_days - 2.0).max(0.0) * 0.1;
    let score = (overdue as f64 * 0.35 + abandoned as f64 * 0.15 + waiting).min(1.0);
    let detail = format!(
        "{overdue} overdue promises x 0.35, {abandoned} abandoned opens x 0.15, {waiting_days:.1} waiting days (0.1 per day over 2)"
    );
    (score, detail)
}
