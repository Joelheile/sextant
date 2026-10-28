//! Browser history (plan.md §6): a History file copied into memory becomes visits, then sessions.

use std::collections::HashMap;

use rusqlite::{Connection, MAIN_DB, params};
use serde_json::json;

use crate::error::Result;
use crate::model::{Event, EventKind, Source};
use crate::text::split_url;

/// Chromium counts microseconds since 1601-01-01.
const WEBKIT_TO_UNIX_S: i64 = 11_644_473_600;
/// Safari counts seconds since 2001-01-01.
const SAFARI_TO_UNIX_S: i64 = 978_307_200;
/// A visit more than this long after the last one on the same host starts a new session.
const SESSION_GAP_S: i64 = 300;
/// Visits report no duration until the tab closes, so the gap to the next visit stands in.
const OPEN_VISIT_S: i64 = 120;
const MIN_VISIT_S: i64 = 5;
const MAX_VISIT_S: i64 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryKind {
    Chromium,
    Safari,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Visit {
    pub ts: i64,
    pub dur_s: i64,
    pub url: String,
    pub title: String,
}

/// A History database held in memory. The live file is never opened (the browser locks it).
pub struct Snapshot {
    conn: Connection,
}

impl Snapshot {
    /// A WAL-mode file is read as a plain one: frames not yet checkpointed into the main
    /// file are not part of the copy.
    pub fn from_bytes(mut bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() > 19 && bytes[18] == 2 && bytes[19] == 2 {
            bytes[18] = 1;
            bytes[19] = 1;
        }
        let mut conn = Connection::open_in_memory()?;
        let size = bytes.len();
        conn.deserialize_read_exact(MAIN_DB, bytes.as_slice(), size, true)?;
        Ok(Self { conn })
    }

    /// Visits at or after `since` (Unix seconds), oldest first. Subframe loads are skipped.
    pub fn visits(&self, kind: HistoryKind, since: i64) -> Result<Vec<Visit>> {
        let mut visits = match kind {
            HistoryKind::Chromium => self.chromium(since)?,
            HistoryKind::Safari => self.safari(since)?,
        };
        visits.retain(|v| v.ts > 0 && web_host(&v.url).is_some());
        visits.sort_by_key(|v| v.ts);
        Ok(visits)
    }

    fn chromium(&self, since: i64) -> Result<Vec<Visit>> {
        let mut statement = self.conn.prepare(
            "SELECT v.visit_time, v.visit_duration, u.url, u.title
             FROM visits v JOIN urls u ON u.id = v.url
             WHERE v.visit_time >= ?1 AND (v.transition & 255) NOT IN (3, 4)",
        )?;
        let rows = statement.query_map(params![(since + WEBKIT_TO_UNIX_S) * 1_000_000], |row| {
            Ok(Visit {
                ts: row.get::<_, i64>(0)? / 1_000_000 - WEBKIT_TO_UNIX_S,
                dur_s: row.get::<_, i64>(1)? / 1_000_000,
                url: row.get(2)?,
                title: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    fn safari(&self, since: i64) -> Result<Vec<Visit>> {
        let mut statement = self.conn.prepare(
            "SELECT v.visit_time, v.title, i.url
             FROM history_visits v JOIN history_items i ON i.id = v.history_item
             WHERE v.visit_time >= ?1",
        )?;
        let rows = statement.query_map(params![(since - SAFARI_TO_UNIX_S) as f64], |row| {
            Ok(Visit {
                ts: row.get::<_, f64>(0)? as i64 + SAFARI_TO_UNIX_S,
                dur_s: 0,
                url: row.get(2)?,
                title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

/// Host of an http(s) URL. Other schemes (`chrome://`, `file://`, extensions) are not browsing.
fn web_host(url: &str) -> Option<String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return None;
    }
    let (host, _) = split_url(url);
    (!host.is_empty()).then_some(host)
}

struct Session {
    start: i64,
    end: i64,
    best: usize,
}

/// One `visit` event per session: consecutive visits to one host with gaps of at most 5 minutes.
/// The event carries the page the user stayed on longest. `visits` must be sorted by time.
pub fn sessions(browser: &str, visits: &[Visit]) -> Vec<Event> {
    let duration = |index: usize| {
        let visit = &visits[index];
        let dur = if visit.dur_s > 0 {
            visit.dur_s
        } else {
            visits
                .get(index + 1)
                .map_or(OPEN_VISIT_S, |next| (next.ts - visit.ts).min(OPEN_VISIT_S))
        };
        dur.clamp(MIN_VISIT_S, MAX_VISIT_S)
    };
    let mut open: HashMap<String, Session> = HashMap::new();
    let mut done: Vec<Session> = Vec::new();
    for (index, visit) in visits.iter().enumerate() {
        let Some(host) = web_host(&visit.url) else {
            continue;
        };
        let end = visit.ts + duration(index);
        match open.get_mut(&host) {
            Some(session) if visit.ts - session.end <= SESSION_GAP_S => {
                if duration(index) > duration(session.best) {
                    session.best = index;
                }
                session.end = session.end.max(end);
            }
            slot => {
                let fresh = Session {
                    start: visit.ts,
                    end,
                    best: index,
                };
                if let Some(old) = slot {
                    done.push(std::mem::replace(old, fresh));
                } else {
                    open.insert(host, fresh);
                }
            }
        }
    }
    done.extend(open.into_values());
    done.sort_by_key(|s| (s.start, s.best));
    done.into_iter()
        .map(|session| {
            let page = &visits[session.best];
            Event {
                ts: session.start,
                kind: EventKind::Visit,
                source: Source::Backfill,
                json: json!({
                    "app": browser,
                    "title": page.title,
                    "url": page.url,
                    "dur_s": session.end - session.start,
                }),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visit(ts: i64, dur_s: i64, url: &str) -> Visit {
        Visit {
            ts,
            dur_s,
            url: url.into(),
            title: format!("title of {url}"),
        }
    }

    fn history(sql: &str) -> Snapshot {
        let conn = Connection::open_in_memory().expect("memory db");
        conn.execute_batch(sql).expect("schema");
        let mut conn = conn;
        let data = conn.serialize(MAIN_DB).expect("serialize");
        Snapshot::from_bytes(data.to_vec()).expect("snapshot")
    }

    #[test]
    fn chromium_rows_convert_from_the_webkit_epoch() {
        // 2026-03-02 09:00:00 UTC is 1_772_442_000 Unix seconds.
        let webkit = (1_772_442_000 + WEBKIT_TO_UNIX_S) * 1_000_000;
        let snapshot = history(&format!(
            "CREATE TABLE urls(id INTEGER PRIMARY KEY, url TEXT, title TEXT);
             CREATE TABLE visits(id INTEGER PRIMARY KEY, url INTEGER, visit_time INTEGER, visit_duration INTEGER, transition INTEGER);
             INSERT INTO urls VALUES (1, 'https://app.lexoffice.de/vouchers/1', 'Rechnung'), (2, 'chrome://settings', 'Settings'), (3, 'https://ads.example.com/frame', NULL);
             INSERT INTO visits VALUES (1, 1, {webkit}, 42000000, 805306368), (2, 2, {webkit}, 0, 0), (3, 3, {webkit}, 0, 3), (4, 1, {old}, 0, 0);",
            old = webkit - 200 * 86_400 * 1_000_000,
        ));
        let visits = snapshot
            .visits(HistoryKind::Chromium, 1_772_442_000 - 3600)
            .expect("visits");
        assert_eq!(
            visits,
            [Visit {
                ts: 1_772_442_000,
                dur_s: 42,
                url: "https://app.lexoffice.de/vouchers/1".into(),
                title: "Rechnung".into()
            }],
            "the chrome:// page, the subframe and the old visit are dropped"
        );
    }

    #[test]
    fn safari_rows_convert_from_the_core_data_epoch() {
        let safari = 1_772_442_000 - SAFARI_TO_UNIX_S;
        let snapshot = history(&format!(
            "CREATE TABLE history_items(id INTEGER PRIMARY KEY, url TEXT);
             CREATE TABLE history_visits(id INTEGER PRIMARY KEY, history_item INTEGER, visit_time REAL, title TEXT);
             INSERT INTO history_items VALUES (1, 'https://www.apple.com/mac/');
             INSERT INTO history_visits VALUES (1, 1, {safari}.5, 'Mac');"
        ));
        let visits = snapshot.visits(HistoryKind::Safari, 0).expect("visits");
        assert_eq!((visits[0].ts, visits[0].dur_s), (1_772_442_000, 0));
        assert_eq!(visits[0].title, "Mac");
    }

    #[test]
    fn wal_files_open_as_plain_files() {
        let path = std::env::temp_dir().join(format!("sextant-wal-{}.db", std::process::id()));
        {
            let conn = Connection::open(&path).expect("file db");
            conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get::<_, String>(0))
                .expect("wal");
            conn.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (1);")
                .expect("rows");
            conn.close().expect("close checkpoints");
        }
        let bytes = std::fs::read(&path).expect("read");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        assert_eq!((bytes[18], bytes[19]), (2, 2));
        let snapshot = Snapshot::from_bytes(bytes).expect("snapshot");
        let n: i64 = snapshot
            .conn
            .query_row("SELECT count(*) FROM t", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 1);
    }

    #[test]
    fn sessions_split_by_host_and_five_minute_gaps() {
        let visits = [
            visit(1000, 30, "https://a.com/1"),
            visit(1040, 200, "https://b.com/x"),
            visit(1100, 60, "https://a.com/2"),
            visit(1400, 20, "https://a.com/3"),
            visit(1721, 20, "https://a.com/4"),
            visit(5000, 0, "https://b.com/y"),
        ];
        let events = sessions("Google Chrome", &visits);
        let summary: Vec<(i64, i64, &str)> = events
            .iter()
            .map(|e| {
                (
                    e.ts,
                    e.json["dur_s"].as_i64().unwrap_or(-1),
                    e.json["url"].as_str().unwrap_or(""),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (1000, 420, "https://a.com/2"),
                (1040, 200, "https://b.com/x"),
                (1721, 20, "https://a.com/4"),
                (5000, 120, "https://b.com/y"),
            ]
        );
        assert_eq!(events[0].json["app"], "Google Chrome");
    }
}
