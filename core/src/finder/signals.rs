//! §7.8 Behaviour signals from the event stream alone, no integrations: avoidance flinches,
//! interruptions and the time away they cost, and complaints in my own speech.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use regex::Regex;

use super::commitments::Linker;
use super::episodes::Episode;
use super::steps::{Category, Step};
use crate::text::pattern;
use crate::time::day;

/// An open this short that ends in social or media is a flinch.
const FLINCH_S: i64 = 30;
/// The escape starts within this gap after the flinch.
const ESCAPE_GAP_S: i64 = 60;
/// Work before an interruption lasts at least this long.
const WORKING_S: i64 = 60;
/// Chat or mail that starts this soon after work ends cut in.
const CUT_IN_GAP_S: i64 = 10;
const CUT_IN_MAX_S: i64 = 5 * 60;
/// Work picked up later than this after an interruption counts as dropped.
const RESUME_WITHIN_S: i64 = 30 * 60;

static COMPLAINT: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"(?i)\b(?:annoying|tedious|i hate|hate doing|waste of time|drives me (?:crazy|nuts)|pain in the|again and again|every (?:single )?(?:time|day|week|month) i (?:have|need) to|by hand|manually|nervig|nervt|ätzend|lästig|mühsam|ich hasse|zeitverschwendung|schon wieder|jedes mal|jede woche muss ich|von hand|händisch|manuell|keine lust)\b",
    )
});

pub(crate) fn is_complaint(text: &str) -> bool {
    COMPLAINT.is_match(text)
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Flinches {
    pub count: usize,
    pub days: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Interruption {
    /// Step that was cut into, and the chat or mail step that cut in.
    pub step: usize,
    pub by: usize,
    /// From leaving the work to coming back to it, None when it was not picked up within 30 min.
    pub away_s: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Complaint {
    pub ts: i64,
    /// Index of the linked task.
    pub task: Option<usize>,
}

#[derive(Debug, Default)]
pub(crate) struct Signals {
    /// Per step index.
    pub flinches: Vec<Flinches>,
    pub interruptions: Vec<Interruption>,
    pub complaints: Vec<Complaint>,
}

fn distraction(category: Category) -> bool {
    matches!(category, Category::Social | Category::Media)
}

fn reactive(category: Category) -> bool {
    matches!(category, Category::Chat | Category::Email)
}

fn focused_work(category: Category) -> bool {
    !distraction(category)
        && !reactive(category)
        && !matches!(category, Category::Meeting | Category::Calendar)
}

/// `spoken` are (ts, text) of mic utterances.
pub(crate) fn build(
    episodes: &[Episode],
    steps: &[Step],
    spoken: &[(i64, String)],
    linker: &Linker,
    utc_offset_s: i32,
) -> Signals {
    let category = |e: &Episode| steps[e.step].category;
    Signals {
        flinches: flinches(episodes, steps.len(), &category, utc_offset_s),
        interruptions: interruptions(episodes, &category),
        complaints: spoken
            .iter()
            .filter(|(_, text)| is_complaint(text))
            .map(|(ts, text)| Complaint {
                ts: *ts,
                task: linker.link(&COMPLAINT.replace_all(text, " ")),
            })
            .collect(),
    }
}

/// Opened a piece of work for under 30 s, then went straight to social or media.
fn flinches(
    episodes: &[Episode],
    step_count: usize,
    category: &impl Fn(&Episode) -> Category,
    utc_offset_s: i32,
) -> Vec<Flinches> {
    let mut days: BTreeMap<usize, (usize, BTreeSet<i64>)> = BTreeMap::new();
    for pair in episodes.windows(2) {
        let (open, escape) = (&pair[0], &pair[1]);
        if focused_work(category(open))
            && open.seconds() <= FLINCH_S
            && distraction(category(escape))
            && (0..=ESCAPE_GAP_S).contains(&(escape.start - open.end))
        {
            let entry = days.entry(open.step).or_default();
            entry.0 += 1;
            entry.1.insert(day(open.start, utc_offset_s));
        }
    }
    let mut out = vec![Flinches::default(); step_count];
    for (step, (count, on)) in days {
        out[step] = Flinches {
            count,
            days: on.len(),
        };
    }
    out
}

/// Focused work for a minute or more, then chat or mail within 10 s, for at most 5 min.
fn interruptions(
    episodes: &[Episode],
    category: &impl Fn(&Episode) -> Category,
) -> Vec<Interruption> {
    let mut out = Vec::new();
    for (index, pair) in episodes.windows(2).enumerate() {
        let (work, cut) = (&pair[0], &pair[1]);
        if !focused_work(category(work))
            || work.seconds() < WORKING_S
            || !reactive(category(cut))
            || cut.seconds() > CUT_IN_MAX_S
            || !(0..=CUT_IN_GAP_S).contains(&(cut.start - work.end))
        {
            continue;
        }
        let away_s = episodes[index + 2..]
            .iter()
            .take_while(|e| e.start - cut.end <= RESUME_WITHIN_S)
            .find(|e| e.step == work.step)
            .map(|e| e.start - work.end);
        out.push(Interruption {
            step: work.step,
            by: cut.step,
            away_s,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complaints_match_english_and_german_phrases() {
        for text in [
            "Every week I have to copy these totals, so annoying",
            "I type it in by hand",
            "Das ist echt nervig",
            "Schon wieder die Rechnungen",
            "Ich mache das jedes Mal händisch",
        ] {
            assert!(is_complaint(text), "{text}");
        }
        for text in [
            "I'll send it tomorrow",
            "The handover is on Monday",
            "Manual of style",
        ] {
            assert!(!is_complaint(text), "{text}");
        }
    }
}
