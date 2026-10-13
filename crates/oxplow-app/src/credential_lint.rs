//! A shared extension travels with the repository, so a secret written into
//! it travels too. `extension check` reads the text of its manifest, scripts
//! and fixtures for what looks like one: a token with a well-known prefix, a
//! private-key header, or a long high-entropy literal. A finding names the
//! file and line and a masked form; the value is never printed. It is a
//! warning — a hash in a fixture is not a credential — and a keychain
//! `credentials:` entry on a collector or provider is the way to hold a real
//! one.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;

/// One suspicious spot: the 1-based line and what it looks like, masked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suspect {
    pub line: usize,
    pub what: String,
}

/// Tokens that start with a known issuer's prefix, with the shape that
/// separates a real token from prose that merely mentions the prefix.
static PREFIXED: LazyLock<Vec<(&'static str, &'static str, Regex)>> = LazyLock::new(|| {
    [
        ("a GitHub token", "gh?_", r"\bgh[pousr]_[A-Za-z0-9]{30,}"),
        (
            "a GitHub token",
            "github_pat_",
            r"\bgithub_pat_[A-Za-z0-9_]{22,}",
        ),
        (
            "an Anthropic API key",
            "sk-ant-",
            r"\bsk-ant-[A-Za-z0-9_-]{20,}",
        ),
        ("an API key", "sk-", r"\bsk-[A-Za-z0-9][A-Za-z0-9_-]{19,}"),
        ("a Slack token", "xox?-", r"\bxox[bpas]-[A-Za-z0-9-]{10,}"),
        (
            "an AWS access key id",
            "AKIA",
            r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b",
        ),
        ("a GitLab token", "glpat-", r"\bglpat-[A-Za-z0-9_-]{20,}"),
        ("a Google API key", "AIza", r"\bAIza[0-9A-Za-z_-]{35}"),
        (
            "a Stripe key",
            "sk_",
            r"\b[sr]k_(?:live|test)_[0-9A-Za-z]{16,}",
        ),
    ]
    .into_iter()
    .map(|(what, shown, re)| (what, shown, Regex::new(re).expect("a fixed pattern")))
    .collect()
});

static PRIVATE_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY-----").expect("a fixed pattern"));

/// A run of the characters base64, base64url and hex use.
static LONG_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9+/_=-]{32,}").expect("a fixed pattern"));

/// Shannon entropy of `s`, in bits per character.
fn entropy(s: &str) -> f64 {
    let mut counts = std::collections::HashMap::new();
    for c in s.chars() {
        *counts.entry(c).or_insert(0usize) += 1;
    }
    let n = s.chars().count() as f64;
    counts
        .values()
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum()
}

/// A long literal that is random-looking: mixed letters and digits, no
/// shape of its own. Hex digests (an md5, a sha1, a sha256 — ids and hashes
/// a fixture legitimately holds) and uuids don't count.
fn high_entropy(token: &str) -> Option<String> {
    let is_hex = token.chars().all(|c| c.is_ascii_hexdigit());
    if is_hex && matches!(token.len(), 32 | 40 | 64) {
        return None;
    }
    let has = |f: fn(&char) -> bool| token.chars().any(|c| f(&c));
    if !(has(char::is_ascii_digit) && has(char::is_ascii_alphabetic)) {
        return None;
    }
    let bits = entropy(token);
    let bar = if is_hex { 3.0 } else { 4.3 };
    (bits >= bar).then(|| {
        format!(
            "a {}-character high-entropy string ({bits:.1} bits per character)",
            token.len()
        )
    })
}

fn overlaps(covered: &[std::ops::Range<usize>], r: std::ops::Range<usize>) -> bool {
    covered.iter().any(|c| c.start < r.end && r.start < c.end)
}

/// The lines of `text` that look like they hold a credential. A prefixed
/// token is shown as its issuer's prefix and a length; nothing else of it
/// is repeated.
pub fn suspects(text: &str) -> Vec<Suspect> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line_no = i + 1;
        let mut found = BTreeSet::new();
        let mut covered: Vec<std::ops::Range<usize>> = Vec::new();
        if PRIVATE_KEY.is_match(line) {
            found.insert("a private key block".to_string());
        }
        for (what, shown, re) in PREFIXED.iter() {
            for m in re.find_iter(line) {
                if overlaps(&covered, m.range()) {
                    continue;
                }
                // `?` stands for the one variable character of the prefix.
                let prefix: String = shown
                    .chars()
                    .zip(m.as_str().chars())
                    .map(|(s, c)| if s == '?' { c } else { s })
                    .collect();
                found.insert(format!(
                    "{what} (`{prefix}…`, {} characters)",
                    m.as_str().len()
                ));
                covered.push(m.range());
            }
        }
        for m in LONG_TOKEN.find_iter(line) {
            if overlaps(&covered, m.range()) {
                continue;
            }
            if let Some(what) = high_entropy(m.as_str()) {
                found.insert(what);
            }
        }
        out.extend(found.into_iter().map(|what| Suspect {
            line: line_no,
            what,
        }));
    }
    out
}

/// The warnings for one file of a shared extension: `path:line: …`.
pub fn warnings(path: &str, text: &str) -> Vec<String> {
    suspects(text)
        .into_iter()
        .map(|s| {
            format!(
                "{path}:{}: looks like {} in a shared extension, which travels with the repository — \
                 fix: keep it out of the files (a collector's `credentials:` holds one in the \
                 keychain) and rotate it if it was ever real",
                s.line, s.what
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_token_prefixes_are_found_and_masked() {
        let github = format!("ghp_{}", "a1B2c3D4e5".repeat(4));
        let samples = [
            (format!("token: {github}"), "GitHub token"),
            (
                format!("github_pat_{}", "11ABCDEFG0abcdefgh_".repeat(3)),
                "GitHub token",
            ),
            (
                format!("key = \"sk-ant-api03-{}\"", "xY9".repeat(12)),
                "Anthropic",
            ),
            (format!("k: sk-{}", "Ab3dE6gH9j".repeat(3)), "API key"),
            ("slack: xoxb-1234567890-abcdefghij".to_string(), "Slack"),
            ("aws: AKIAIOSFODNN7EXAMPLE".to_string(), "AWS"),
            (format!("glpat-{}", "q".repeat(20)), "GitLab"),
            (
                "-----BEGIN OPENSSH PRIVATE KEY-----".to_string(),
                "private key",
            ),
        ];
        for (text, what) in samples {
            let found = suspects(&text);
            assert_eq!(found.len(), 1, "{text}: {found:?}");
            assert!(found[0].what.contains(what), "{text}: {found:?}");
        }
        assert!(!warnings("x", &format!("t: {github}"))
            .join(" ")
            .contains(&github));
    }

    #[test]
    fn a_long_random_literal_is_found_but_ids_and_hashes_are_not() {
        let random = "q8Zk2Lp9XmV4rT7nB1cD6fG3hJ5sW0yA";
        assert_eq!(suspects(&format!("secret: {random}")).len(), 1);
        for fine in [
            "sha: 0123456789abcdef0123456789abcdef01234567",
            "sha256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "id: 123e4567-e89b-12d3-a456-426614174000",
            "ref: work_item:oxplow:tsk1",
            "entry: handlers/some_very_long_handler_name_for_a_command.star",
            "# prose mentioning ghp_ or sk- or AKIA as prefixes",
            "words: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ] {
            assert_eq!(suspects(fine), Vec::new(), "{fine}");
        }
    }

    #[test]
    fn a_finding_names_the_file_and_line_and_hides_the_value() {
        let text = format!("a: 1\nb: ghp_{}\n", "Zz9Yy8Xx7W".repeat(4));
        let w = warnings("oxplow/extensions/x/extension.yaml", &text);
        assert_eq!(w.len(), 1);
        assert!(w[0].starts_with("oxplow/extensions/x/extension.yaml:2: looks like a GitHub token (`ghp_…`, 44 characters)"), "{w:?}");
        assert!(!w[0].contains("Zz9Yy8Xx7W"));
    }
}
