//! §7.3 Entities and §7.5 cases. Rules only, no model.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use super::activities::Activity;
use crate::text::{pattern, text_enum};

/// Declaration order sorts entities by kind.
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

static INVOICE: LazyLock<Regex> = LazyLock::new(|| pattern(r"(?i)\b(?:INV|RE|RG)-\d{4}-\d{3,6}\b"));
static TICKET: LazyLock<Regex> = LazyLock::new(|| pattern(r"\b[A-Z][A-Z0-9]{1,5}-\d{1,6}\b"));
static EMAIL: LazyLock<Regex> = LazyLock::new(|| pattern(r"<email:([0-9a-f]{16})>"));
static COMPANY: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"\b\p{Lu}[\p{L}&-]+ (?:GmbH|AG|UG|KG|SE|Ltd|Inc|LLC|Corp)\b"));
static PERSON: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"\b(?:with|mit) (\p{Lu}\p{Ll}+ \p{Lu}\p{Ll}+)\b"));
static AMOUNT: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"[€$£] ?\d[\d.,]*\d|\d[\d.,]*\d ?(?:€|EUR|USD|GBP|\$|£)"));
static ADDRESS: LazyLock<Regex> = LazyLock::new(|| pattern(r"^[\w.+-]+@[\w-]+(?:\.[\w-]+)+$"));
static DATE: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"^(?:\d{4}-\d{2}-\d{2}|\d{1,2}\.\d{1,2}\.\d{4})$"));
static NUMBER: LazyLock<Regex> = LazyLock::new(|| pattern(r"^\d[\d.,]*$"));

text_enum!(ValueType {
    Email => "email",
    Id => "id",
    Amount => "amount",
    Date => "date",
    Number => "number",
    ShortText => "short_text",
    LongText => "long_text",
});

/// The type of an edit value, from its shape. Text over 80 characters is long, the label limit.
pub(crate) fn value_type(value: &str) -> ValueType {
    let whole = |re: &Regex| {
        re.find(value)
            .is_some_and(|m| m.range() == (0..value.len()))
    };
    if whole(&EMAIL) || whole(&ADDRESS) {
        ValueType::Email
    } else if whole(&INVOICE) || whole(&TICKET) {
        ValueType::Id
    } else if whole(&AMOUNT) {
        ValueType::Amount
    } else if whole(&DATE) {
        ValueType::Date
    } else if whole(&NUMBER) {
        ValueType::Number
    } else if value.chars().count() > 80 {
        ValueType::LongText
    } else {
        ValueType::ShortText
    }
}

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
    pub activities: Vec<usize>,
}

/// Entities sorted by (kind, value). Companies and persons need 3+ activities.
/// With 20 or more activities, entities in more than a fifth of them are dropped.
pub(crate) fn build(activities: &[Activity]) -> Vec<Entity> {
    let mut seen: BTreeMap<(EntityKind, String), Vec<usize>> = BTreeMap::new();
    for (index, activity) in activities.iter().enumerate() {
        let mut found = find(&activity.text);
        if !activity.doc.is_empty() {
            found.push((EntityKind::Doc, activity.doc.clone()));
        }
        for entity in found {
            let list = seen.entry(entity).or_default();
            if list.last() != Some(&index) {
                list.push(index);
            }
        }
    }
    let total = activities.len();
    seen.into_iter()
        .filter(|((kind, _), list)| {
            let rare = !matches!(kind, EntityKind::Company | EntityKind::Person) || list.len() >= 3;
            let common = total >= 20 && list.len() * 5 > total;
            rare && !common
        })
        .map(|((kind, value), activities)| Entity {
            kind,
            value,
            activities,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_ids_companies_and_amounts() {
        let found = find(
            "Rechnung INV-2026-0412 Acme GmbH ACME-77 €1,250.00 und 1.250,00 € <email:0a1b2c3d4e5f6071>",
        );
        let kinds: Vec<_> = found.iter().map(|(k, v)| format!("{k:?}:{v}")).collect();
        assert_eq!(
            kinds,
            [
                "Invoice:INV-2026-0412",
                "Ticket:ACME-77",
                "Email:0a1b2c3d4e5f6071",
                "Company:Acme GmbH",
                "Amount:125000 EUR",
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

    #[test]
    fn value_types_follow_the_shape_of_the_value() {
        use ValueType::*;
        assert_eq!(value_type("anna.keller@acme-gmbh.de"), Email);
        assert_eq!(value_type("<email:0a1b2c3d4e5f6071>"), Email);
        assert_eq!(value_type("INV-2026-0412"), Id);
        assert_eq!(value_type("OPS-41"), Id);
        assert_eq!(value_type("€1,250.00"), Amount);
        assert_eq!(value_type("1.980,00 €"), Amount);
        assert_eq!(value_type("2026-03-15"), Date);
        assert_eq!(value_type("42"), Number);
        assert_eq!(value_type("Lunch at noon?"), ShortText);
        assert_eq!(value_type(&"text ".repeat(20)), LongText);
    }
}
