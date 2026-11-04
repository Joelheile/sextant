//! Search hits grouped into sessions for the MCP `search` tool: the same screen or speaker within
//! ten minutes, with excluded apps skipped.

use crate::error::Result;
use crate::search::{Hit, HitSource};
use crate::settings::Settings;
use crate::store::Store;

pub(crate) const SESSION_GAP_S: i64 = 600;
/// Most batches read while skipping excluded apps.
const MAX_BATCHES: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub source: HitSource,
    pub app: String,
    pub title: String,
    pub url: String,
    pub start: i64,
    pub end: i64,
    /// Time and text, newest first.
    pub lines: Vec<(i64, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPage {
    /// Lines on this page.
    pub lines: usize,
    pub sessions: Vec<Session>,
    /// Pass as `offset` for the next page; `None` on the last one.
    pub next_offset: Option<usize>,
}

impl Store {
    /// One page of up to `limit` matching lines, newest first, without lines from excluded apps,
    /// grouped into sessions.
    pub fn search_sessions(
        &self,
        settings: &Settings,
        query: &str,
        range: (Option<i64>, Option<i64>),
        app: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<SessionPage> {
        let mut hits: Vec<Hit> = Vec::new();
        let mut raw = offset;
        for _ in 0..MAX_BATCHES {
            let batch = self.search(query, range.0, range.1, app, limit, raw)?;
            let batch_len = batch.len();
            for hit in batch {
                raw += 1;
                if !settings.is_excluded(&hit.app) {
                    hits.push(hit);
                }
                if hits.len() == limit {
                    break;
                }
            }
            if hits.len() == limit || batch_len < limit {
                break;
            }
        }

        let mut sessions: Vec<Session> = Vec::new();
        for hit in &hits {
            let joined = sessions.iter_mut().find(|s| {
                s.source == hit.source
                    && s.app == hit.app
                    && s.title == hit.title
                    && s.url == hit.url
                    && hit.ts >= s.start - SESSION_GAP_S
                    && hit.ts <= s.end + SESSION_GAP_S
            });
            match joined {
                Some(session) => {
                    session.start = session.start.min(hit.ts);
                    session.end = session.end.max(hit.ts);
                    session.lines.push((hit.ts, hit.text.clone()));
                }
                None => sessions.push(Session {
                    source: hit.source,
                    app: hit.app.clone(),
                    title: hit.title.clone(),
                    url: hit.url.clone(),
                    start: hit.ts,
                    end: hit.ts,
                    lines: vec![(hit.ts, hit.text.clone())],
                }),
            }
        }
        Ok(SessionPage {
            lines: hits.len(),
            sessions,
            next_offset: (hits.len() == limit).then_some(raw),
        })
    }
}
