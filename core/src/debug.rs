//! The debug report (D1) a user pastes into a GitHub issue. It holds versions, switches, row
//! counts, times and error kinds: nothing the user saw, typed, said or named.

use std::fmt::Write;

use serde_json::Value;

use crate::error::Result;
use crate::settings::Settings;
use crate::store::Store;
use crate::time::{Local, iso};

/// What only the app knows. Names are `&'static str`, so no captured text can get in.
pub struct Facts {
    pub app_version: &'static str,
    pub macos: String,
    /// Permission and macOS status, for example ("accessibility", "granted").
    pub permissions: Vec<(&'static str, &'static str)>,
    /// Capture counters and timings, for example ("ocr p95 ms", 41.0).
    pub numbers: Vec<(&'static str, f64)>,
    /// The latest errors, oldest first, as time and kind.
    pub errors: Vec<(i64, &'static str)>,
}

impl Store {
    /// Markdown, built only when asked, so it costs capture and refresh nothing.
    pub fn debug_report(&self, settings: &Settings, facts: &Facts, when: Local) -> Result<String> {
        let time = |ts: Option<i64>| ts.map_or("never".to_owned(), |ts| iso(ts, when.utc_offset_s));
        let mut out = String::with_capacity(4096);
        let _ = writeln!(out, "## Sextant debug report\n");
        let _ = writeln!(out, "- App: {}", facts.app_version);
        let _ = writeln!(out, "- macOS: {}", facts.macos);
        let _ = writeln!(out, "- Schema: {}", self.schema_version()?);
        let _ = writeln!(out, "- Made: {}", time(Some(when.now)));

        let _ = writeln!(out, "\n### Settings\n");
        let _ = writeln!(
            out,
            "- Backfill: {}",
            switches(&serde_json::to_value(settings.sources)?)
        );
        let _ = writeln!(
            out,
            "- Live: {}",
            switches(&serde_json::to_value(settings.live)?)
        );
        let _ = writeln!(
            out,
            "- Agent access: screen text {}, workflows {}",
            on(settings.mcp_rewind),
            on(settings.mcp_workflows)
        );
        let _ = writeln!(
            out,
            "- Agent: {}, weekly {}",
            settings.agent.map_or("off", |agent| agent.as_str()),
            on(settings.agent_weekly)
        );
        let _ = match settings.retention_days {
            Some(days) => writeln!(out, "- Retention: {days} days"),
            None => writeln!(out, "- Retention: forever"),
        };
        let _ = writeln!(out, "- Excluded apps: {}", settings.excluded_apps.len());
        let _ = writeln!(out, "- Git folders: {}", settings.git_folders.len());
        let _ = writeln!(out, "- Onboarded: {}", settings.onboarded);

        let _ = writeln!(out, "\n### Permissions\n");
        for (name, status) in &facts.permissions {
            let _ = writeln!(out, "- {name}: {status}");
        }

        let _ = writeln!(out, "\n### Rows\n\n| Table | Rows |\n| --- | --- |");
        for (table, rows) in self.table_rows()? {
            let _ = writeln!(out, "| {table} | {rows} |");
        }

        let _ = writeln!(
            out,
            "\n### Events\n\n| Kind | Source | Rows | Latest |\n| --- | --- | --- | --- |"
        );
        let mut events = self.conn.prepare(
            "SELECT kind, source, COUNT(*), MAX(ts) FROM events GROUP BY kind, source ORDER BY kind, source",
        )?;
        let mut rows = events.query([])?;
        let mut live = None;
        while let Some(row) = rows.next()? {
            let (kind, source): (String, String) = (row.get(0)?, row.get(1)?);
            let (count, latest): (i64, i64) = (row.get(2)?, row.get(3)?);
            if source == "live" {
                live = live.max(Some(latest));
            }
            let _ = writeln!(
                out,
                "| {kind} | {source} | {count} | {} |",
                time(Some(latest))
            );
        }

        // Both columns are indexed, so each MAX is one index lookup.
        let latest = |sql: &str| -> Result<Option<i64>> {
            Ok(self.conn.query_row(sql, [], |row| row.get(0))?)
        };
        let _ = writeln!(out, "\n### Latest capture\n");
        let _ = writeln!(out, "- Live event: {}", time(live));
        let _ = writeln!(
            out,
            "- Screen text: {}",
            time(latest("SELECT MAX(ts) FROM frames")?)
        );
        let _ = writeln!(
            out,
            "- Transcript: {}",
            time(latest("SELECT MAX(start_ts) FROM utterances")?)
        );

        let _ = writeln!(out, "\n### Capture and refresh\n");
        for (name, value) in &facts.numbers {
            let _ = writeln!(out, "- {name}: {value:.1}");
        }

        let _ = writeln!(out, "\n### Latest errors\n");
        if facts.errors.is_empty() {
            let _ = writeln!(out, "None");
        }
        for (ts, kind) in &facts.errors {
            let _ = writeln!(out, "- {}: {kind}", time(Some(*ts)));
        }
        Ok(out)
    }

    /// Every table, without SQLite's own and the full-text index's internal tables.
    fn table_rows(&self) -> Result<Vec<(String, i64)>> {
        let mut tables = self.conn.prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table'
             AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '%_fts_%' ORDER BY name",
        )?;
        let names = tables
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        names
            .into_iter()
            .map(|name| {
                let sql = format!("SELECT COUNT(*) FROM \"{name}\"");
                let rows = self.conn.query_row(&sql, [], |row| row.get(0))?;
                Ok((name, rows))
            })
            .collect()
    }
}

fn on(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

/// `browsers on, safari off` from a struct of switches.
fn switches(value: &Value) -> String {
    let mut out = String::new();
    for (name, value) in value.as_object().into_iter().flatten() {
        let sep = if out.is_empty() { "" } else { ", " };
        let _ = write!(out, "{sep}{name} {}", on(value.as_bool() == Some(true)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{self, TempDb, UTC_OFFSET_S, WEEK_NOW, local};
    use crate::model::{Source, Speaker, Utterance};

    fn facts() -> Facts {
        Facts {
            app_version: "0.1.0",
            macos: "26.0".to_owned(),
            permissions: vec![("accessibility", "granted")],
            numbers: vec![("refresh ms", 120.0)],
            errors: vec![(WEEK_NOW - 60, "refresh")],
        }
    }

    /// Every string the fixture captured: titles, addresses, field values, screen lines, speech.
    fn captured_text(fixture: &fixtures::Fixture) -> Vec<String> {
        fn strings(value: &Value, found: &mut Vec<String>) {
            match value {
                Value::String(text) => found.push(text.clone()),
                Value::Array(items) => items.iter().for_each(|v| strings(v, found)),
                Value::Object(fields) => fields.values().for_each(|v| strings(v, found)),
                _ => {}
            }
        }
        let mut found = Vec::new();
        for event in &fixture.events {
            strings(&event.json, &mut found);
        }
        for frame in &fixture.frames {
            let window = &frame.window;
            found.extend([&window.app, &window.title, &window.doc, &window.url].map(String::clone));
            found.extend(frame.lines.iter().map(|line| line.text.clone()));
        }
        found.extend(fixture.utterances.iter().map(|u| u.text.clone()));
        found
    }

    #[test]
    fn the_report_counts_the_fixture_without_any_of_its_text() {
        let db = TempDb::new("debug-report");
        let mut store = db.open().expect("open");
        let mut fixture = fixtures::week();
        fixture.utterances.push(Utterance {
            ts_start: WEEK_NOW - 600,
            ts_end: WEEK_NOW - 590,
            source: Speaker::System,
            text: "Kannst du die Rechnung an Acme schicken".to_owned(),
        });
        fixtures::load(&mut store, &fixture, WEEK_NOW).expect("load");
        store.refresh(local(WEEK_NOW)).expect("refresh");
        let mut settings = Settings::default();
        settings.excluded_apps.push("Secret Ledger".to_owned());
        settings
            .git_folders
            .push("/Users/someone/acme-books".to_owned());

        let report = store
            .debug_report(&settings, &facts(), local(WEEK_NOW))
            .expect("report");

        let counts = store.counts().expect("counts");
        assert!(report.contains(&format!("| events | {} |", counts.events)));
        assert!(report.contains(&format!("| frames | {} |", counts.frames)));
        assert!(report.contains(&format!("| utterances | {} |", counts.utterances)));
        assert!(report.contains("| focus | live |"));
        let live = fixture
            .events
            .iter()
            .filter(|e| e.source == Source::Live)
            .map(|e| e.ts)
            .max()
            .expect("live events");
        assert!(report.contains(&format!("- Live event: {}", iso(live, UTC_OFFSET_S))));
        assert!(report.contains("- accessibility: granted"));
        assert!(report.contains("- refresh ms: 120.0"));
        assert!(report.contains(": refresh"));
        assert!(report.contains("- Excluded apps: 13"));

        let captured = captured_text(&fixture);
        assert!(captured.len() > 100, "the fixture has text to leak");
        let leaked: Vec<&String> = captured
            .iter()
            .chain(&settings.excluded_apps)
            .chain(&settings.git_folders)
            .filter(|text| text.chars().count() >= 4 && report.contains(text.as_str()))
            .collect();
        assert!(leaked.is_empty(), "{leaked:?} leaked into\n{report}");
    }

    #[test]
    fn an_empty_database_reports_never_and_no_errors() {
        let db = TempDb::new("debug-empty");
        let store = db.open().expect("open");
        let mut facts = facts();
        facts.errors.clear();
        let report = store
            .debug_report(&Settings::default(), &facts, local(WEEK_NOW))
            .expect("report");
        assert!(report.contains("- Live event: never"));
        assert!(report.contains("| events | 0 |"));
        assert!(report.contains("### Latest errors\n\nNone"));
    }
}
