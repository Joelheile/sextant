//! `SKILL.md` builder: a workflow becomes instructions an agent can follow.

use std::fmt::Write;

use crate::error::{Error, Result};
use crate::finder::{Action, WorkflowDetail};
use crate::store::Store;
use crate::text::slug;
use crate::time::label;

/// Deterministic Markdown for one workflow. Dates use the given UTC offset.
pub fn build(detail: &WorkflowDetail, utc_offset_s: i32) -> String {
    let w = &detail.workflow;
    let mut out = String::new();
    let day = |ts| label(ts, utc_offset_s);
    let _ = writeln!(out, "---");
    let _ = writeln!(out, "name: {}", slug(&w.name));
    let _ = writeln!(
        out,
        "description: Repeat the recurring workflow {}. Use it when the user starts or asks about this work.",
        w.name
    );
    let _ = writeln!(out, "---\n");
    let _ = writeln!(out, "# {}\n", w.name);
    let _ = writeln!(
        out,
        "Sextant saw this {} times. It takes about {:.1} hours a week. Automation potential is {:.2} and the suggested action is {}.{}\n",
        w.case_count,
        w.hours_week,
        w.potential,
        w.action.as_str(),
        if w.rough {
            " The data comes from backfill only, so treat the steps as a first draft."
        } else {
            ""
        }
    );

    let _ = writeln!(out, "## Steps\n");
    for (index, step) in w.steps.iter().enumerate() {
        let _ = writeln!(
            out,
            "{}. {} ({}, potential {:.2})",
            index + 1,
            step.name,
            step.category.as_str(),
            step.potential
        );
    }
    let _ = writeln!(out, "\nStart with: {}.\n", w.first_step);

    if !detail.copy_points.is_empty() {
        let _ = writeln!(out, "## Data copied by hand\n");
        for point in &detail.copy_points {
            let _ = writeln!(
                out,
                "- {} to {}: {} copy events. Read the value from the source and write it to the target.",
                point.from, point.to, point.copies
            );
        }
        out.push('\n');
    }

    if !detail.examples.is_empty() {
        let _ = writeln!(out, "## Example cases\n");
        for case in &detail.examples {
            let _ = writeln!(
                out,
                "- {}: {} to {}, {}",
                case.entity,
                day(case.first_ts),
                day(case.last_ts),
                case.path.join(" > ")
            );
        }
        out.push('\n');
    }

    if !detail.commitments.is_empty() {
        let _ = writeln!(out, "## Open promises\n");
        for commitment in &detail.commitments {
            let _ = writeln!(
                out,
                "- {}: {}{}",
                day(commitment.ts),
                commitment.text,
                if commitment.overdue { " (overdue)" } else { "" }
            );
        }
        out.push('\n');
    }

    let _ = writeln!(out, "## Rules\n");
    let _ = writeln!(
        out,
        "- Show the user what you will do before you send, pay or delete anything."
    );
    let _ = writeln!(
        out,
        "- Stop and ask when a value is missing or two sources disagree."
    );
    if w.client {
        let _ = writeln!(out, "- This is client work. Keep every output reviewable.");
    }
    if w.action == Action::Assist {
        let _ = writeln!(
            out,
            "- Prepare and summarize only. The user does the live part."
        );
    }
    let _ = writeln!(out, "\n## Why Sextant picked it\n");
    for reason in &w.reasons {
        let _ = writeln!(out, "- {reason}");
    }
    out
}

impl Store {
    /// The `SKILL.md` for a workflow id.
    pub fn skill_md(&self, id: i64) -> Result<String> {
        let detail = self.workflow_detail(id)?.ok_or(Error::NoWorkflow(id))?;
        Ok(build(&detail, self.utc_offset_s))
    }
}
