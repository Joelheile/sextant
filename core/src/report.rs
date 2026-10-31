//! The weekly report (plan.md §9) and the redacted data behind its share image. It holds numbers
//! and category names only: no entity, title, URL, text, step name or user-written name.

use crate::error::Result;
use crate::finder::{Action, Category};
use crate::store::Store;
use crate::time::{DAY, Local, WEEK_DAYS};

const TOP: usize = 3;

/// A workflow as the share image shows it: the categories its steps pass through.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportRow {
    pub categories: Vec<Category>,
    pub hours_week: f64,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WeeklyReport {
    pub from_ts: i64,
    pub to_ts: i64,
    /// Hours a week the visible workflows could give back.
    pub hours_back: f64,
    /// Hours a week already given back by measured handoffs.
    pub hours_saved: f64,
    pub handoffs: usize,
    pub top: Vec<ReportRow>,
}

impl Store {
    /// The report for the 7 days up to `when.now`.
    pub fn weekly_report(&self, when: Local) -> Result<WeeklyReport> {
        let results = self.results(when)?;
        let top = self
            .workflows()?
            .iter()
            .filter(|w| matches!(w.action, Action::Automate | Action::Assist))
            .take(TOP)
            .map(|w| {
                let mut categories: Vec<Category> = w.steps.iter().map(|s| s.category).collect();
                categories.dedup();
                ReportRow {
                    categories,
                    hours_week: w.hours_week,
                    action: w.action,
                }
            })
            .collect();
        Ok(WeeklyReport {
            from_ts: when.now - WEEK_DAYS * DAY,
            to_ts: when.now,
            hours_back: self.hours_back()?,
            hours_saved: results.iter().filter_map(|m| m.saved()).sum(),
            handoffs: results.len(),
            top,
        })
    }
}
