//! Recall: finds what you half remember. Each term is searched on its own and sessions are ranked
//! by how many terms they cover, counting the window title, so "what did I write to Anna about
//! pricing" finds the email whose title says Anna and whose body says pricing.

use crate::error::Result;
use crate::redact::redact;
use crate::sessions::{SESSION_GAP_S, Session};
use crate::settings::Settings;
use crate::store::Store;
use crate::text::words;

/// Newest hits read per term. Old matches of a common word fall off; a time window brings them back.
const PER_TERM: usize = 200;
const MAX_TERMS: usize = 8;

/// Question words and filler in English and German. Names, topics and numbers stay.
const FILLER: &[&str] = &[
    "a",
    "about",
    "again",
    "all",
    "am",
    "an",
    "and",
    "any",
    "are",
    "at",
    "be",
    "can",
    "could",
    "did",
    "do",
    "does",
    "for",
    "from",
    "had",
    "has",
    "have",
    "how",
    "i",
    "in",
    "is",
    "it",
    "me",
    "my",
    "of",
    "on",
    "or",
    "said",
    "say",
    "saw",
    "see",
    "seen",
    "send",
    "sent",
    "she",
    "that",
    "the",
    "there",
    "this",
    "to",
    "told",
    "was",
    "we",
    "were",
    "what",
    "when",
    "where",
    "which",
    "who",
    "why",
    "with",
    "write",
    "writing",
    "wrote",
    "written",
    "you",
    "he",
    "him",
    "her",
    "them",
    "they",
    "someone",
    "something",
    "find",
    "show",
    "last",
    "yesterday",
    "today",
    "week",
    "ago",
    "remember",
    "forgot",
    "message",
    "email",
    "mail",
    "auf",
    "aus",
    "bei",
    "das",
    "dem",
    "den",
    "der",
    "die",
    "du",
    "ein",
    "eine",
    "einen",
    "er",
    "es",
    "für",
    "gestern",
    "habe",
    "hat",
    "heute",
    "ich",
    "ihr",
    "im",
    "in",
    "ist",
    "mit",
    "nach",
    "noch",
    "sie",
    "über",
    "und",
    "von",
    "was",
    "wann",
    "wem",
    "wen",
    "wer",
    "wie",
    "wo",
    "woche",
    "zu",
    "geschrieben",
    "geschickt",
    "gesagt",
    "gesehen",
    "schreiben",
    "nachricht",
];

/// The words of `question` worth searching for, in order, without repeats.
pub fn keywords(question: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in words(question) {
        if !FILLER.contains(&word.as_str()) && !out.contains(&word) {
            out.push(word);
        }
    }
    out.truncate(MAX_TERMS);
    out
}

/// Sessions are not merged across terms, so this is the share of the terms one place covers.
fn coverage(session: &Session, terms: &[String]) -> usize {
    let mut found = words(&session.title);
    found.extend(words(&session.url));
    for (_, text) in &session.lines {
        found.extend(words(text));
    }
    terms.iter().filter(|term| found.contains(term)).count()
}

impl Store {
    /// Up to `limit` sessions that match any of `terms`, most terms covered first, then newest.
    /// Excluded apps never show up, as in `search_sessions`.
    pub fn recall(
        &self,
        settings: &Settings,
        terms: &[String],
        range: (Option<i64>, Option<i64>),
        limit: usize,
    ) -> Result<Vec<Session>> {
        let terms: Vec<String> = terms
            .iter()
            .flat_map(|term| words(&redact(term, &self.secret)))
            .collect();
        let mut sessions: Vec<Session> = Vec::new();
        for term in &terms {
            let page = self.search_sessions(settings, term, range, None, PER_TERM, 0)?;
            for found in page.sessions {
                let same = sessions.iter_mut().find(|s| {
                    s.source == found.source
                        && s.app == found.app
                        && s.title == found.title
                        && s.url == found.url
                        && s.start <= found.end + SESSION_GAP_S
                        && found.start <= s.end + SESSION_GAP_S
                });
                match same {
                    Some(session) => {
                        session.start = session.start.min(found.start);
                        session.end = session.end.max(found.end);
                        for line in found.lines {
                            if !session.lines.contains(&line) {
                                session.lines.push(line);
                            }
                        }
                        session.lines.sort_by_key(|(ts, _)| std::cmp::Reverse(*ts));
                    }
                    None => sessions.push(found),
                }
            }
        }
        let mut ranked: Vec<(usize, Session)> = sessions
            .into_iter()
            .map(|session| (coverage(&session, &terms), session))
            .collect();
        ranked.sort_by_key(|(covered, session)| std::cmp::Reverse((*covered, session.end)));
        Ok(ranked
            .into_iter()
            .take(limit)
            .map(|(_, session)| session)
            .collect())
    }
}
