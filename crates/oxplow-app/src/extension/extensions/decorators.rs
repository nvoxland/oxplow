//! `ui.decorators` (P6b.C5; stable since P10, on oxplow-bundled's verdict):
//! labels from one of the extension's datasets shown on core refs — a chip
//! on a page whose ref the dataset lists (`ref-chip`), a badge after a page
//! cell that links to one (`row-badge`). Additive: a page is complete
//! without them.
//!
//! ```yaml
//! ui:
//!   decorators:
//!     - { dataset: flags, resource: oxplow/work_item, placement: ref-chip, label: label, color: color }
//! ```
//!
//! The dataset is `v_<namespace>_<dataset>`; it must declare a `ref` column
//! and the `label` (and `color`) columns named.

use oxplow_domain::diagnostic::Diagnostic;
use serde::{Deserialize, Serialize};

use super::manifest_v2::{at, item_lines, key_line, line_under};

/// Where a decoration shows.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum DecoratorPlacement {
    /// A chip in the header of a page whose ref the dataset lists.
    RefChip,
    /// A badge after a page cell that links to a listed ref.
    RowBadge,
}

/// A decorator (valid ones; invalid ones are in the extension's `errors`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UiDecorator {
    /// `<extension>/<n>`.
    pub id: String,
    pub extension: String,
    /// The dataset's view (`v_<namespace>_<dataset>`), which has a `ref` column.
    pub view: String,
    /// The resource it decorates (`oxplow/work_item`).
    pub resource: String,
    pub placement: DecoratorPlacement,
    /// The view's column holding the text shown.
    pub label: String,
    /// The view's column holding a color (`#rrggbb` or a CSS color name).
    pub color: Option<String>,
}

/// A `ui.decorators` entry as the manifest holds it.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DecoratorFile {
    dataset: String,
    resource: String,
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

/// Parse `ui.decorators` against the extension's datasets: the valid ones,
/// and an error (`file:line: …`) for each broken one.
pub fn parse_decorators(
    extension: &str,
    namespace: &str,
    datasets: &[oxplow_db::datasets::DatasetSource],
    value: &serde_yaml::Value,
    file: &str,
    manifest: &str,
) -> (Vec<UiDecorator>, Vec<Diagnostic>) {
    let block = line_under(manifest, "ui", "decorators:").or(key_line(manifest, "ui"));
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(
                "decorator.not_a_list",
                file,
                block,
                "`ui.decorators` must be a list",
            )],
        );
    };
    let mut out = Vec::new();
    let mut errors = Vec::new();
    let lines = item_lines(manifest, "ui.decorators");
    for (i, item) in items.iter().enumerate() {
        let item_line = lines.get(i).copied().or(block);
        let d: DecoratorFile = match serde_yaml::from_value(item.clone()) {
            Ok(d) => d,
            Err(e) => {
                // `model:` is the old name of `dataset:` (the manifest-wide
                // renamed-key list can't say so: `model` is valid elsewhere).
                let old_model = item.get("model").is_some() && item.get("dataset").is_none();
                let d = if old_model {
                    at(
                        "decorator.unknown_key",
                        file,
                        item_line,
                        "`ui.decorators`: unknown key `model`",
                    )
                    .fix("write `dataset:`")
                } else {
                    at(
                        "decorator.invalid",
                        file,
                        item_line,
                        format!("`ui.decorators`: {e}"),
                    )
                };
                errors.push(d);
                continue;
            }
        };
        let line = item_line;
        let dataset = datasets.iter().find(|m| m.decl.name == d.dataset);
        let has = |col: &str| dataset.is_some_and(|m| m.decl.fields.iter().any(|c| c.name == col));
        let wanted: Vec<&str> = ["uri", d.label.as_str()]
            .into_iter()
            .chain(d.color.as_deref())
            .collect();
        let problem = if let Some(why) =
            crate::extension::extension_resources::resource_problem(&d.resource)
        {
            Some((
                "decorator.invalid_resource",
                format!("`resource: {}`: {why}", d.resource),
                None,
            ))
        } else if dataset.is_none() {
            Some((
                "decorator.unknown_dataset",
                format!(
                    "`dataset: {}` isn't one of this extension's datasets (`datasets:`)",
                    d.dataset
                ),
                None,
            ))
        } else if let Some(bad) = wanted.iter().find(|c| !identifier(c)) {
            Some((
                "decorator.invalid_column",
                format!("`{bad}` isn't a column name"),
                None,
            ))
        } else if let Some(missing) = wanted.iter().find(|c| !has(c)) {
            Some((
                "decorator.missing_column",
                format!("dataset `{}` has no `{missing}` column", d.dataset),
                Some("a decorator's dataset needs `uri` and the columns it names"),
            ))
        } else {
            None
        };
        if let Some((code, p, fix)) = problem {
            let d = at(code, file, line, format!("`ui.decorators`: {p}"));
            errors.push(match fix {
                Some(fix) => d.fix(fix),
                None => d,
            });
            continue;
        }
        out.push(UiDecorator {
            // Positional and never stored: its extension's, so two in a
            // namespace don't meet.
            id: format!("{extension}/{i}"),
            extension: extension.to_string(),
            view: oxplow_db::datasets::extension_view(namespace, &d.dataset),
            resource: d.resource,
            placement: d.placement,
            label: d.label,
            color: d.color,
        });
    }
    (out, errors)
}

#[cfg(test)]
mod tests {
    use crate::extension::extensions::{load_extensions, Extension};
    use std::path::Path;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// Extension `flags` with a `flags` dataset and `decorators`.
    fn load(decorators: &str) -> Extension {
        let d = tempfile::tempdir().unwrap();
        write(
            d.path(),
            ".oxplow/extensions/flags/extension.yaml",
            &format!(
                "manifest: 2\nname: flags\nengine: \">=0.1\"\nversion: 0.1.0\nintent:\n  purpose: p\n  examples: [{{ name: a }}]\ndatasets:\n  - name: flags\n    version: 1\n    description: d\n    fields:\n      - {{ name: uri, type: TEXT, doc: r }}\n      - {{ name: label, type: TEXT, doc: l }}\n      - {{ name: color, type: TEXT, doc: c }}\nui:\n  decorators:\n{decorators}"
            ),
        );
        write(
            d.path(),
            ".oxplow/extensions/flags/datasets/flags.sql",
            "SELECT 'oxplow:oxplow/work_item?id=tsk1&provider=oxplow' AS uri, 'urgent' AS label, '#f00' AS color\n",
        );
        load_extensions(d.path())
            .into_iter()
            .find(|e| e.name == "flags")
            .unwrap()
    }

    #[test]
    fn a_decorator_names_one_of_its_datasets_columns() {
        let ext = load("    - { dataset: flags, resource: oxplow/work_item, placement: ref-chip, label: label, color: color }\n",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let d = &ext.ui.decorators[0];
        assert_eq!(d.view, "v_flags_flags");
        assert_eq!(d.placement, super::DecoratorPlacement::RefChip);
        assert_eq!(d.color.as_deref(), Some("color"));
        for (entry, says) in [
            (
                "{ dataset: nope, resource: oxplow/work_item, placement: ref-chip, label: label }",
                "isn't one of this extension's datasets",
            ),
            (
                "{ dataset: flags, resource: oxplow/nope, placement: ref-chip, label: label }",
                "oxplow declares no such resource",
            ),
            (
                "{ dataset: flags, resource: oxplow/work_item, placement: ref-chip, label: title }",
                "has no `title` column",
            ),
            (
                "{ dataset: flags, resource: oxplow/work_item, placement: ref-chip, label: \"x; DROP\" }",
                "isn't a column name",
            ),
            (
                "{ dataset: flags, resource: oxplow/work_item, placement: sidebar, label: label }",
                "unknown variant",
            ),
        ] {
            let ext = load(&format!("    - {entry}\n"));
            let errs = ext.errors.iter().map(|e| ext.shown(e)).collect::<Vec<_>>().join("\n");
            assert!(
                errs.contains(says) && errs.contains("extension.yaml:"),
                "{entry}: {errs}"
            );
            assert!(ext.ui.decorators.is_empty(), "{entry}");
        }
    }

    /// A decorator's old `model:` key says what to write.
    #[test]
    fn an_old_model_key_says_to_write_dataset() {
        let ext = load("    - { model: flags, resource: oxplow/work_item, placement: ref-chip, label: label }\n");
        let errs = ext
            .errors
            .iter()
            .map(|e| ext.shown(e))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            ext.errors
                .iter()
                .any(|e| e.message.contains("unknown key `model`")
                    && e.fix.as_deref() == Some("write `dataset:`")
                    && e.file.as_deref() == Some("extension.yaml")
                    && e.line.is_some()),
            "{errs}"
        );
        assert!(ext.ui.decorators.is_empty());
    }

    /// P10: stable, so a shared extension's decorators load like a
    /// private one's.
    #[test]
    fn a_shared_extension_may_declare_decorators() {
        let ext =
            load("    - { dataset: flags, resource: oxplow/work_item, placement: ref-chip, label: label }\n");
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(ext.ui.decorators.len(), 1);
    }
}
