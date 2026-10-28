//! FTS search over screen lines and transcripts (plan.md §5, §8).

use std::collections::HashSet;

use rusqlite::{OptionalExtension, params};

use crate::error::Result;
use crate::redact::redact;
use crate::store::Store;
use crate::text::words;
use crate::writer::unpack;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitSource {
    Screen,
    Mic,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub ts: i64,
    pub source: HitSource,
    pub app: String,
    pub title: String,
    pub url: String,
    pub text: String,
}

/// The query is redacted first, so searching for an email finds its `<email:hash>`.
fn query_terms(query: &str) -> Vec<String> {
    words(&redact(query))
}

/// Quotes every word so user input can never be FTS syntax.
fn match_expression(terms: &[String]) -> String {
    let quoted: Vec<String> = terms.iter().map(|t| format!("\"{t}\"")).collect();
    quoted.join(" ")
}

/// A query matching more lines than this is answered by walking back from the newest frame
/// instead of looking up the newest frame of every match.
const FEW_MATCHES: usize = 1_000;

/// Distinct lines the walk may test before search gives up on it and looks up every match.
const WALK_BUDGET: usize = 50_000;

/// Finds each match's newest frame in range from the `frame_lines` index alone, keeps the page
/// of newest, and only then reads the page's frames, windows and texts.
const SCREEN: &str = "
    SELECT f.ts, w.app, w.title, w.url_host_path, l.text
    FROM (
        SELECT line_id, frame_id FROM (
            SELECT lines_fts.rowid AS line_id, (
                SELECT fl.frame_id FROM frame_lines fl
                WHERE fl.line_id = lines_fts.rowid AND fl.frame_id BETWEEN ?2 AND ?3
                  AND (?4 IS NULL OR EXISTS (
                      SELECT 1 FROM frames f2 JOIN windows w2 ON w2.id = f2.window_id
                      WHERE f2.id = fl.frame_id AND w2.app = ?4 COLLATE NOCASE))
                ORDER BY fl.frame_id DESC LIMIT 1) AS frame_id
            FROM lines_fts WHERE lines_fts MATCH ?1)
        WHERE frame_id IS NOT NULL
        ORDER BY frame_id DESC, line_id DESC
        LIMIT ?5) newest
    JOIN frames f ON f.id = newest.frame_id
    JOIN windows w ON w.id = f.window_id
    JOIN lines l ON l.id = newest.line_id
    ORDER BY newest.frame_id DESC, newest.line_id DESC";

const WALK_FRAMES: &str = "
    SELECT f.id, f.ts, f.packed, w.app, w.title, w.url_host_path
    FROM frames f JOIN windows w ON w.id = f.window_id
    WHERE f.id BETWEEN ?1 AND ?2 AND (?3 IS NULL OR w.app = ?3 COLLATE NOCASE)
    ORDER BY f.id DESC";

const SPEECH: &str = "
    SELECT u.ts_start, u.source, u.text
    FROM utterances_fts
    JOIN utterances u ON u.id = utterances_fts.rowid
    WHERE utterances_fts MATCH ?1 AND u.ts_start BETWEEN ?2 AND ?3
    ORDER BY u.ts_start DESC
    LIMIT ?4";

impl Store {
    /// Newest first. Every word must appear in the same screen line or utterance.
    /// Each distinct screen line is returned once, at its latest appearance in range.
    /// An `app` filter limits results to screen text of that app.
    pub fn search(
        &self,
        query: &str,
        from: Option<i64>,
        to: Option<i64>,
        app: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Hit>> {
        let terms = query_terms(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let expression = match_expression(&terms);
        let (from, to) = (from.unwrap_or(i64::MIN), to.unwrap_or(i64::MAX));
        let cap = (limit + offset) as i64;
        let mut hits = self.search_screen(&terms, from, to, app, cap)?;
        if app.is_none() {
            hits.extend(self.search_speech(&expression, from, to, cap)?);
        }
        hits.sort_by(|a, b| b.ts.cmp(&a.ts));
        Ok(hits.into_iter().skip(offset).take(limit).collect())
    }

    /// Frame ids grow with time, so the time range becomes an id range that the
    /// `frame_lines` primary key can seek into, and a line's newest frame is the one with the
    /// highest id. Hits come newest frame first, and by line id within a frame.
    fn search_screen(
        &self,
        terms: &[String],
        from: i64,
        to: i64,
        app: Option<&str>,
        cap: i64,
    ) -> Result<Vec<Hit>> {
        let Some(range) = self.frame_range(from, to)? else {
            return Ok(Vec::new());
        };
        let expression = match_expression(terms);
        let matches = self.matching_lines(&expression)?;
        if matches.len() > FEW_MATCHES {
            let walked = self.search_screen_walk(&matches, range, app, cap, WALK_BUDGET)?;
            if let Some(hits) = walked {
                return Ok(hits);
            }
        }
        self.search_screen_lookup(&expression, range, app, cap)
    }

    /// Ids of every line the expression matches, ascending. Listing them costs about 25 ns each.
    fn matching_lines(&self, expression: &str) -> Result<Vec<i64>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT rowid FROM lines_fts WHERE lines_fts MATCH ?1 ORDER BY rowid",
        )?;
        let rows = statement.query_map(params![expression], |row| row.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// One index lookup per match, so the cost grows with the number of matching lines.
    fn search_screen_lookup(
        &self,
        expression: &str,
        (first, last): (i64, i64),
        app: Option<&str>,
        cap: i64,
    ) -> Result<Vec<Hit>> {
        let mut statement = self.conn.prepare_cached(SCREEN)?;
        let rows = statement.query_map(params![expression, first, last, app, cap], |row| {
            Ok(Hit {
                ts: row.get(0)?,
                source: HitSource::Screen,
                app: row.get(1)?,
                title: row.get(2)?,
                url: row.get(3)?,
                text: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Reads frames from the newest back and looks each line up in `matches` the first time it
    /// shows up, which is its newest appearance. Stops after the frame that completes `cap`
    /// hits. Returns `None` after testing `budget` lines: a query that few lines in range match
    /// is cheaper to answer by looking up its matches.
    fn search_screen_walk(
        &self,
        matches: &[i64],
        (first, last): (i64, i64),
        app: Option<&str>,
        cap: i64,
        budget: usize,
    ) -> Result<Option<Vec<Hit>>> {
        let mut frames = self.conn.prepare_cached(WALK_FRAMES)?;
        let mut rows = frames.query(params![first, last, app])?;
        let mut seen = HashSet::new();
        let mut found: Vec<(i64, Hit)> = Vec::new();
        while let Some(row) = rows.next()? {
            let mut in_frame = Vec::new();
            for (line_id, _) in unpack(&row.get::<_, Vec<u8>>(2)?) {
                if !seen.insert(line_id) {
                    continue;
                }
                if seen.len() > budget {
                    return Ok(None);
                }
                if matches.binary_search(&line_id).is_ok() {
                    in_frame.push(line_id);
                }
            }
            in_frame.sort_unstable_by(|a, b| b.cmp(a));
            for line_id in in_frame {
                let hit = Hit {
                    ts: row.get(1)?,
                    source: HitSource::Screen,
                    app: row.get(3)?,
                    title: row.get(4)?,
                    url: row.get(5)?,
                    text: String::new(),
                };
                found.push((line_id, hit));
            }
            if found.len() as i64 >= cap {
                break;
            }
        }
        found.truncate(cap as usize);
        let mut text = self
            .conn
            .prepare_cached("SELECT text FROM lines WHERE id = ?1")?;
        let hits = found
            .into_iter()
            .map(|(line_id, mut hit)| {
                hit.text = text.query_row(params![line_id], |row| row.get(0))?;
                Ok(hit)
            })
            .collect::<rusqlite::Result<_>>()?;
        Ok(Some(hits))
    }

    fn frame_range(&self, from: i64, to: i64) -> Result<Option<(i64, i64)>> {
        let first = self.frame_id_at(
            "SELECT id FROM frames WHERE ts >= ?1 ORDER BY ts ASC LIMIT 1",
            from,
        )?;
        let last = self.frame_id_at(
            "SELECT id FROM frames WHERE ts <= ?1 ORDER BY ts DESC LIMIT 1",
            to,
        )?;
        Ok(first.zip(last))
    }

    fn frame_id_at(&self, sql: &str, ts: i64) -> Result<Option<i64>> {
        Ok(self
            .conn
            .prepare_cached(sql)?
            .query_row(params![ts], |row| row.get(0))
            .optional()?)
    }

    fn search_speech(&self, expression: &str, from: i64, to: i64, cap: i64) -> Result<Vec<Hit>> {
        let mut statement = self.conn.prepare_cached(SPEECH)?;
        let rows = statement.query_map(params![expression, from, to, cap], |row| {
            let source: String = row.get(1)?;
            Ok(Hit {
                ts: row.get(0)?,
                source: if source == "mic" {
                    HitSource::Mic
                } else {
                    HitSource::System
                },
                app: String::new(),
                title: String::new(),
                url: String::new(),
                text: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{TEST_KEY, TempDb, common_words, fill_frames};

    /// The walk back from the newest frame and the lookup of every match return the same hits.
    #[test]
    fn walk_and_lookup_return_the_same_hits() {
        let db = TempDb::new("search-plans");
        let mut store = Store::open(&db.path, &TEST_KEY).expect("test setup");
        let now = 1_800_000_000;
        fill_frames(&mut store, now, 6, 80, 7).expect("test setup");

        let day = 86_400;
        let ranges = [
            (i64::MIN, i64::MAX),
            (now - 4 * day, i64::MAX),
            (i64::MIN, now - 2 * day),
            (now - 5 * day, now - 3 * day),
        ];
        let words = common_words(3);
        let queries = [
            words[..1].to_vec(),
            words[1..2].to_vec(),
            words[..2].to_vec(),
            vec!["nosuchword".to_string()],
        ];
        let mut found = 0;
        for terms in &queries {
            let expression = match_expression(terms);
            let matches = store.matching_lines(&expression).expect("matches");
            for (from, to) in ranges {
                let Some(range) = store.frame_range(from, to).expect("range") else {
                    continue;
                };
                for app in [None, Some("slack"), Some("Code")] {
                    for cap in [1, 5, 22, 200] {
                        let lookup = store
                            .search_screen_lookup(&expression, range, app, cap)
                            .expect("lookup plan");
                        let walk = store
                            .search_screen_walk(&matches, range, app, cap, usize::MAX)
                            .expect("walk plan")
                            .expect("budget is unlimited");
                        assert_eq!(walk, lookup, "{terms:?} {from}..{to} {app:?} cap {cap}");
                        found += lookup.len();
                    }
                }
            }
        }
        assert!(found > 0);
    }
}
