//! `run_patterns:` in `extension.yaml`: the command substrings that mark a
//! shell call as a test run or a static-analysis run
//! (`.context/collection.md` "Ingestion"). Core matches the enabled
//! extensions' patterns, plus the project's own `testing:` settings; it
//! knows no tool by name.
//!
//! ```yaml
//! run_patterns:
//!   - { kind: test, patterns: [pytest, cargo test] }
//!   - { kind: analysis, patterns: [ruff, mypy] }
//! ```
//!
//! A pattern is matched as a case-insensitive substring of one
//! sub-command; a sub-command that only reads (`grep`, `echo`, …) and a
//! `--no-run` build are never a run.

use oxplow_domain::diagnostic::Diagnostic;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::manifest_v2::{at, item_lines, key_line};

/// What a pattern marks a run as: a report collector's `RunKind` too.
pub use oxplow_config::collectors::RunKind;

/// One `run_patterns:` entry as the extension declares it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RunPatternDecl {
    /// Its id within the extension: `id:` as declared, else `<kind>`, then
    /// `<kind>-2`, … in declaration order. Extensions sharing a namespace
    /// (oxplow's shipped ones) name theirs apart (`rust-test`).
    pub id: String,
    pub kind: RunKind,
    pub patterns: Vec<String>,
}

/// A `run_patterns:` entry as the manifest holds it.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunPatternFile {
    /// Its id within the extension (lowercase letters, digits, dashes);
    /// default `<kind>`. Name it when another extension of your namespace
    /// declares the same kind.
    #[serde(default)]
    id: Option<String>,
    /// `test` or `analysis`.
    kind: String,
    /// Command substrings, matched case-insensitively; none empty.
    patterns: Vec<String>,
}

/// Parse `run_patterns:`: the valid entries, and what's wrong with the rest
/// (at their lines).
pub(crate) fn parse_run_patterns(
    value: &Value,
    file: &str,
    manifest: &str,
) -> (Vec<RunPatternDecl>, Vec<Diagnostic>) {
    let block = key_line(manifest, "run_patterns");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(
                "run_patterns.not_a_list",
                file,
                block,
                "`run_patterns` must be a list",
            )],
        );
    };
    let lines = item_lines(manifest, "run_patterns");
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let line = lines.get(i).copied().or(block);
        let f: RunPatternFile = match serde_yaml::from_value(item.clone()) {
            Ok(f) => f,
            Err(e) => {
                errors.push(at(
                    "run_patterns.invalid",
                    file,
                    line,
                    format!("run_patterns: {e}"),
                ));
                continue;
            }
        };
        let kind = match f.kind.as_str() {
            "test" => RunKind::Test,
            "analysis" => RunKind::Analysis,
            other => {
                errors.push(at(
                    "run_patterns.invalid_kind",
                    file,
                    line,
                    format!("run_patterns: `kind` is `test` or `analysis`, not `{other}`"),
                ));
                continue;
            }
        };
        if f.patterns.is_empty() || f.patterns.iter().any(|p| p.trim().is_empty()) {
            errors.push(at(
                "run_patterns.empty_pattern",
                file,
                line,
                "run_patterns: `patterns` lists at least one pattern, and none is empty",
            ));
            continue;
        }
        let id = match f.id {
            Some(id) => id,
            None => {
                let kind = match kind {
                    RunKind::Test => "test",
                    RunKind::Analysis => "analysis",
                };
                let n = out
                    .iter()
                    .filter(|o: &&RunPatternDecl| {
                        o.id == kind || o.id.starts_with(&format!("{kind}-"))
                    })
                    .count();
                if n == 0 {
                    kind.to_string()
                } else {
                    format!("{kind}-{}", n + 1)
                }
            }
        };
        if !oxplow_domain::contribution::valid_local(
            oxplow_domain::contribution::ContributionKind::RunPattern,
            &id,
        ) || out.iter().any(|o| o.id == id)
        {
            errors.push(at(
                "run_patterns.invalid_id",
                file,
                line,
                format!("run_patterns: id `{id}` is lowercase letters, digits and dashes, once per extension"),
            ));
            continue;
        }
        out.push(RunPatternDecl {
            id,
            kind,
            patterns: f.patterns,
        });
    }
    (out, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> (Vec<RunPatternDecl>, Vec<Diagnostic>) {
        let v: serde_yaml::Value = serde_yaml::from_str(text).unwrap();
        parse_run_patterns(&v["run_patterns"], "extension.yaml", text)
    }

    /// Valid entries load; an unknown kind, an empty pattern, an empty list
    /// and an unknown key are errors at their own lines.
    #[test]
    fn run_patterns_parse_and_report_their_problems() {
        let text = "run_patterns:\n  - { kind: test, patterns: [pytest, cargo test] }\n  - { kind: build, patterns: [make] }\n  - { kind: analysis, patterns: ['  '] }\n  - { kind: test, patterns: [] }\n  - { kind: test, patterns: [x], extra: 1 }\n  - { kind: analysis, patterns: [ruff] }\n";
        let (ok, errors) = parse(text);
        assert_eq!(
            ok,
            vec![
                RunPatternDecl {
                    id: "test".into(),
                    kind: RunKind::Test,
                    patterns: vec!["pytest".into(), "cargo test".into()]
                },
                RunPatternDecl {
                    id: "analysis".into(),
                    kind: RunKind::Analysis,
                    patterns: vec!["ruff".into()]
                },
            ]
        );
        let got: Vec<(&str, Option<u32>)> =
            errors.iter().map(|e| (e.code.as_str(), e.line)).collect();
        assert_eq!(
            got,
            vec![
                ("run_patterns.invalid_kind", Some(3)),
                ("run_patterns.empty_pattern", Some(4)),
                ("run_patterns.empty_pattern", Some(5)),
                ("run_patterns.invalid", Some(6)),
            ],
            "{errors:?}"
        );
    }
}
