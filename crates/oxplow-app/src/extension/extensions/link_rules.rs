//! `links:` in `extension.yaml`: text that is a link (`.context/extensions-manifest.md`
//! "links"). An entry names a pattern and the URI a match links to; the
//! terminal and markdown turn each whole-word match into that link, which
//! gets the same right-click menu as any (Open In ▸ and the commands about
//! it).
//!
//! ```yaml
//! links:
//!   - { id: linear-issue, pattern: "[A-Z]{2,10}-[0-9]+", uri: "https://linear.app/acme/issue/{{match}}" }
//! ```
//!
//! The pattern is the param-pattern dialect without its anchors
//! (`crate::param_pattern`: Rust and the desktop's JS read it alike). The
//! desktop scans with the same rule as [`find_rule_links`] (`linkRules.ts`);
//! `link_rules_cases.json` holds the cases both run.

use oxplow_domain::diagnostic::Diagnostic;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::manifest_v2::{at, item_lines, key_line};

/// The one placeholder a `links:` URI takes: the matched text, encoded.
pub const MATCH: &str = "{{match}}";

/// One `links:` entry, checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct LinkRuleDecl {
    /// Its id within the extension.
    pub id: String,
    /// The pattern as both engines read it: the param pattern's kept form,
    /// without `^…$` (`\d` spelled `[0-9]`).
    pub pattern: String,
    /// The URI a match links to, `{{match}}` where the text goes.
    pub uri: String,
}

/// A `links:` entry as the manifest holds it.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LinkRuleFile {
    /// Its id within the extension (lowercase letters, digits, dashes).
    id: String,
    /// What text is a link: a param pattern without `^…$` (characters,
    /// classes, `\d`, `\w`, escaped punctuation, each with one quantifier;
    /// no `.`, groups or `|`), matched as a whole word.
    pattern: String,
    /// The URI a match links to: `https://…` or an oxplow URI, with
    /// `{{match}}` where the matched text goes (percent-encoded).
    uri: String,
}

/// Parse `links:`: the valid entries, and what's wrong with the rest (at
/// their lines).
pub(crate) fn parse_links(
    value: &Value,
    file: &str,
    manifest: &str,
) -> (Vec<LinkRuleDecl>, Vec<Diagnostic>) {
    let block = key_line(manifest, "links");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(
                "links.not_a_list",
                file,
                block,
                "`links` must be a list",
            )],
        );
    };
    let lines = item_lines(manifest, "links");
    let mut out: Vec<LinkRuleDecl> = Vec::new();
    let mut errors = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let line = lines.get(i).copied().or(block);
        let f: LinkRuleFile = match serde_yaml::from_value(item.clone()) {
            Ok(f) => f,
            Err(e) => {
                errors.push(at("links.invalid", file, line, format!("links: {e}")));
                continue;
            }
        };
        match check(&f, &out) {
            Ok(rule) => out.push(rule),
            Err((code, message)) => errors.push(at(code, file, line, message)),
        }
    }
    (out, errors)
}

fn check(
    f: &LinkRuleFile,
    earlier: &[LinkRuleDecl],
) -> Result<LinkRuleDecl, (&'static str, String)> {
    let id_ok = oxplow_domain::contribution::valid_local(
        oxplow_domain::contribution::ContributionKind::Link,
        &f.id,
    );
    if !id_ok || earlier.iter().any(|r| r.id == f.id) {
        return Err((
            "links.invalid_id",
            format!(
                "links: `{}` must be a new id of lowercase letters, digits and dashes",
                f.id
            ),
        ));
    }
    if f.pattern.starts_with('^') || f.pattern.ends_with('$') {
        return Err((
            "links.invalid_pattern",
            format!(
                "links `{}`: the pattern is matched as a whole word in text; leave out `^` and `$`",
                f.id
            ),
        ));
    }
    let kept = crate::param_pattern::param_pattern(&format!("^{}$", f.pattern)).map_err(|why| {
        (
            "links.invalid_pattern",
            format!(
                "links `{}`: `{}` isn't a link pattern: {why}",
                f.id, f.pattern
            ),
        )
    })?;
    let pattern = kept[1..kept.len() - 1].to_string();
    let others = f.uri.replace(MATCH, "");
    if !f.uri.contains(MATCH) || others.contains("{{") {
        return Err((
            "links.invalid_uri",
            format!("links `{}`: the uri takes `{MATCH}` where the matched text goes, and no other placeholder", f.id),
        ));
    }
    let sample = f.uri.replace(MATCH, "x");
    let is_uri = sample.starts_with("https://")
        || sample.starts_with("http://")
        || oxplow_domain::uri::OxplowUri::parse(&sample).is_ok();
    if !is_uri {
        return Err((
            "links.invalid_uri",
            format!(
                "links `{}`: `{}` isn't an `https://` URL or an oxplow URI",
                f.id, f.uri
            ),
        ));
    }
    Ok(LinkRuleDecl {
        id: f.id.clone(),
        pattern,
        uri: f.uri.clone(),
    })
}

/// A run of text a rule made a link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleLink {
    /// Byte offsets in the text (the desktop's are UTF-16: the same for
    /// ASCII, which every pattern matches).
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub uri: String,
}

/// Percent-encode `text` for a URI: every byte but `A-Z a-z 0-9 - . _ ~`.
pub fn encode_match(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn word(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The links `rules` make in `text`: each rule's matches that are whole
/// words (no letter, digit or `_` either side), leftmost first; where two
/// rules' matches overlap, the earlier rule's.
pub fn find_rule_links(text: &str, rules: &[LinkRuleDecl]) -> Vec<RuleLink> {
    let mut found: Vec<(usize, RuleLink)> = Vec::new();
    for (order, rule) in rules.iter().enumerate() {
        let Ok(re) = regex::Regex::new(&rule.pattern) else {
            continue;
        };
        for m in re.find_iter(text) {
            let before = text[..m.start()].chars().next_back();
            let after = text[m.end()..].chars().next();
            if word(before) || word(after) || m.as_str().is_empty() {
                continue;
            }
            found.push((
                order,
                RuleLink {
                    start: m.start(),
                    end: m.end(),
                    text: m.as_str().to_string(),
                    uri: rule.uri.replace(MATCH, &encode_match(m.as_str())),
                },
            ));
        }
    }
    found.sort_by_key(|(order, l)| (*order, l.start));
    let mut kept: Vec<RuleLink> = Vec::new();
    for (_, link) in found {
        if kept
            .iter()
            .all(|k| link.end <= k.start || link.start >= k.end)
        {
            kept.push(link);
        }
    }
    kept.sort_by_key(|l| l.start);
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> (Vec<LinkRuleDecl>, Vec<String>) {
        let manifest = format!("links:\n{yaml}");
        let doc: Value = serde_yaml::from_str(&manifest).unwrap();
        let (rules, errors) = parse_links(&doc["links"], "extension.yaml", &manifest);
        (rules, errors.into_iter().map(|d| d.message).collect())
    }

    #[test]
    fn a_rule_is_a_pattern_and_a_uri_with_the_match() {
        let (rules, errors) = parse(
            "  - { id: issue, pattern: '[A-Z]{2,10}-\\d+', uri: 'https://linear.app/acme/issue/{{match}}' }\n",
        );
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(rules[0].pattern, "[A-Z]{2,10}\\-[0-9]+");
    }

    #[test]
    fn what_cant_link_is_refused_at_its_line() {
        for (yaml, says) in [
            (
                "  - { id: a, pattern: '^X$', uri: 'https://x/{{match}}' }\n",
                "leave out",
            ),
            (
                "  - { id: a, pattern: 'a.b', uri: 'https://x/{{match}}' }\n",
                "no `.`",
            ),
            (
                "  - { id: a, pattern: 'X', uri: 'https://x/' }\n",
                "{{match}}",
            ),
            (
                "  - { id: a, pattern: 'X', uri: 'ftp://x/{{match}}' }\n",
                "isn't an",
            ),
            (
                "  - { id: A, pattern: 'X', uri: 'https://x/{{match}}' }\n",
                "lowercase",
            ),
        ] {
            let (rules, errors) = parse(yaml);
            assert!(rules.is_empty(), "{yaml}");
            assert!(
                errors.iter().any(|e| e.contains(says)),
                "{yaml}: {errors:?}"
            );
        }
    }

    /// The cases the desktop's matcher runs too (`linkRules.test.ts`).
    #[test]
    fn the_shared_cases_match() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("link_rules_cases.json")).unwrap();
        let rules: Vec<LinkRuleDecl> = serde_json::from_value(cases["rules"].clone()).unwrap();
        for case in cases["cases"].as_array().unwrap() {
            let text = case["text"].as_str().unwrap();
            let got: Vec<serde_json::Value> = find_rule_links(text, &rules)
                .into_iter()
                .map(|l| serde_json::json!({ "text": l.text, "uri": l.uri }))
                .collect();
            assert_eq!(serde_json::Value::Array(got), case["links"], "{text}");
        }
    }
}
