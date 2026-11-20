//! `file_kinds:` in `extension.yaml`: what kind of file a path is, by glob
//! (`.context/extensions-manifest.md`). Core reads them as
//! `v_file_kind_rule` (with the project's own `oxplow.files.kinds`) and
//! names no kind's files itself; a dataset or page matches a path with
//! `glob_match(glob, path)`.
//!
//! ```yaml
//! file_kinds:
//!   - { kind: ci, globs: ['.github/workflows/**', Jenkinsfile], label: CI }
//! ```
//!
//! The documented kinds are `dependency-manifest`, `lockfile`,
//! `test-harness`, `ci`, `lint-config`, `generated`, `vendored`,
//! `migration` and `sensitive`; the name is open text, so an extension may
//! declare its own, but it is lowercase words joined by dashes.

use oxplow_domain::diagnostic::Diagnostic;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::manifest_v2::{at, item_lines, key_line};

/// One `file_kinds:` entry as the extension declares it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FileKindDecl {
    /// Its id within the extension: `id:` as declared, else `<kind>`, then
    /// `<kind>-2`, … in declaration order. Extensions sharing a namespace
    /// (oxplow's shipped ones) name theirs apart (`rust-lockfile`).
    pub id: String,
    pub kind: String,
    pub globs: Vec<String>,
    pub label: Option<String>,
}

/// A `file_kinds:` entry as the manifest holds it.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileKindFile {
    /// Its id within the extension (lowercase letters, digits, dashes);
    /// default `<kind>`. Name it when another extension of your namespace
    /// declares the same kind.
    #[serde(default)]
    id: Option<String>,
    /// The kind: lowercase words joined by dashes (`ci`, `lint-config`).
    kind: String,
    /// Repo-relative globs (`*` stays in a folder, `**` crosses folders); at
    /// least one.
    globs: Vec<String>,
    /// What a person calls the kind.
    #[serde(default)]
    label: Option<String>,
}

/// Parse `file_kinds:`: the valid entries, and what's wrong with the rest
/// (at their lines).
pub(crate) fn parse_file_kinds(
    value: &Value,
    file: &str,
    manifest: &str,
) -> (Vec<FileKindDecl>, Vec<Diagnostic>) {
    let block = key_line(manifest, "file_kinds");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(
                "file_kinds.not_a_list",
                file,
                block,
                "`file_kinds` must be a list",
            )],
        );
    };
    let lines = item_lines(manifest, "file_kinds");
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let line = lines.get(i).copied().or(block);
        let f: FileKindFile = match serde_yaml::from_value(item.clone()) {
            Ok(f) => f,
            Err(e) => {
                errors.push(at(
                    "file_kinds.invalid",
                    file,
                    line,
                    format!("file_kinds: {e}"),
                ));
                continue;
            }
        };
        let problem = if !oxplow_config::core_settings::valid_file_kind(&f.kind) {
            Some((
                "file_kinds.invalid_kind",
                format!(
                    "file_kinds: kind `{}` must be lowercase words joined by dashes",
                    f.kind
                ),
            ))
        } else if f.globs.is_empty() {
            Some((
                "file_kinds.no_globs",
                format!("file_kinds: kind `{}` lists no globs", f.kind),
            ))
        } else {
            f.globs.iter().find_map(|glob| {
                globset::GlobBuilder::new(glob)
                    .literal_separator(true)
                    .build()
                    .err()
                    .map(|e| {
                        (
                            "file_kinds.invalid_glob",
                            format!("file_kinds: `{glob}` isn't a valid glob: {e}"),
                        )
                    })
            })
        };
        if let Some((code, message)) = problem {
            errors.push(at(code, file, line, message));
            continue;
        }
        let id = f.id.unwrap_or_else(|| {
            let n = out
                .iter()
                .filter(|o: &&FileKindDecl| {
                    o.id == f.kind || o.id.starts_with(&format!("{}-", f.kind))
                })
                .count();
            if n == 0 {
                f.kind.clone()
            } else {
                format!("{}-{}", f.kind, n + 1)
            }
        });
        if !oxplow_domain::contribution::valid_local(
            oxplow_domain::contribution::ContributionKind::FileKind,
            &id,
        ) || out.iter().any(|o| o.id == id)
        {
            errors.push(at(
                "file_kinds.invalid_id",
                file,
                line,
                format!("file_kinds: id `{id}` is lowercase letters, digits and dashes, once per extension"),
            ));
            continue;
        }
        out.push(FileKindDecl {
            id,
            kind: f.kind,
            globs: f.globs,
            label: f.label.filter(|l| !l.trim().is_empty()),
        });
    }
    (out, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> (Vec<FileKindDecl>, Vec<Diagnostic>) {
        let v: serde_yaml::Value = serde_yaml::from_str(text).unwrap();
        parse_file_kinds(&v["file_kinds"], "extension.yaml", text)
    }

    /// Valid entries load (an unknown kind name is fine); a kind that isn't
    /// lowercase-dashed, no globs, a glob that doesn't parse and an
    /// unknown key are errors at their own lines.
    #[test]
    fn file_kinds_parse_and_report_their_problems() {
        let text = "file_kinds:\n  - { kind: ci, globs: ['.github/workflows/**'], label: CI }\n  - { kind: Bad_Kind, globs: [x] }\n  - { kind: lockfile, globs: [] }\n  - { kind: house-rules, globs: ['a{b'] }\n  - { kind: ci, globs: [x], extra: 1 }\n  - { kind: house-rules, globs: ['docs/**'] }\n";
        let (ok, errors) = parse(text);
        assert_eq!(
            ok,
            vec![
                FileKindDecl {
                    id: "ci".into(),
                    kind: "ci".into(),
                    globs: vec![".github/workflows/**".into()],
                    label: Some("CI".into())
                },
                FileKindDecl {
                    id: "house-rules".into(),
                    kind: "house-rules".into(),
                    globs: vec!["docs/**".into()],
                    label: None
                },
            ]
        );
        let got: Vec<(&str, Option<u32>)> =
            errors.iter().map(|e| (e.code.as_str(), e.line)).collect();
        assert_eq!(
            got,
            vec![
                ("file_kinds.invalid_kind", Some(3)),
                ("file_kinds.no_globs", Some(4)),
                ("file_kinds.invalid_glob", Some(5)),
                ("file_kinds.invalid", Some(6)),
            ],
            "{errors:?}"
        );
    }
}
