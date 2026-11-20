//! `test_conventions:` in `extension.yaml`: what makes a file, a container
//! or a function a test, and what counts as an assertion or a skip
//! (`.context/language-support.md` "Test conventions"). Core keeps the
//! engine (`crate::test_conventions`: comment stripping, counting, the
//! project's `oxplow.testing.testPaths`) and names no framework or
//! language's rules itself; oxplow-foundation declares the ones that hold
//! anywhere and each language pack the ones of its language.
//!
//! ```yaml
//! test_conventions:
//!   - id: go-tests
//!     languages: [go]
//!     paths: ['**/*_test.go']
//!     functions: ['Test*', 'Benchmark*']
//!     assertions: ['\bt\.(?:Error|Errorf|Fatal|Fatalf)\b']
//!     skips: ['\bt\.(?:Skip|Skipf|SkipNow)\b']
//! ```
//!
//! - `languages`: language ids the convention applies to (a file's
//!   language is the one `languages:` gives its path); absent, any file.
//!   An id no enabled or shipped extension declares is a warning, the
//!   pack may be off.
//! - `paths`: repo-relative globs (`*` stays in a folder, `**` crosses
//!   folders), as `oxplow.testing.testPaths`.
//! - `containers`, `functions`: name patterns, matched against one whole
//!   name: `*` any run of characters, `?` one, `[A-Z]` a character class
//!   (`Test*`, `*Test`, `test_*`, `Test[A-Z]*`, `*-test`).
//! - `assertions`, `skips`: regular expressions, each match on a line of
//!   code (line comments are left out) counting one.

use oxplow_domain::diagnostic::Diagnostic;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::manifest_v2::{at, item_lines, key_line};

/// One `test_conventions:` entry as the extension declares it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct TestConventionDecl {
    pub id: String,
    pub languages: Vec<String>,
    pub paths: Vec<String>,
    pub containers: Vec<String>,
    pub functions: Vec<String>,
    pub assertions: Vec<String>,
    pub skips: Vec<String>,
}

/// A `test_conventions:` entry as the manifest holds it.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TestConventionFile {
    /// Lowercase letters, digits and dashes; unique in the extension.
    id: String,
    /// Language ids it applies to; absent, any file.
    #[serde(default)]
    languages: Vec<String>,
    /// Repo-relative globs of test files.
    #[serde(default)]
    paths: Vec<String>,
    /// Name patterns (`Test*`, `*Test`) of test containers: classes, modules.
    #[serde(default)]
    containers: Vec<String>,
    /// Name patterns (`test_*`) of test functions.
    #[serde(default)]
    functions: Vec<String>,
    /// Regular expressions; each match on a code line is an assertion.
    #[serde(default)]
    assertions: Vec<String>,
    /// Regular expressions; each match on a code line is a skip.
    #[serde(default)]
    skips: Vec<String>,
}

/// The ids of the languages the extensions that ship with oxplow declare,
/// enabled or not.
pub fn shipped_language_ids() -> &'static std::collections::BTreeSet<String> {
    static IDS: std::sync::OnceLock<std::collections::BTreeSet<String>> =
        std::sync::OnceLock::new();
    IDS.get_or_init(|| {
        crate::bundled_extensions::BUNDLED
            .iter()
            .filter_map(|b| b.file("extension.yaml"))
            .filter_map(|text| serde_yaml::from_str::<Value>(text).ok())
            .filter_map(|v| v.get("languages").and_then(Value::as_sequence).cloned())
            .flatten()
            .filter_map(|l| l.get("id").and_then(Value::as_str).map(str::to_string))
            .collect()
    })
}

/// A name pattern, compiled (`*`, `?`, `[A-Z]` over a whole name).
pub fn name_glob(pattern: &str) -> Result<globset::GlobMatcher, globset::Error> {
    globset::GlobBuilder::new(pattern)
        .literal_separator(false)
        .build()
        .map(|g| g.compile_matcher())
}

/// A path glob, compiled (`testing.testPaths`' dialect).
pub fn path_glob(pattern: &str) -> Result<globset::Glob, globset::Error> {
    globset::GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
}

/// Parse `test_conventions:`: the valid entries, and what's wrong with the
/// rest (at their lines).
pub(crate) fn parse_test_conventions(
    value: &Value,
    file: &str,
    manifest: &str,
) -> (Vec<TestConventionDecl>, Vec<Diagnostic>) {
    let block = key_line(manifest, "test_conventions");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(
                "test_conventions.not_a_list",
                file,
                block,
                "`test_conventions` must be a list",
            )],
        );
    };
    let lines = item_lines(manifest, "test_conventions");
    let mut out: Vec<TestConventionDecl> = Vec::new();
    let mut errors = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let line = lines.get(i).copied().or(block);
        let f: TestConventionFile = match serde_yaml::from_value(item.clone()) {
            Ok(f) => f,
            Err(e) => {
                errors.push(at(
                    "test_conventions.invalid",
                    file,
                    line,
                    format!("test_conventions: {e}"),
                ));
                continue;
            }
        };
        let problem = if !oxplow_config::core_settings::valid_file_kind(&f.id) {
            Some((
                "test_conventions.invalid_id",
                format!(
                    "test convention id `{}` must be lowercase letters, digits and dashes",
                    f.id
                ),
            ))
        } else if out.iter().any(|o| o.id == f.id) {
            Some((
                "test_conventions.duplicate",
                format!("test convention `{}` is declared twice", f.id),
            ))
        } else {
            first_problem(&f)
        };
        match problem {
            Some((code, message)) => errors.push(at(code, file, line, message)),
            None => out.push(TestConventionDecl {
                id: f.id,
                languages: f.languages,
                paths: f.paths,
                containers: f.containers,
                functions: f.functions,
                assertions: f.assertions,
                skips: f.skips,
            }),
        }
    }
    (out, errors)
}

/// The first thing wrong with an entry's patterns: one that is empty, a
/// glob or name pattern that doesn't parse, a regex that doesn't compile.
fn first_problem(f: &TestConventionFile) -> Option<(&'static str, String)> {
    let ctx = |m: String| format!("test convention `{}`: {m}", f.id);
    for (what, list) in [
        ("languages", &f.languages),
        ("paths", &f.paths),
        ("containers", &f.containers),
        ("functions", &f.functions),
        ("assertions", &f.assertions),
        ("skips", &f.skips),
    ] {
        if list.iter().any(|p| p.trim().is_empty()) {
            return Some((
                "test_conventions.empty_pattern",
                ctx(format!("`{what}` has an empty entry")),
            ));
        }
    }
    for g in &f.paths {
        if let Err(e) = path_glob(g) {
            return Some((
                "test_conventions.invalid_glob",
                ctx(format!("path `{g}` isn't a valid glob: {e}")),
            ));
        }
    }
    for g in f.containers.iter().chain(&f.functions) {
        if let Err(e) = name_glob(g) {
            return Some((
                "test_conventions.invalid_glob",
                ctx(format!("name pattern `{g}` isn't valid: {e}")),
            ));
        }
    }
    for r in f.assertions.iter().chain(&f.skips) {
        if let Err(e) = regex::Regex::new(r) {
            return Some((
                "test_conventions.invalid_regex",
                ctx(format!("`{r}` isn't a regular expression: {e}")),
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> (Vec<TestConventionDecl>, Vec<Diagnostic>) {
        let v: serde_yaml::Value = serde_yaml::from_str(text).unwrap();
        parse_test_conventions(&v["test_conventions"], "extension.yaml", text)
    }

    /// Valid entries load; a bad id, a repeat, an empty entry, a glob, name
    /// pattern or regex that doesn't parse and an unknown key are errors at
    /// their own lines.
    #[test]
    fn test_conventions_parse_and_report_their_problems() {
        let text = "test_conventions:\n  - { id: go, languages: [go], paths: ['**/*_test.go'], functions: ['Test*'], skips: ['\\bt\\.Skip\\b'] }\n  - { id: Bad, paths: [x] }\n  - { id: go, paths: [x] }\n  - { id: empty, paths: [' '] }\n  - { id: glob, paths: ['a{b'] }\n  - { id: name, functions: ['Test[A-'] }\n  - { id: re, assertions: ['('] }\n  - { id: extra, paths: [x], nope: 1 }\n  - { id: ok, containers: ['*Test'] }\n";
        let (ok, errors) = parse(text);
        assert_eq!(
            ok.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["go", "ok"]
        );
        let got: Vec<(&str, Option<u32>)> =
            errors.iter().map(|e| (e.code.as_str(), e.line)).collect();
        assert_eq!(
            got,
            vec![
                ("test_conventions.invalid_id", Some(3)),
                ("test_conventions.duplicate", Some(4)),
                ("test_conventions.empty_pattern", Some(5)),
                ("test_conventions.invalid_glob", Some(6)),
                ("test_conventions.invalid_glob", Some(7)),
                ("test_conventions.invalid_regex", Some(8)),
                ("test_conventions.invalid", Some(9)),
            ],
            "{errors:?}"
        );
    }
}
