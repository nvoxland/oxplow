//! A problem an extension check, load, activation or fixture run found:
//! one shape for all of them (`.context/extensions-sdk.md` "Diagnostics").
//!
//! Its `file` is relative to the extension's own folder (`extension.yaml`,
//! `pages/todos.yaml`), never a path of the copy that runs: whoever shows
//! it to a person names the folder they edit ([`Diagnostic::render`]),
//! and an agent reads the fields (`--json`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Whether it stops the extension (or the run) or is only worth fixing.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, specta::Type,
)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

/// Where a fixture's expectation and what happened differ.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Mismatch {
    /// A JSON pointer into what was compared (`/rows/0/state`).
    pub pointer: String,
    #[specta(type = crate::Json)]
    pub expected: Value,
    #[specta(type = crate::Json)]
    pub actual: Value,
}

/// One problem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Stable and dotted, by family: `manifest.name_mismatch`,
    /// `page.unknown_column`, `fixture.compare`.
    pub code: String,
    /// Relative to the extension's folder; `None` when no one file is at
    /// fault (a cycle of `requires:`).
    pub file: Option<String>,
    /// 1-based.
    pub line: Option<u32>,
    /// What it is about: a contribution's URI
    /// (`oxplow:oxplow/effect?effect=acme/notify`) or a command's bus id.
    pub contribution: Option<String>,
    pub message: String,
    /// What to change, when the check knows.
    pub fix: Option<String>,
    /// A fixture's comparison that didn't hold (boxed: only fixtures carry
    /// one, and every other diagnostic stays small).
    pub mismatch: Option<Box<Mismatch>>,
}

impl Diagnostic {
    pub fn error(code: &str, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, code, message)
    }

    pub fn warning(code: &str, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, code, message)
    }

    fn new(severity: Severity, code: &str, message: impl Into<String>) -> Self {
        Self {
            severity,
            code: code.to_string(),
            file: None,
            line: None,
            contribution: None,
            message: message.into(),
            fix: None,
            mismatch: None,
        }
    }

    /// In `file` (relative to the extension's folder), at `line` when known.
    pub fn at(mut self, file: impl Into<String>, line: Option<usize>) -> Self {
        self.file = Some(file.into());
        self.line = line.and_then(|n| u32::try_from(n).ok());
        self
    }

    pub fn contribution(mut self, contribution: impl Into<String>) -> Self {
        self.contribution = Some(contribution.into());
        self
    }

    pub fn fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }

    pub fn mismatch(mut self, pointer: impl Into<String>, expected: Value, actual: Value) -> Self {
        self.mismatch = Some(Box::new(Mismatch {
            pointer: pointer.into(),
            expected,
            actual,
        }));
        self
    }

    /// The line a person reads: `<root>/<file>:<line>: <message> — fix:
    /// <fix>`, where `root` is the folder they edit (the extension's source
    /// folder, repo-relative). Without a file, just the message.
    pub fn render(&self, root: &str) -> String {
        let mut out = match &self.file {
            Some(file) => {
                let path = if root.is_empty() {
                    file.clone()
                } else {
                    format!("{}/{file}", root.trim_end_matches('/'))
                };
                match self.line {
                    Some(n) => format!("{path}:{n}: {}", self.message),
                    None => format!("{path}: {}", self.message),
                }
            }
            None => self.message.clone(),
        };
        if let Some(fix) = &self.fix {
            out.push_str(" — fix: ");
            out.push_str(fix);
        }
        out
    }

    /// Whether `other` is the same problem: the same code at the same place
    /// about the same thing (two passes may word it differently).
    pub fn same_as(&self, other: &Diagnostic) -> bool {
        self.code == other.code
            && self.file == other.file
            && self.line == other.line
            && self.contribution == other.contribution
    }
}

/// `file` relative to the extension folder `root` it is under (a
/// contribution's declared-at place carries the folder it was read from);
/// as it was when it isn't under `root`.
pub fn relative_to<'a>(root: &str, file: &'a str) -> &'a str {
    let root = root.trim_end_matches('/');
    if root.is_empty() {
        return file;
    }
    file.strip_prefix(root)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(file)
}

/// A code is dotted lowercase words: a family, then the problem.
pub fn valid_code(code: &str) -> bool {
    let mut parts = code.split('.');
    let family_ok = parts
        .next()
        .is_some_and(|f| !f.is_empty() && f.chars().all(|c| c.is_ascii_lowercase()));
    let rest: Vec<_> = parts.collect();
    family_ok
        && !rest.is_empty()
        && rest.iter().all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn it_renders_under_the_folder_a_person_edits() {
        let d = Diagnostic::error(
            "manifest.name_mismatch",
            "name `a` must match its folder `b`",
        )
        .at("extension.yaml", Some(2))
        .fix("rename the folder");
        assert_eq!(
            d.render("extensions/oxplow-foundation"),
            "extensions/oxplow-foundation/extension.yaml:2: name `a` must match its folder `b` — fix: rename the folder"
        );
        assert_eq!(
            d.render(""),
            "extension.yaml:2: name `a` must match its folder `b` — fix: rename the folder"
        );
        let unplaced = Diagnostic::warning("requires.cycle", "`a` and `b` require each other");
        assert_eq!(unplaced.render("x"), "`a` and `b` require each other");
        let no_line = Diagnostic::error("page.parse", "bad").at("pages/p.yaml", None);
        assert_eq!(no_line.render("x/"), "x/pages/p.yaml: bad");
    }

    #[test]
    fn an_agent_reads_its_fields() {
        let d = Diagnostic::error("fixture.compare", "the page answered otherwise")
            .at("fixtures/board.yaml", Some(9))
            .contribution("triage/board")
            .mismatch("/rows/0/state", json!("done"), json!("todo"));
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            json!({
                "severity": "error",
                "code": "fixture.compare",
                "file": "fixtures/board.yaml",
                "line": 9,
                "contribution": "triage/board",
                "message": "the page answered otherwise",
                "fix": null,
                "mismatch": { "pointer": "/rows/0/state", "expected": "done", "actual": "todo" },
            })
        );
    }

    #[test]
    fn the_same_problem_is_known_by_code_place_and_subject() {
        let a = Diagnostic::error("dataset.compile", "one wording").at("datasets/a.sql", Some(1));
        let b = Diagnostic::error("dataset.compile", "another").at("datasets/a.sql", Some(1));
        let c = Diagnostic::error("dataset.compile", "one wording").at("datasets/b.sql", Some(1));
        assert!(a.same_as(&b));
        assert!(!a.same_as(&c));
    }

    #[test]
    fn codes_are_a_family_then_the_problem() {
        assert!(valid_code("manifest.name_mismatch"));
        assert!(valid_code("fixture.compare"));
        assert!(valid_code("import.v2_required"));
        assert!(!valid_code("manifest"));
        assert!(!valid_code("Manifest.x"));
        assert!(!valid_code("manifest.bad-dash"));
        assert!(!valid_code(".x"));
    }
}
