//! `ui.decorators` (experimental — a private extension only; P6b.C5):
//! labels from one of the extension's models shown on core refs — a chip
//! on a page whose ref the model lists (`ref-chip`), a badge after a lens
//! cell that links to one (`row-badge`). Additive: a page is complete
//! without them.
//!
//! ```yaml
//! ui:
//!   decorators:
//!     - { model: flags, kind: work_item, placement: ref-chip, label: label, color: color }
//! ```
//!
//! The model is `v_<extension>_<model>`; it must declare a `ref` column
//! and the `label` (and `color`) columns named.

use serde::{Deserialize, Serialize};

use super::manifest_v2::{at, key_line, line_under};

/// Where a decoration shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum DecoratorPlacement {
    /// A chip in the header of a page whose ref the model lists.
    RefChip,
    /// A badge after a lens cell that links to a listed ref.
    RowBadge,
}

/// A decorator (valid ones; invalid ones are in the extension's `errors`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UiDecorator {
    /// `<extension>/<n>`.
    pub id: String,
    pub extension: String,
    /// The model's view (`v_<extension>_<model>`), which has a `ref` column.
    pub view: String,
    /// The ref kind it decorates (`work_item`).
    pub kind: String,
    pub placement: DecoratorPlacement,
    /// The view's column holding the text shown.
    pub label: String,
    /// The view's column holding a color (`#rrggbb` or a CSS color name).
    pub color: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DecoratorFile {
    model: String,
    kind: String,
    placement: DecoratorPlacement,
    label: String,
    #[serde(default)]
    color: Option<String>,
}

/// A column name that is safe to name in SQL as is.
fn identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Parse `ui.decorators` against the extension's models: the valid ones,
/// and an error (`file:line: …`) for each broken one.
pub fn parse_decorators(
    extension: &str,
    models: &[oxplow_db::models::ModelSource],
    value: &serde_yaml::Value,
    file: &str,
    manifest: &str,
) -> (Vec<UiDecorator>, Vec<String>) {
    let block = line_under(manifest, "ui", "decorators:").or(key_line(manifest, "ui"));
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(file, block, "`ui.decorators` must be a list")],
        );
    };
    let kinds = oxplow_domain::refs::kind::core_kinds();
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let d: DecoratorFile = match serde_yaml::from_value(item.clone()) {
            Ok(d) => d,
            Err(e) => {
                errors.push(at(file, block, format!("`ui.decorators`: {e}")));
                continue;
            }
        };
        let line = line_under(manifest, "ui", &format!("model: {}", d.model)).or(block);
        let model = models.iter().find(|m| m.decl.name == d.model);
        let has = |col: &str| model.is_some_and(|m| m.decl.columns.iter().any(|c| c.name == col));
        let wanted: Vec<&str> = ["ref", d.label.as_str()]
            .into_iter()
            .chain(d.color.as_deref())
            .collect();
        let problem = if kinds.get(&d.kind).is_none() {
            Some(format!(
                "`kind: {}` isn't a kind of ref (`work_item`, `commit`, …)",
                d.kind
            ))
        } else if model.is_none() {
            Some(format!(
                "`model: {}` isn't one of this extension's models (`models:`)",
                d.model
            ))
        } else if let Some(bad) = wanted.iter().find(|c| !identifier(c)) {
            Some(format!("`{bad}` isn't a column name"))
        } else if let Some(missing) = wanted.iter().find(|c| !has(c)) {
            Some(format!(
                "model `{}` has no `{missing}` column (a decorator's model needs `ref` and the \
                 columns it names)",
                d.model
            ))
        } else {
            None
        };
        if let Some(p) = problem {
            errors.push(at(file, line, format!("`ui.decorators`: {p}")));
            continue;
        }
        out.push(UiDecorator {
            id: format!("{extension}/{i}"),
            extension: extension.to_string(),
            view: oxplow_db::models::extension_view(extension, &d.model),
            kind: d.kind,
            placement: d.placement,
            label: d.label,
            color: d.color,
        });
    }
    (out, errors)
}

#[cfg(test)]
mod tests {
    use crate::extensions::{load_extensions, Extension};
    use std::path::Path;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// Extension `flags` (`sharing`) with a `flags` model and `decorators`.
    fn load(sharing: &str, decorators: &str) -> Extension {
        let d = tempfile::tempdir().unwrap();
        write(
            d.path(),
            "oxplow/extensions/flags/extension.yaml",
            &format!(
                "manifest: 2\nname: flags\nsharing: {sharing}\nengine: \">=0.1\"\nintent:\n  purpose: p\n  examples: [{{ name: a }}]\nmodels:\n  - name: flags\n    version: 1\n    description: d\n    columns:\n      - {{ name: ref, type: TEXT, doc: r }}\n      - {{ name: label, type: TEXT, doc: l }}\n      - {{ name: color, type: TEXT, doc: c }}\nui:\n  decorators:\n{decorators}"
            ),
        );
        write(
            d.path(),
            "oxplow/extensions/flags/models/flags.sql",
            "SELECT 'work_item:oxplow:tsk1' AS ref, 'urgent' AS label, '#f00' AS color\n",
        );
        load_extensions(d.path())
            .into_iter()
            .find(|e| e.name == "flags")
            .unwrap()
    }

    #[test]
    fn a_decorator_names_one_of_its_models_columns() {
        let ext = load(
            "private",
            "    - { model: flags, kind: work_item, placement: ref-chip, label: label, color: color }\n",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let d = &ext.ui.decorators[0];
        assert_eq!(d.view, "v_flags_flags");
        assert_eq!(d.placement, super::DecoratorPlacement::RefChip);
        assert_eq!(d.color.as_deref(), Some("color"));
        for (entry, says) in [
            (
                "{ model: nope, kind: work_item, placement: ref-chip, label: label }",
                "isn't one of this extension's models",
            ),
            (
                "{ model: flags, kind: nope, placement: ref-chip, label: label }",
                "isn't a kind of ref",
            ),
            (
                "{ model: flags, kind: work_item, placement: ref-chip, label: title }",
                "has no `title` column",
            ),
            (
                "{ model: flags, kind: work_item, placement: ref-chip, label: \"x; DROP\" }",
                "isn't a column name",
            ),
            (
                "{ model: flags, kind: work_item, placement: sidebar, label: label }",
                "unknown variant",
            ),
        ] {
            let ext = load("private", &format!("    - {entry}\n"));
            let errs = ext.errors.join("\n");
            assert!(
                errs.contains(says) && errs.contains("extension.yaml:"),
                "{entry}: {errs}"
            );
            assert!(ext.ui.decorators.is_empty(), "{entry}");
        }
    }

    #[test]
    fn a_shared_extension_may_not_declare_decorators() {
        let ext = load(
            "shared",
            "    - { model: flags, kind: work_item, placement: ref-chip, label: label }\n",
        );
        assert!(
            ext.errors
                .join("\n")
                .contains("`ui.decorators` is experimental"),
            "{:?}",
            ext.errors
        );
        assert!(ext.ui.decorators.is_empty());
    }
}
