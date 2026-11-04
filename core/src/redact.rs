//! Redaction before storage. Emails become `<email:hash>` so entity linking still works.

use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::text::{pattern, siphash24};

static API_KEY: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"\b(?:sk[-_][A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{20,}|AKIA[0-9A-Z]{16}|xox[abprs]-[A-Za-z0-9-]{10,}|AIza[0-9A-Za-z_-]{35})",
    )
});
static JWT: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}"));
static BEARER: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{16,}"));
static SECRET: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"(?i)\b(token|secret|password|passwd|pwd|api[_-]?key)(\s*[:=]\s*)[^\s<]{6,}")
});
static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"));
static IBAN: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"\b[A-Z]{2}\d{2}(?: ?[A-Z0-9]{4}){2,7}(?: ?[A-Z0-9]{1,3})?\b"));
static CARD: LazyLock<Regex> = LazyLock::new(|| pattern(r"\b\d(?:[ -]?\d){12,18}\b"));

/// Per-install key for email hashes. Without it `<email:hash>` cannot be matched to an address.
pub type Secret = [u8; 16];

/// Replaces emails, IBANs, card numbers, API keys and tokens. Idempotent.
pub fn redact(text: &str, secret: &Secret) -> String {
    let text = API_KEY.replace_all(text, "<apikey>");
    let text = JWT.replace_all(&text, "<token>");
    let text = BEARER.replace_all(&text, "Bearer <token>");
    let text = SECRET.replace_all(&text, "$1$2<token>");
    let text = EMAIL.replace_all(&text, |caps: &Captures| email_tag(&caps[0], secret));
    let text = IBAN.replace_all(&text, |caps: &Captures| {
        if valid_iban(&caps[0]) {
            "<iban>".to_string()
        } else {
            caps[0].to_string()
        }
    });
    let text = CARD.replace_all(&text, |caps: &Captures| {
        if luhn(&caps[0]) {
            "<card>".to_string()
        } else {
            caps[0].to_string()
        }
    });
    text.into_owned()
}

fn email_tag(email: &str, secret: &Secret) -> String {
    let hash = siphash24(secret, email.to_lowercase().as_bytes());
    format!("<email:{hash:016x}>")
}

fn luhn(text: &str) -> bool {
    let digits: Vec<u32> = text.chars().filter_map(|c| c.to_digit(10)).collect();
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| {
            if i % 2 == 1 {
                if d * 2 > 9 { d * 2 - 9 } else { d * 2 }
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

fn valid_iban(text: &str) -> bool {
    let compact: Vec<char> = text.chars().filter(|c| *c != ' ').collect();
    if !(15..=34).contains(&compact.len()) {
        return false;
    }
    let mut remainder = 0u32;
    for c in compact[4..].iter().chain(&compact[..4]) {
        let Some(value) = c.to_digit(36) else {
            return false;
        };
        remainder = if value > 9 {
            (remainder * 100 + value) % 97
        } else {
            (remainder * 10 + value) % 97
        };
    }
    remainder == 1
}

/// Redacts every string inside a JSON value. `url` strings also lose their query and fragment.
pub fn redact_json(value: &mut serde_json::Value, secret: &Secret) {
    match value {
        serde_json::Value::String(text) => *text = redact(text, secret),
        serde_json::Value::Array(items) => {
            items.iter_mut().for_each(|item| redact_json(item, secret))
        }
        serde_json::Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                match item {
                    serde_json::Value::String(url) if key == "url" => {
                        *url = redact(&crate::text::host_path(url), secret);
                    }
                    _ => redact_json(item, secret),
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: Secret = [3; 16];

    #[test]
    fn emails_keep_a_stable_hash() {
        let a = redact("Mail anna@Example.com now", &SECRET);
        let b = redact("anna@example.com", &SECRET);
        assert!(a.starts_with("Mail <email:"));
        assert_eq!(a.trim_start_matches("Mail ").trim_end_matches(" now"), b);
        assert!(!a.contains("anna"));
    }

    #[test]
    fn email_hash_is_keyed_and_64_bits() {
        let a = redact("anna@example.com", &[1; 16]);
        let b = redact("anna@example.com", &[2; 16]);
        assert_ne!(a, b);
        assert_eq!(a.len(), "<email:>".len() + 16);
    }

    #[test]
    fn iban_and_card_need_valid_checksums() {
        assert_eq!(
            redact("IBAN DE89 3704 0044 0532 0130 00 ok", &SECRET),
            "IBAN <iban> ok"
        );
        assert_eq!(redact("Card 4111 1111 1111 1111", &SECRET), "Card <card>");
        assert_eq!(
            redact("Order 1234 5678 9012 3456", &SECRET),
            "Order 1234 5678 9012 3456"
        );
        assert_eq!(
            redact("DE00 3704 0044 0532 0130 00", &SECRET),
            "DE00 3704 0044 0532 0130 00"
        );
    }

    #[test]
    fn keys_and_tokens_are_removed() {
        let text = "export KEY=sk-proj-abcdefghijklmnopqrstuvwx and ghp_abcdefghijklmnopqrstuvwxyz0123456789 \
                    Authorization: Bearer abcdefghijklmnop12345678 password: hunter2hunter2";
        let out = redact(text, &SECRET);
        assert!(!out.contains("abcdefghijkl"), "{out}");
        assert!(!out.contains("hunter2"), "{out}");
        assert!(
            out.contains("<apikey>")
                && out.contains("Bearer <token>")
                && out.contains("password: <token>")
        );
    }

    #[test]
    fn redaction_is_idempotent() {
        let once = redact(
            "a@b.de DE89 3704 0044 0532 0130 00 4111111111111111",
            &SECRET,
        );
        assert_eq!(redact(&once, &SECRET), once);
    }

    #[test]
    fn json_strings_and_urls_are_cleaned() {
        let mut value =
            serde_json::json!({"title": "to a@b.de", "url": "https://x.com/p?token=abc#z", "n": 1});
        redact_json(&mut value, &SECRET);
        assert_eq!(value["url"], "x.com/p");
        assert!(
            value["title"]
                .as_str()
                .is_some_and(|t| t.starts_with("to <email:"))
        );
    }
}
