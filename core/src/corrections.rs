//! §7.12 Corrections: fixed English and German phrases, stored per step key or workflow variant.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{Error, Result};
use crate::finder::Enjoyment;
use crate::redact::redact;
use crate::store::Store;
use crate::text::pattern;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    Enjoyment(Enjoyment),
    Client,
    NotAutomatable,
    Rename(String),
    Hide,
    /// "These belong together": two steps become one.
    Merge,
    /// "This is not one workflow": only identical step sequences stay together.
    Split,
}

impl Rule {
    fn name(&self) -> &'static str {
        match self {
            Self::Enjoyment(_) => "enjoyment",
            Self::Client => "client",
            Self::NotAutomatable => "not_automatable",
            Self::Rename(_) => "rename",
            Self::Hide => "hide",
            Self::Merge => "merge",
            Self::Split => "split",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A step key, as in `StepInfo::key`.
    Step(String),
    /// A workflow id, as in `Workflow::id`.
    Workflow(i64),
}

static RENAME: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"(?i)^\s*(?:rename (?:this |it )?to|call (?:this|it)|umbenennen (?:in|zu)|benenne (?:das |es )?um (?:in|zu)|nenne (?:das|es))\s+(.+?)\s*$",
    )
});

fn rules() -> &'static [(Regex, Rule)] {
    static RULES: LazyLock<Vec<(Regex, Rule)>> = LazyLock::new(|| {
        let rule = |source: &str, rule| (pattern(source), rule);
        vec![
            rule(
                r"not one workflow|isn't one workflow|not a single workflow|don't belong together|kein einzelner workflow|nicht ein workflow|sind (?:zwei|mehrere) workflows|gehört? nicht zusammen|gehören nicht zusammen",
                Rule::Split,
            ),
            rule(
                r"belong together|gehört zusammen|gehören zusammen",
                Rule::Merge,
            ),
            rule(
                r"\bhide (?:this|it)\b|\bausblenden\b|\bverstecke? (?:das|es)\b|\bblende (?:das|es) aus\b",
                Rule::Hide,
            ),
            rule(
                r"(?:can't|cannot|can not|won't) be automated|not automatable|can't automate|nicht automatisier\w*|kann nicht automatisiert|lässt sich nicht automatisieren",
                Rule::NotAutomatable,
            ),
            rule(
                r"for (?:a|my) client|client work|kundenarbeit|kundenprojekt|für (?:einen|meinen|eine|meine) kund\w*",
                Rule::Client,
            ),
            rule(
                r"\bi hate\b|hate this|hate it|can't stand|ich hasse|hasse (?:das|es)\b",
                Rule::Enjoyment(Enjoyment::Hate),
            ),
            rule(
                r"\bi love\b|love this|love it|ich liebe",
                Rule::Enjoyment(Enjoyment::Love),
            ),
            rule(
                r"\bi dislike\b|don't like|ich mag (?:das|es|diese\w*) nicht|mag ich nicht|\bnervt\b",
                Rule::Enjoyment(Enjoyment::Dislike),
            ),
            rule(
                r"don't mind|it's fine|it's ok|it's okay|\bneutral\b|ist mir egal|stört mich nicht",
                Rule::Enjoyment(Enjoyment::Neutral),
            ),
            rule(
                r"\bi like\b|\bi enjoy\b|ich mag|gefällt mir|macht mir spaß",
                Rule::Enjoyment(Enjoyment::Like),
            ),
        ]
    });
    &RULES
}

/// The rule a phrase matches, if any. Rename keeps the new name in its original case.
pub fn parse(phrase: &str) -> Option<Rule> {
    if let Some(caps) = RENAME.captures(phrase) {
        let name = caps[1].trim_matches(|c: char| {
            c == '"' || c == '“' || c == '”' || c == '\'' || c == '.' || c == '!'
        });
        return (!name.is_empty()).then(|| Rule::Rename(name.to_string()));
    }
    let lower = phrase.to_lowercase().replace('’', "'");
    rules()
        .iter()
        .find(|(regex, _)| regex.is_match(&lower))
        .map(|(_, rule)| rule.clone())
}

/// Applied corrections, read at the start of every refresh.
#[derive(Default)]
pub(crate) struct Overrides(HashMap<(String, String), String>);

impl Overrides {
    pub fn load(conn: &Connection) -> Result<Self> {
        let mut statement = conn.prepare("SELECT key, field, value FROM overrides")?;
        let rows = statement.query_map([], |row| Ok(((row.get(0)?, row.get(1)?), row.get(2)?)))?;
        Ok(Self(rows.collect::<std::result::Result<_, _>>()?))
    }

    pub fn get(&self, key: &str, field: &str) -> Option<&str> {
        self.0
            .get(&(key.to_string(), field.to_string()))
            .map(String::as_str)
    }

    pub fn flag(&self, key: &str, field: &str) -> bool {
        self.get(key, field) == Some("1")
    }

    /// key -> value for every override of one field.
    pub fn field(&self, field: &str) -> HashMap<String, String> {
        self.0
            .iter()
            .filter(|((_, f), _)| f == field)
            .map(|((k, _), v)| (k.clone(), v.clone()))
            .collect()
    }

    pub fn keys_with(&self, field: &str) -> HashSet<String> {
        self.field(field).into_keys().collect()
    }
}

impl Store {
    /// Applies a user phrase. Merge takes two steps (the second joins the first); Client, NotAutomatable,
    /// Hide and Split take a workflow; Enjoyment and Rename take a step or a workflow.
    /// Takes effect at the next `refresh`.
    pub fn correct(&mut self, ts: i64, phrase: &str, targets: &[Target]) -> Result<Rule> {
        let rule = parse(phrase).ok_or(Error::UnknownCorrection)?;
        let resolved: Vec<(String, bool)> = targets
            .iter()
            .map(|target| match target {
                Target::Step(key) => Ok((key.clone(), true)),
                Target::Workflow(id) => self
                    .conn
                    .query_row(
                        "SELECT variant FROM workflows WHERE id = ?1",
                        params![id],
                        |row| row.get(0),
                    )
                    .optional()?
                    .map(|variant| (variant, false))
                    .ok_or(Error::NoWorkflow(*id)),
            })
            .collect::<Result<_>>()?;
        let one = |need_workflow: bool| -> Result<&str> {
            match resolved.as_slice() {
                [(key, is_step)] if !(need_workflow && *is_step) => Ok(key),
                _ if need_workflow => Err(Error::BadTarget("exactly one workflow")),
                _ => Err(Error::BadTarget("exactly one step or workflow")),
            }
        };
        let (key, field, value): (&str, &str, String) = match &rule {
            Rule::Enjoyment(e) => (one(false)?, "enjoyment", e.as_str().into()),
            Rule::Rename(name) => (one(false)?, "name", name.clone()),
            Rule::Client => (one(true)?, "client", "1".into()),
            Rule::NotAutomatable => (one(true)?, "not_automatable", "1".into()),
            Rule::Hide => (one(true)?, "hidden", "1".into()),
            Rule::Split => (one(true)?, "split", "1".into()),
            Rule::Merge => match resolved.as_slice() {
                [(first, true), (second, true)] => (second, "merge_into", first.clone()),
                _ => return Err(Error::BadTarget("exactly two steps")),
            },
        };
        let (phrase, value) = (redact(phrase, &self.secret), redact(&value, &self.secret));
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO corrections(ts, phrase, rule, key, arg) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ts, phrase, rule.name(), key, value],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO overrides(key, field, value) VALUES (?1, ?2, ?3)",
            params![key, field, value],
        )?;
        tx.commit()?;
        Ok(rule)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_and_german_phrases_parse() {
        let cases = [
            ("I hate this", Rule::Enjoyment(Enjoyment::Hate)),
            ("Ich hasse das", Rule::Enjoyment(Enjoyment::Hate)),
            ("I don’t like it", Rule::Enjoyment(Enjoyment::Dislike)),
            ("ich mag das nicht", Rule::Enjoyment(Enjoyment::Dislike)),
            ("I like this", Rule::Enjoyment(Enjoyment::Like)),
            ("I love this part", Rule::Enjoyment(Enjoyment::Love)),
            ("It's for a client", Rule::Client),
            ("Das ist für einen Kunden", Rule::Client),
            ("This can't be automated", Rule::NotAutomatable),
            ("Das lässt sich nicht automatisieren", Rule::NotAutomatable),
            ("Rename to Invoice run", Rule::Rename("Invoice run".into())),
            (
                "Benenne das um in \"Rechnungslauf\"",
                Rule::Rename("Rechnungslauf".into()),
            ),
            ("hide this", Rule::Hide),
            ("Bitte ausblenden", Rule::Hide),
            ("These belong together", Rule::Merge),
            ("Das gehört zusammen", Rule::Merge),
            ("This is not one workflow", Rule::Split),
            ("Das sind zwei Workflows", Rule::Split),
        ];
        for (phrase, rule) in cases {
            assert_eq!(parse(phrase), Some(rule), "{phrase}");
        }
        assert_eq!(parse("what time is it"), None);
    }

    #[test]
    fn stored_phrase_and_value_are_redacted() {
        let db = crate::fixtures::TempDb::new("correction-redact");
        let mut store = db.open().expect("test setup");
        store
            .correct(
                1,
                "Rename to Invoices for anna@example.com",
                &[Target::Step("mail".into())],
            )
            .expect("test setup");
        let (phrase, arg): (String, String) = store
            .conn
            .query_row("SELECT phrase, arg FROM corrections", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .expect("test setup");
        let value: String = store
            .conn
            .query_row("SELECT value FROM overrides", [], |row| row.get(0))
            .expect("test setup");
        for stored in [phrase, arg, value] {
            assert!(!stored.contains("anna@example.com"), "{stored}");
            assert!(stored.contains("<email:"), "{stored}");
        }
    }
}
