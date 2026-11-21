//! `languages:` in `extension.yaml`: a programming language, with its files
//! (`.context/language-support.md`). oxplow's own are its language packs
//! (`extensions/oxplow-rust` and the rest); any extension may declare one.
//!
//! ```yaml
//! languages:
//!   - id: rust
//!     title: Rust
//!     extensions: [rs]
//!     aliases: [rs]
//!     grammar: { native: rust }          # shipped extensions; or { wasm: grammars/x.wasm }
//!     analysis: languages/rust/analysis.yaml
//!     symbols: languages/rust/symbols.scm  # + symbols.yaml beside it
//!     lsp: { package: rust-analyzer }
//!     units: [class, module]
//! ```
//!
//! Each is read and built at load (its grammar, its analysis file checked
//! against the grammar's node kinds, its symbol query compiled), so a wrong
//! one is this extension's error at its line. Two enabled extensions
//! claiming one name or file extension is settled when the set is built
//! (`crate::extension::language_set`).

use oxplow_code_metrics::{Language, LanguageDecl, LanguageSource};
use oxplow_domain::diagnostic::Diagnostic;
use serde_yaml::Value;

use super::manifest_v2::{at, item_lines, key_line};
use super::ExtensionFiles;

/// Parse `languages:`: each valid language with its files read, and what's
/// wrong with the rest (at their lines).
pub(crate) fn parse_languages(
    value: &Value,
    file: &str,
    manifest: &str,
    files: &dyn ExtensionFiles,
    extension: &str,
    shipped: bool,
) -> (Vec<LanguageSource>, Vec<Diagnostic>) {
    let block = key_line(manifest, "languages");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(
                "language.not_a_list",
                file,
                block,
                "`languages` must be a list",
            )],
        );
    };
    let lines = item_lines(manifest, "languages");
    let mut out: Vec<LanguageSource> = Vec::new();
    let mut errors = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let line = lines.get(i).copied().or(block);
        let decl: LanguageDecl = match serde_yaml::from_value(item.clone()) {
            Ok(d) => d,
            Err(e) => {
                errors.push(at(
                    "language.invalid",
                    file,
                    line,
                    format!("languages: {e}"),
                ));
                continue;
            }
        };
        let id_ok = !decl.id.is_empty()
            && decl
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !id_ok {
            errors.push(at(
                "language.invalid_id",
                file,
                line,
                format!(
                    "language id `{}` must be lowercase letters, digits and `_` (it is stored, as `symbol.language`)",
                    decl.id
                ),
            ));
            continue;
        }
        if out.iter().any(|o| o.decl.id == decl.id) {
            errors.push(at(
                "language.duplicate",
                file,
                line,
                format!("language `{}` is declared twice", decl.id),
            ));
            continue;
        }
        let source =
            match LanguageSource::read(extension, shipped, decl, &|rel| files.bytes(rel).ok()) {
                Ok(s) => s,
                Err(e) => {
                    errors.push(at("language.missing_file", file, line, e));
                    continue;
                }
            };
        let built = Language::build(&source).and_then(|l| l.check_symbols().map(|()| l));
        if let Err(e) = built {
            errors.push(at("language.invalid", file, line, e));
            continue;
        }
        out.push(source);
    }
    (out, errors)
}
