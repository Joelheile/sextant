//! Calendar, reminder, git and file records become events (plan.md §6).

use serde_json::json;

use crate::model::{Event, EventKind, Source};

const LONGEST_MEETING_S: i64 = 12 * 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarRecord {
    pub title: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderRecord {
    pub title: String,
    pub created: Option<i64>,
    pub due: Option<i64>,
    pub done: bool,
}

/// A file the user opened, by name only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRecord {
    pub name: String,
    pub last_used: i64,
}

fn backfill(ts: i64, kind: EventKind, json: serde_json::Value) -> Event {
    Event {
        ts,
        kind,
        source: Source::Backfill,
        json,
    }
}

/// All-day entries and absurdly long ones are not meetings.
pub fn calendar_events(records: &[CalendarRecord]) -> Vec<Event> {
    records
        .iter()
        .filter(|r| !r.all_day && (1..=LONGEST_MEETING_S).contains(&(r.end - r.start)))
        .map(|r| {
            backfill(
                r.start,
                EventKind::Calendar,
                json!({"title": r.title, "end_ts": r.end}),
            )
        })
        .collect()
}

/// A reminder happens when it was created, or when it is due if the creation date is unknown.
pub fn reminder_events(records: &[ReminderRecord]) -> Vec<Event> {
    records
        .iter()
        .filter_map(|r| {
            let ts = r.created.or(r.due)?;
            let mut json = json!({"title": r.title, "done": r.done});
            if let Some(due) = r.due {
                json["due_ts"] = due.into();
            }
            Some(backfill(ts, EventKind::Reminder, json))
        })
        .collect()
}

const LAST_USED: &str = " kMDItemLastUsedDate = ";

/// Reads `mdfind -attr kMDItemLastUsedDate` output: `<path> kMDItemLastUsedDate = 2026-03-02 08:40:12 +0000`.
/// Keeps the file name only. Apps, folders and anything under a Library folder are not documents.
pub fn parse_spotlight(output: &str) -> Vec<FileRecord> {
    output
        .lines()
        .filter_map(|line| {
            let (path, date) = line.split_once(LAST_USED)?;
            let name = path
                .rsplit('/')
                .next()
                .filter(|n| !n.is_empty() && n.contains('.'))?;
            if name.ends_with(".app") || path.contains("/Library/") {
                return None;
            }
            let mut parts = date.split_whitespace();
            let (day, clock, zone) = (parts.next()?, parts.next()?, parts.next().unwrap_or("Z"));
            let last_used = crate::time::parse(&format!("{day}T{clock}{zone}"), 0, false)?;
            Some(FileRecord {
                name: name.to_string(),
                last_used,
            })
        })
        .collect()
}

pub fn file_events(records: &[FileRecord]) -> Vec<Event> {
    records
        .iter()
        .map(|r| backfill(r.last_used, EventKind::File, json!({"name": r.name})))
        .collect()
}

/// Parses `git log --format=%H%x09%at%x09%s` output. Lines that do not fit are skipped.
pub fn commit_events(repo: &str, output: &str) -> Vec<Event> {
    let name = repo
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(repo);
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let hash = fields.next()?;
            let ts = fields.next()?.trim().parse::<i64>().ok()?;
            let subject = fields.next()?;
            if hash.len() < 7 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            Some(backfill(
                ts,
                EventKind::Commit,
                json!({"repo": name, "hash": &hash[..7], "subject": subject}),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_log_lines_become_commit_events() {
        let output = "0123456789abcdef0123456789abcdef01234567\t1772442000\tFix invoice total\n\
                      not a commit line\n\
                      fedcba9876543210fedcba9876543210fedcba98\t1772445600\tTabs\tin subject\n\
                      0123456789abcdef0123456789abcdef01234567\tsoon\tBad time\n";
        let events = commit_events("/Users/me/code/acme-app/", output);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].ts, 1_772_442_000);
        assert_eq!(
            events[0].json,
            json!({"repo": "acme-app", "hash": "0123456", "subject": "Fix invoice total"})
        );
        assert_eq!(events[1].json["subject"], "Tabs\tin subject");
    }

    #[test]
    fn calendar_keeps_timed_meetings_only() {
        let record = |start, end, all_day| CalendarRecord {
            title: "Sync".into(),
            start,
            end,
            all_day,
        };
        let events = calendar_events(&[
            record(1000, 4600, false),
            record(0, 86_400, true),
            record(1000, 1000, false),
            record(1000, 1000 + 13 * 3600, false),
        ]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].json, json!({"title": "Sync", "end_ts": 4600}));
    }

    #[test]
    fn reminders_use_creation_then_due_time() {
        let record = |created, due| ReminderRecord {
            title: "Call Anna".into(),
            created,
            due,
            done: false,
        };
        let events = reminder_events(&[
            record(Some(10), Some(50)),
            record(None, Some(60)),
            record(None, None),
        ]);
        assert_eq!(events.iter().map(|e| e.ts).collect::<Vec<_>>(), [10, 60]);
        assert_eq!(events[0].json["due_ts"], 50);
    }

    #[test]
    fn spotlight_lines_become_file_records() {
        let output = "/Users/jo/Documents/Offer 7.pdf kMDItemLastUsedDate = 2026-03-02 08:40:12 +0000\n\
                      /Users/jo/Library/Mail/x.emlx kMDItemLastUsedDate = 2026-03-02 08:40:12 +0000\n\
                      /Applications/Notes.app kMDItemLastUsedDate = 2026-03-02 08:40:12 +0000\n\
                      /Users/jo/Desktop/a.txt kMDItemLastUsedDate = (null)\n\
                      /Users/jo/Desktop/b.txt kMDItemLastUsedDate = 2026-03-02 09:40:12 +0100\n\
                      garbage";
        let files = parse_spotlight(output);
        assert_eq!(
            files,
            [
                FileRecord {
                    name: "Offer 7.pdf".into(),
                    last_used: 1_772_440_812
                },
                FileRecord {
                    name: "b.txt".into(),
                    last_used: 1_772_440_812
                },
            ]
        );
    }
}
