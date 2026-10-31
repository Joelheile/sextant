//! A fake macOS profile for backfill tests, the first-map budget and `sextant bench`: Chromium and
//! Safari History files (real SQLite bytes), a git log, calendar entries, reminders and recent files.
//! Dates and invoice numbers follow `fixtures`, so the invoice workflow is recoverable.

use rusqlite::{Connection, MAIN_DB, params};

use crate::backfill::{
    CalendarRecord, FileRecord, HistoryFile, HistoryKind, Platform, ReminderRecord,
};
use crate::error::{Error, Result};
use crate::fixtures::{CLIENTS, INVOICES, Part, Rng, SLACK_TEXT, at};
use crate::time::{DAY, SAFARI_TO_UNIX_S, WEBKIT_TO_UNIX_S};

pub struct ProfileSpec {
    /// Local midnight of a Monday.
    pub first_monday: i64,
    pub weeks: u32,
    pub noise_visits_per_day: u32,
}

pub struct Profile {
    pub chromium: Vec<u8>,
    pub safari: Vec<u8>,
    pub git_log: String,
    pub calendar: Vec<CalendarRecord>,
    pub reminders: Vec<ReminderRecord>,
    pub files: Vec<FileRecord>,
}

/// One Chromium visit: Unix seconds, seconds on the page, url, title, transition.
type Row = (i64, i64, String, String, i64);

const NOISE: [(&str, &str); 16] = [
    (
        "https://news.ycombinator.com/item?id={n}",
        "Show HN: tiny tool {n} | Hacker News",
    ),
    (
        "https://en.wikipedia.org/wiki/Process_mining",
        "Process mining - Wikipedia",
    ),
    (
        "https://github.com/acme/app/pull/{n}",
        "Fix flaky test {n} by dev · Pull Request #{n} · acme/app",
    ),
    (
        "https://stackoverflow.com/questions/{n}/rust-lifetimes",
        "Rust lifetimes explained - Stack Overflow",
    ),
    (
        "https://www.youtube.com/watch?v={n}",
        "Rust conference talk - YouTube",
    ),
    ("https://x.com/home", "Home / X"),
    ("https://www.linkedin.com/feed/", "Feed | LinkedIn"),
    (
        "https://www.figma.com/file/{n}/Landing",
        "Landing page – Figma",
    ),
    ("https://www.notion.so/Roadmap-{n}", "Roadmap – Notion"),
    (
        "https://docs.google.com/document/d/{n}/edit",
        "Pricing draft - Google Docs",
    ),
    ("https://app.hubspot.com/contacts/{n}", "Contacts - HubSpot"),
    (
        "https://www.heise.de/news/Datenschutz-{n}.html",
        "Datenschutz: neue Leitlinien | heise online",
    ),
    (
        "https://www.reddit.com/r/rust/comments/{n}",
        "Ask r/rust: async traits",
    ),
    (
        "https://www.spiegel.de/wirtschaft/{n}",
        "Wirtschaft | DER SPIEGEL",
    ),
    (
        "https://maps.google.com/maps?q=Hamburg",
        "Hamburg - Google Maps",
    ),
    (
        "https://www.amazon.de/dp/{n}",
        "Amazon.de: Schreibtischlampe",
    ),
];

fn invoice_rows(first_monday: i64, weeks: u32) -> Vec<Row> {
    let mut rows = Vec::new();
    for week in 0..weeks {
        let monday = first_monday + i64::from(week) * 7 * DAY;
        for (index, (number, german, visits)) in INVOICES.iter().enumerate() {
            let id = format!(
                "{}-2026-{:04}",
                if *german { "RE" } else { "INV" },
                number + 20 * week
            );
            let (client, _) = CLIENTS[index];
            for &(day, hour, minute, part, seconds, _) in *visits {
                let ts = at(monday + day * DAY, hour, minute);
                let (url, title) = match part {
                    Part::Mail => (
                        format!("https://mail.google.com/mail/u/0/#inbox/{id}"),
                        format!("Invoice {id} attached - Gmail"),
                    ),
                    Part::Excel => (
                        format!("https://docs.google.com/spreadsheets/d/{id}/edit"),
                        format!("Invoice {id} {client} - Google Sheets"),
                    ),
                    Part::Lex => (
                        format!("https://app.lexoffice.de/vouchers/{id}/edit?tab=1"),
                        "Rechnung bearbeiten - lexoffice".to_string(),
                    ),
                };
                rows.push((ts, seconds, url, title, 0));
            }
        }
    }
    rows
}

fn routine_rows(first_monday: i64, weeks: u32) -> Vec<Row> {
    let mut rows = Vec::new();
    for day in 0..i64::from(weeks) * 7 {
        if day % 7 >= 5 {
            continue;
        }
        let midnight = first_monday + day * DAY;
        let slack = SLACK_TEXT[(day % 7) as usize];
        for (hour, minute, seconds, url, title) in [
            (
                9,
                0,
                120,
                "https://calendar.google.com/calendar/u/0/r/week",
                "Calendar - Today",
            ),
            (
                9,
                3,
                150,
                "https://app.slack.com/client/T01/C01",
                "Slack | #standup",
            ),
            (
                9,
                7,
                120,
                "https://linear.app/acme/my-issues",
                "My issues - Linear",
            ),
        ] {
            let title = if url.contains("slack") {
                format!("{title} | {}", slack.chars().take(20).collect::<String>())
            } else {
                title.to_string()
            };
            rows.push((
                at(midnight, hour, minute),
                seconds,
                url.to_string(),
                title,
                0,
            ));
        }
    }
    rows
}

fn noise_rows(first_monday: i64, weeks: u32, per_day: u32, rng: &mut Rng) -> Vec<Row> {
    let mut rows = Vec::new();
    for day in 0..i64::from(weeks) * 7 {
        if day % 7 >= 5 {
            continue;
        }
        let midnight = first_monday + day * DAY;
        for _ in 0..per_day {
            let (url, title) = NOISE[rng.below(NOISE.len() as u64) as usize];
            let n = (1000 + rng.below(9000)).to_string();
            let ts = midnight + 8 * 3600 + rng.below(11 * 3600) as i64;
            rows.push((
                ts,
                15 + rng.below(226) as i64,
                url.replace("{n}", &n),
                title.replace("{n}", &n),
                0,
            ));
        }
        // Pages the importer must skip: browser-internal pages and ad frames.
        let ts = midnight + 10 * 3600;
        rows.push((ts, 3, "chrome://newtab/".into(), "New Tab".into(), 0));
        rows.push((
            ts + 60,
            1,
            "https://ads.example.com/frame".into(),
            String::new(),
            3,
        ));
    }
    rows
}

fn chromium_bytes(rows: &[Row]) -> Result<Vec<u8>> {
    let mut conn = Connection::open_in_memory()?;
    conn.execute_batch(
        "CREATE TABLE urls(id INTEGER PRIMARY KEY, url LONGVARCHAR, title LONGVARCHAR, visit_count INTEGER DEFAULT 0, last_visit_time INTEGER DEFAULT 0);
         CREATE TABLE visits(id INTEGER PRIMARY KEY, url INTEGER NOT NULL, visit_time INTEGER NOT NULL, from_visit INTEGER, transition INTEGER DEFAULT 0 NOT NULL, visit_duration INTEGER DEFAULT 0 NOT NULL);
         CREATE INDEX visits_time ON visits(visit_time);",
    )?;
    let tx = conn.transaction()?;
    {
        let mut url_id = tx.prepare("SELECT id FROM urls WHERE url = ?1")?;
        let mut new_url = tx.prepare("INSERT INTO urls(url, title) VALUES (?1, ?2)")?;
        let mut visit = tx.prepare(
            "INSERT INTO visits(url, visit_time, transition, visit_duration) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (ts, seconds, url, title, transition) in rows {
            let id: i64 = match url_id.query_row(params![url], |r| r.get(0)) {
                Ok(id) => id,
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    new_url.execute(params![url, title])?;
                    tx.last_insert_rowid()
                }
                Err(err) => return Err(err.into()),
            };
            visit.execute(params![
                id,
                (ts + WEBKIT_TO_UNIX_S) * 1_000_000,
                // Chromium keeps qualifier bits above the low byte.
                transition | 0x1000_0000,
                seconds * 1_000_000
            ])?;
        }
    }
    tx.commit()?;
    Ok(conn.serialize(MAIN_DB)?.to_vec())
}

fn safari_bytes(first_monday: i64, weeks: u32) -> Result<Vec<u8>> {
    let mut conn = Connection::open_in_memory()?;
    conn.execute_batch(
        "CREATE TABLE history_items(id INTEGER PRIMARY KEY, url TEXT NOT NULL);
         CREATE TABLE history_visits(id INTEGER PRIMARY KEY, history_item INTEGER NOT NULL, visit_time REAL NOT NULL, title TEXT);
         INSERT INTO history_items VALUES (1, 'https://www.apple.com/de/mac/'), (2, 'https://www.tagesschau.de/');",
    )?;
    let tx = conn.transaction()?;
    for day in 0..i64::from(weeks) * 7 {
        for (item, hour, title) in [(1, 12, "Mac - Apple (DE)"), (2, 18, "tagesschau.de")] {
            let ts = at(first_monday + day * DAY, hour, 15);
            tx.execute(
                "INSERT INTO history_visits(history_item, visit_time, title) VALUES (?1, ?2, ?3)",
                params![item, (ts - SAFARI_TO_UNIX_S) as f64, title],
            )?;
        }
    }
    tx.commit()?;
    Ok(conn.serialize(MAIN_DB)?.to_vec())
}

fn git_log(first_monday: i64, weeks: u32) -> String {
    let subjects = [
        "Fix invoice total",
        "Add export",
        "Bump deps",
        "Refactor mailer",
    ];
    let mut out = String::new();
    for day in 0..i64::from(weeks) * 7 {
        if day % 7 >= 5 {
            continue;
        }
        for (n, subject) in subjects.iter().enumerate().take(1 + (day % 3) as usize) {
            let hash = format!("{:040x}", (day as u64 + 1) * 1_000 + n as u64);
            let ts = at(first_monday + day * DAY, 11 + n as i64, 5);
            out.push_str(&format!("{hash}\t{ts}\t{subject}\n"));
        }
    }
    out
}

pub fn profile(spec: &ProfileSpec) -> Result<Profile> {
    let mut rng = Rng(0x0bac_f111 ^ u64::from(spec.weeks));
    let mut rows = invoice_rows(spec.first_monday, spec.weeks);
    rows.extend(routine_rows(spec.first_monday, spec.weeks));
    rows.extend(noise_rows(
        spec.first_monday,
        spec.weeks,
        spec.noise_visits_per_day,
        &mut rng,
    ));
    rows.sort_by_key(|row| row.0);

    let mut calendar = Vec::new();
    let mut reminders = Vec::new();
    for week in 0..i64::from(spec.weeks) {
        let monday = spec.first_monday + week * 7 * DAY;
        for (day, hour, minutes) in [(2, 14, 45), (4, 15, 30)] {
            let start = at(monday + day * DAY, hour, 0);
            calendar.push(CalendarRecord {
                title: "Weekly sync".into(),
                start,
                end: start + minutes * 60,
                all_day: false,
            });
        }
        calendar.push(CalendarRecord {
            title: "Public holiday".into(),
            start: monday,
            end: monday + DAY,
            all_day: true,
        });
        reminders.push(ReminderRecord {
            title: format!("Send reminder for INV-2026-{:04}", 423 + 20 * week),
            created: Some(at(monday + 2 * DAY, 13, 10)),
            due: Some(at(monday + 3 * DAY, 17, 0)),
            done: false,
        });
    }
    let files = (0..spec.weeks * 3)
        .map(|n| FileRecord {
            name: format!("Offer {n}.pdf"),
            last_used: at(spec.first_monday + i64::from(n) * 2 * DAY, 15, 0),
        })
        .collect();
    Ok(Profile {
        chromium: chromium_bytes(&rows)?,
        safari: safari_bytes(spec.first_monday, spec.weeks)?,
        git_log: git_log(spec.first_monday, spec.weeks),
        calendar,
        reminders,
        files,
    })
}

/// Replays a [`Profile`] through the backfill [`Platform`] trait, with no permissions needed.
pub struct FakePlatform {
    pub profile: Profile,
    /// Makes calendar and reminder reads fail, like a denied permission.
    pub calendar_denied: bool,
}

impl Platform for FakePlatform {
    fn history_files(&self, safari: bool) -> Vec<HistoryFile> {
        let mut files = vec![HistoryFile {
            browser: "Google Chrome".into(),
            kind: HistoryKind::Chromium,
            location: "chromium".into(),
        }];
        if safari {
            files.push(HistoryFile {
                browser: "Safari".into(),
                kind: HistoryKind::Safari,
                location: "safari".into(),
            });
        }
        files
    }

    fn read_history(&self, file: &HistoryFile) -> Result<Vec<u8>> {
        Ok(match file.kind {
            HistoryKind::Chromium => self.profile.chromium.clone(),
            HistoryKind::Safari => self.profile.safari.clone(),
        })
    }

    fn calendar(&self, from: i64, to: i64) -> Result<Vec<CalendarRecord>> {
        if self.calendar_denied {
            return Err(Error::Platform("calendar access denied".into()));
        }
        let mut records = self.profile.calendar.clone();
        records.retain(|r| (from..=to).contains(&r.start));
        Ok(records)
    }

    fn reminders(&self, from: i64, to: i64) -> Result<Vec<ReminderRecord>> {
        if self.calendar_denied {
            return Err(Error::Platform("reminders access denied".into()));
        }
        let mut records = self.profile.reminders.clone();
        records.retain(|r| {
            r.created
                .or(r.due)
                .is_some_and(|ts| (from..=to).contains(&ts))
        });
        Ok(records)
    }

    fn git_repos(&self, folder: &str) -> Vec<String> {
        vec![format!("{}/acme-app", folder.trim_end_matches('/'))]
    }

    fn git_log(&self, _repo: &str, _since: i64) -> Result<String> {
        Ok(self.profile.git_log.clone())
    }

    fn recent_files(&self, _since: i64) -> Result<Vec<FileRecord>> {
        Ok(self.profile.files.clone())
    }
}
