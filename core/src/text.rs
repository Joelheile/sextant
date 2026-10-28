//! Small text helpers shared by redaction, keys and entity extraction.

use regex::Regex;

/// Compiles a pattern that is a literal in this crate, so it cannot fail at run time.
pub(crate) fn pattern(source: &str) -> Regex {
    Regex::new(source).expect("static pattern compiles")
}

const STOP: &[&str] = &[
    "the", "and", "for", "from", "with", "to", "of", "in", "on", "at", "an", "is", "der", "die",
    "das", "und", "von", "mit", "für", "im", "am", "zu", "den", "dem", "ein", "eine",
];

/// FNV-1a, 64 bit. Stable across platforms, used for line hashes and email hashes.
pub(crate) fn fnv1a(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// SipHash-2-4 with a 128-bit key: a keyed hash, unlike FNV, so output reveals nothing about
/// the input to anyone without the key.
pub(crate) fn siphash24(key: &[u8; 16], data: &[u8]) -> u64 {
    fn le(bytes: &[u8]) -> u64 {
        bytes
            .iter()
            .rev()
            .fold(0, |acc, &b| acc << 8 | u64::from(b))
    }
    fn round(v: &mut [u64; 4]) {
        v[0] = v[0].wrapping_add(v[1]);
        v[1] = v[1].rotate_left(13) ^ v[0];
        v[0] = v[0].rotate_left(32);
        v[2] = v[2].wrapping_add(v[3]);
        v[3] = v[3].rotate_left(16) ^ v[2];
        v[0] = v[0].wrapping_add(v[3]);
        v[3] = v[3].rotate_left(21) ^ v[0];
        v[2] = v[2].wrapping_add(v[1]);
        v[1] = v[1].rotate_left(17) ^ v[2];
        v[2] = v[2].rotate_left(32);
    }
    let (k0, k1) = (le(&key[..8]), le(&key[8..]));
    let mut v = [
        k0 ^ 0x736f_6d65_7073_6575,
        k1 ^ 0x646f_7261_6e64_6f6d,
        k0 ^ 0x6c79_6765_6e65_7261,
        k1 ^ 0x7465_6462_7974_6573,
    ];
    let mut blocks = data.chunks_exact(8);
    for block in &mut blocks {
        let m = le(block);
        v[3] ^= m;
        round(&mut v);
        round(&mut v);
        v[0] ^= m;
    }
    let last = (data.len() as u64) << 56 | le(blocks.remainder());
    v[3] ^= last;
    round(&mut v);
    round(&mut v);
    v[0] ^= last;
    v[2] ^= 0xff;
    for _ in 0..4 {
        round(&mut v);
    }
    v[0] ^ v[1] ^ v[2] ^ v[3]
}

/// Lowercase alphanumeric words.
pub(crate) fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Words without digits, two letters or more, no stop words, in order, without repeats.
pub(crate) fn tokens(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in words(text) {
        let keep = word.chars().count() >= 2
            && !word.chars().any(|c| c.is_ascii_digit())
            && !STOP.contains(&word.as_str());
        if keep && !out.contains(&word) {
            out.push(word);
        }
    }
    out
}

/// Words with digits become `#`, runs of `#` collapse: "Invoice INV-2026-0412" -> "invoice inv #".
pub(crate) fn template(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for word in words(text) {
        let word = if word.chars().any(|c| c.is_ascii_digit()) {
            "#".to_string()
        } else {
            word
        };
        if word == "#" && out.last().is_some_and(|last| last == "#") {
            continue;
        }
        out.push(word);
    }
    out.join(" ")
}

pub(crate) fn jaccard(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let shared = a.iter().filter(|t| b.contains(t)).count();
    shared as f64 / (a.len() + b.len() - shared) as f64
}

/// Splits a URL into lowercase host (no `www.`, no port) and path segments. Query and fragment are dropped.
pub(crate) fn split_url(url: &str) -> (String, Vec<String>) {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let mut parts = rest.split('/');
    let host = parts.next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("").to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    let path = parts
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    (host, path)
}

/// `host/path` without scheme, query or fragment, as stored in `windows.url_host_path`.
pub(crate) fn host_path(url: &str) -> String {
    let (host, path) = split_url(url);
    if path.is_empty() {
        host
    } else {
        format!("{host}/{}", path.join("/"))
    }
}

pub(crate) fn slug(text: &str) -> String {
    let mut out = String::new();
    for word in words(text) {
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&word);
    }
    out.chars()
        .take(48)
        .collect::<String>()
        .trim_end_matches('-')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn siphash_matches_the_reference_vectors() {
        let key: [u8; 16] = std::array::from_fn(|i| i as u8);
        let input: Vec<u8> = (0..15).collect();
        assert_eq!(siphash24(&key, &[]), 0x726f_db47_dd0e_0e31);
        assert_eq!(siphash24(&key, &input), 0xa129_ca61_49be_45e5);
    }

    #[test]
    fn template_replaces_numbers_and_ids() {
        assert_eq!(
            template("Invoice INV-2026-0412 attached"),
            "invoice inv # attached"
        );
        assert_eq!(template("Rechnung RE-2026-0415"), "rechnung re #");
    }

    #[test]
    fn tokens_drop_digits_and_stop_words() {
        assert_eq!(
            tokens("Invoice INV-2026-0412 from Acme"),
            ["invoice", "inv", "acme"]
        );
    }

    #[test]
    fn urls_split_into_host_and_path() {
        let (host, path) = split_url("https://www.Example.com:8080/a/b?q=1#x");
        assert_eq!(host, "example.com");
        assert_eq!(path, ["a", "b"]);
        assert_eq!(
            host_path("https://app.lexoffice.de/vouchers/INV-1?x=1"),
            "app.lexoffice.de/vouchers/INV-1"
        );
    }
}
