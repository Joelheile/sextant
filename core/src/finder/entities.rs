//! §7.3 Entities and §7.5 cases. Rules only, no model.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use super::episodes::Episode;
use crate::text::pattern;

/// Declaration order is case priority: an episode belongs to the case of its strongest entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum EntityKind {
    Invoice,
    Ticket,
    Email,
    Doc,
    Company,
    Person,
    Amount,
}

impl EntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Invoice => "invoice",
            Self::Ticket => "ticket",
            Self::Email => "email",
            Self::Doc => "doc",
            Self::Company => "company",
            Self::Person => "person",
            Self::Amount => "amount",
        }
    }
}

static INVOICE: LazyLock<Regex> = LazyLock::new(|| pattern(r"(?i)\b(?:INV|RE|RG)-\d{4}-\d{3,6}\b"));
static TICKET: LazyLock<Regex> = LazyLock::new(|| pattern(r"\b[A-Z][A-Z0-9]{1,5}-\d{1,6}\b"));
static EMAIL: LazyLock<Regex> = LazyLock::new(|| pattern(r"<email:([0-9a-f]{8})>"));
static COMPANY: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"\b\p{Lu}[\p{L}&-]+ (?:GmbH|AG|UG|KG|SE|Ltd|Inc|LLC|Corp)\b"));
static PERSON: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"\b(?:with|mit) (\p{Lu}\p{Ll}+ \p{Lu}\p{Ll}+)\b"));
static AMOUNT: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"[€$£] ?\d[\d.,]*\d|\d[\d.,]*\d ?(?:€|EUR|USD|GBP|\$|£)"));

/// Entities in a piece of text. Invoice ids are removed before ticket ids are searched, and both before amounts.
pub(crate) fn find(text: &str) -> Vec<(EntityKind, String)> {
    let mut found = Vec::new();
    for hit in INVOICE.find_iter(text) {
        found.push((EntityKind::Invoice, hit.as_str().to_uppercase()));
    }
    let rest = INVOICE.replace_all(text, " ");
    for hit in TICKET.find_iter(&rest) {
        found.push((EntityKind::Ticket, hit.as_str().to_string()));
    }
    for caps in EMAIL.captures_iter(text) {
        found.push((EntityKind::Email, caps[1].to_string()));
    }
    for hit in COMPANY.find_iter(text) {
        found.push((EntityKind::Company, hit.as_str().to_string()));
    }
    for caps in PERSON.captures_iter(text) {
        found.push((EntityKind::Person, caps[1].to_string()));
    }
    let rest = TICKET.replace_all(&rest, " ");
    for hit in AMOUNT.find_iter(&rest) {
        if let Some(value) = normalize_amount(hit.as_str()) {
            found.push((EntityKind::Amount, value));
        }
    }
    found.sort();
    found.dedup();
    found
}

/// "€1,250.00" and "1.250,00 €" both become "125000 EUR".
fn normalize_amount(text: &str) -> Option<String> {
    let currency = if text.contains('€') || text.contains("EUR") {
        "EUR"
    } else if text.contains('$') || text.contains("USD") {
        "USD"
    } else {
        "GBP"
    };
    let number: String = text
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
        .collect();
    let (whole, decimals) = match number.rfind(['.', ',']) {
        Some(at) if number.len() - at - 1 <= 2 => (&number[..at], &number[at + 1..]),
        _ => (number.as_str(), ""),
    };
    let whole: String = whole.chars().filter(char::is_ascii_digit).collect();
    let cents: u64 = format!("{whole}{decimals:0<2}").parse().ok()?;
    Some(format!("{cents} {currency}"))
}

pub(crate) struct Entity {
    pub kind: EntityKind,
    pub value: String,
    pub episodes: Vec<usize>,
}

pub(crate) struct Case {
    /// Index into the entity list.
    pub entity: usize,
    pub episodes: Vec<usize>,
    /// Step indexes in time order, consecutive repeats collapsed.
    pub seq: Vec<usize>,
    pub first: i64,
    pub last: i64,
    pub workflow: Option<usize>,
}

/// Entities sorted by (kind, value), and cases: episodes grouped by their strongest entity, 2+ episodes.
/// Companies and persons need 3+ episodes. Entities in more than a fifth of all episodes are dropped.
pub(crate) fn build(episodes: &[Episode]) -> (Vec<Entity>, Vec<Case>) {
    let mut seen: BTreeMap<(EntityKind, String), Vec<usize>> = BTreeMap::new();
    for (index, episode) in episodes.iter().enumerate() {
        let mut found = find(&episode.text);
        if !episode.doc.is_empty() {
            found.push((EntityKind::Doc, episode.doc.clone()));
        }
        for entity in found {
            let list = seen.entry(entity).or_default();
            if list.last() != Some(&index) {
                list.push(index);
            }
        }
    }
    let total = episodes.len();
    let entities: Vec<Entity> = seen
        .into_iter()
        .filter(|((kind, _), list)| {
            let rare = !matches!(kind, EntityKind::Company | EntityKind::Person) || list.len() >= 3;
            let common = total >= 20 && list.len() * 5 > total;
            rare && !common
        })
        .map(|((kind, value), episodes)| Entity {
            kind,
            value,
            episodes,
        })
        .collect();

    let mut strongest: Vec<Option<usize>> = vec![None; total];
    for (entity_index, entity) in entities.iter().enumerate().rev() {
        for &episode in &entity.episodes {
            strongest[episode] = Some(entity_index);
        }
    }
    let mut grouped: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (episode, entity) in strongest.into_iter().enumerate() {
        if let Some(entity) = entity {
            grouped.entry(entity).or_default().push(episode);
        }
    }
    let cases = grouped
        .into_iter()
        .filter(|(_, eps)| eps.len() >= 2)
        .map(|(entity, eps)| {
            let mut seq: Vec<usize> = Vec::new();
            for &episode in &eps {
                if seq.last() != Some(&episodes[episode].step) {
                    seq.push(episodes[episode].step);
                }
            }
            let first = episodes[eps[0]].start;
            let last = eps.iter().map(|&e| episodes[e].end).max().unwrap_or(first);
            Case {
                entity,
                episodes: eps,
                seq,
                first,
                last,
                workflow: None,
            }
        })
        .collect();
    (entities, cases)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_ids_companies_and_amounts() {
        let found = find(
            "Rechnung INV-2026-0412 Acme GmbH ACME-77 €1,250.00 und 1.250,00 € <email:0a1b2c3d>",
        );
        let kinds: Vec<_> = found
            .iter()
            .map(|(k, v)| format!("{}:{v}", k.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [
                "invoice:INV-2026-0412",
                "ticket:ACME-77",
                "email:0a1b2c3d",
                "company:Acme GmbH",
                "amount:125000 EUR",
            ]
        );
    }

    #[test]
    fn invoice_ids_are_not_tickets() {
        assert!(
            find("RE-2026-0415")
                .iter()
                .all(|(k, _)| *k == EntityKind::Invoice)
        );
    }

    #[test]
    fn persons_follow_with_or_mit() {
        assert_eq!(find("Call with Anna Keller")[0].1, "Anna Keller");
    }
}
