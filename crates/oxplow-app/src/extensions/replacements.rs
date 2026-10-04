//! `ui.replacements` (experimental — a private extension only; P9.A1,
//! `.context/extensions.md` → "Replacements"): a lens that takes the
//! place of a named core sub-component ([`oxplow_domain::replaceable`]).
//!
//! ```yaml
//! ui:
//!   replacements:
//!     - { target: work_item.board, lens: board }
//! ```
//!
//! Checked here: the target exists, the lens is the extension's and
//! declares every prop of the target's contract, and the extension brings
//! a provider of the target's capability. *Which* provider is active is
//! decided when the component renders (`v_capability_provider.active`
//! changes without a reload), so that rule is the renderer's.

use serde::{Deserialize, Serialize};

use super::manifest_v2::{at, entry_line, key_line, line_under};
use super::Lens;

/// A replacement (valid ones; invalid ones are in the extension's
/// `errors`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UiReplacement {
    /// `<extension>/<target>`.
    pub id: String,
    pub extension: String,
    /// The core sub-component it replaces (`work_item.board`).
    pub target: String,
    /// The capability whose active provider must be this extension's for
    /// it to render.
    pub capability: String,
    /// The lens that renders instead, given the target's props.
    pub lens_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplacementFile {
    target: String,
    lens: String,
}

/// Parse `ui.replacements` against the extension's lenses and the
/// capabilities its providers bring: the valid ones, and an error
/// (`file:line: …`) for each broken one.
pub fn parse_replacements(
    extension: &str,
    capabilities: &[String],
    lenses: &[Lens],
    value: &serde_yaml::Value,
    file: &str,
    manifest: &str,
) -> (Vec<UiReplacement>, Vec<String>) {
    let block = line_under(manifest, "ui", "replacements:").or(key_line(manifest, "ui"));
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(file, block, "`ui.replacements` must be a list")],
        );
    };
    let mut out: Vec<UiReplacement> = Vec::new();
    let mut errors = Vec::new();
    for item in items {
        let r: ReplacementFile = match serde_yaml::from_value(item.clone()) {
            Ok(r) => r,
            Err(e) => {
                errors.push(at(file, block, format!("`ui.replacements`: {e}")));
                continue;
            }
        };
        let line = entry_line(manifest, "ui", "target", &r.target).or(block);
        let target = oxplow_domain::replaceable::replaceable(&r.target);
        let lens = lenses.iter().find(|l| l.slug == r.lens);
        let problem = match (target, lens) {
            (None, _) => Some(format!(
                "`target: {}` isn't a replaceable component ({})",
                r.target,
                oxplow_domain::replaceable::targets()
            )),
            (Some(_), None) => Some(format!(
                "`lens: {}` isn't one of this extension's lenses",
                r.lens
            )),
            (Some(t), Some(l)) => {
                // The contract: the replacement is given these and nothing
                // of the host's, so it takes them all.
                let missing: Vec<String> = t
                    .props
                    .iter()
                    .filter(|p| !l.params.iter().any(|lp| lp.name == **p))
                    .map(|p| format!("`{p}`"))
                    .collect();
                if !missing.is_empty() {
                    Some(format!(
                        "lens `{}` must declare {} (`params:`) — what `{}` gives its replacement",
                        r.lens,
                        missing.join(", "),
                        t.target
                    ))
                } else if !capabilities.iter().any(|c| c == t.capability) {
                    Some(format!(
                        "only an extension that brings a `{}` provider may replace `{}` \
                         (`providers:`); it shows while that provider is the active one",
                        t.capability, t.target
                    ))
                } else if out.iter().any(|o| o.target == t.target) {
                    Some(format!("`{}` is already replaced above", t.target))
                } else {
                    None
                }
            }
        };
        if let Some(p) = problem {
            errors.push(at(file, line, format!("`ui.replacements`: {p}")));
            continue;
        }
        let (Some(t), Some(l)) = (target, lens) else {
            continue;
        };
        out.push(UiReplacement {
            id: format!("{extension}/{}", t.target),
            extension: extension.to_string(),
            target: t.target.to_string(),
            capability: t.capability.to_string(),
            lens_id: l.id.clone(),
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

    const BOARD: &str = "title: Board\nparams: [{ name: scope }, { name: thread_id }]\nquery: SELECT ref, title, state FROM v_work_item\n";

    /// Extension `tracker` (`sharing`), with `providers` (YAML, or none),
    /// a `board` lens taking the Board's whole contract, a `partial` one
    /// taking only `scope`, and `replacements`.
    fn load(sharing: &str, providers: &str, replacements: &str) -> Extension {
        let d = tempfile::tempdir().unwrap();
        write(
            d.path(),
            "oxplow/extensions/tracker/extension.yaml",
            &format!(
                "manifest: 2\nname: tracker\nsharing: {sharing}\nengine: \">=0.1\"\nintent:\n  purpose: p\n  examples: [{{ name: a }}]\n{providers}ui:\n  replacements:\n{replacements}"
            ),
        );
        // The provider's own files, so it loads (the fake's declarations).
        write(
            d.path(),
            "oxplow/extensions/tracker/provider.json",
            &serde_json::to_string_pretty(&oxplow_provider_fake::declarations()).unwrap(),
        );
        write(
            d.path(),
            "oxplow/extensions/tracker/bin/provider",
            "#!/bin/sh\nexit 1\n",
        );
        write(
            d.path(),
            "oxplow/extensions/tracker/lenses/board.yaml",
            BOARD,
        );
        write(
            d.path(),
            "oxplow/extensions/tracker/lenses/partial.yaml",
            "title: Partial\nparams: [{ name: scope }]\nquery: SELECT 1 AS n\n",
        );
        write(
            d.path(),
            "oxplow/extensions/tracker/lenses/state.yaml",
            "title: State\nparams: [{ name: ref }]\nquery: SELECT :ref AS ref\n",
        );
        load_extensions(d.path())
            .into_iter()
            .find(|e| e.name == "tracker")
            .unwrap()
    }

    const PROVIDER: &str = "providers:\n  - { id: fake, capability: work_items, entry: bin/provider, declarations: provider.json }\n";

    /// What a replacement's own checks said (the provider's missing
    /// program and declarations are another check's errors).
    fn said(ext: &Extension) -> String {
        ext.errors
            .iter()
            .filter(|e| e.contains("ui.replacements"))
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_replacement_names_a_known_target_and_a_lens_taking_its_whole_contract() {
        let ext = load(
            "private",
            PROVIDER,
            "    - { target: work_item.board, lens: board }\n",
        );
        assert_eq!(said(&ext), "");
        assert_eq!(
            ext.ui.replacements,
            vec![super::UiReplacement {
                id: "tracker/work_item.board".into(),
                extension: "tracker".into(),
                target: "work_item.board".into(),
                capability: "work_items".into(),
                lens_id: "tracker/board".into(),
            }]
        );
        for (entry, says) in [
            (
                "{ target: vcs.history.graph, lens: board }",
                "isn't a replaceable component (work_item.board, work_item.detail.state)",
            ),
            (
                "{ target: work_item.board, lens: nope }",
                "`lens: nope` isn't one of this extension's lenses",
            ),
            (
                "{ target: work_item.board, lens: partial }",
                "must declare `thread_id`",
            ),
            (
                "{ target: work_item.board, lens: board, order: 1 }",
                "unknown field",
            ),
        ] {
            let ext = load("private", PROVIDER, &format!("    - {entry}\n"));
            let errs = said(&ext);
            assert!(
                errs.contains(says) && errs.contains("extension.yaml:"),
                "{entry}: {errs}"
            );
            assert!(ext.ui.replacements.is_empty(), "{entry}");
        }
    }

    /// P10 (K4): the second replaceable component — a work item's state
    /// control — is given the item's `ref`, and its lens must take it.
    #[test]
    fn a_detail_state_replacement_takes_ref() {
        let ext = load(
            "private",
            PROVIDER,
            "    - { target: work_item.detail.state, lens: state }\n",
        );
        assert_eq!(said(&ext), "");
        assert_eq!(
            ext.ui
                .replacements
                .iter()
                .map(|r| (r.target.as_str(), r.lens_id.as_str()))
                .collect::<Vec<_>>(),
            vec![("work_item.detail.state", "tracker/state")]
        );
        let ext = load(
            "private",
            PROVIDER,
            "    - { target: work_item.detail.state, lens: board }\n",
        );
        assert!(said(&ext).contains("must declare `ref`"), "{}", said(&ext));
    }

    /// Every replaceable target has a name a person reads (Settings →
    /// Integrations, the replaced badge): the desktop's
    /// `REPLACEABLE_LABELS` names each one.
    #[test]
    fn every_replaceable_target_has_a_label() {
        let labels = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../apps/desktop/src/lens/useReplacement.ts"),
        )
        .unwrap();
        for r in oxplow_domain::replaceable::REPLACEABLE {
            assert!(
                labels.contains(&format!("\"{}\":", r.target)),
                "`{}` has no label in REPLACEABLE_LABELS",
                r.target
            );
        }
    }

    #[test]
    fn only_an_extension_bringing_a_provider_of_the_capability_may_replace() {
        let ext = load(
            "private",
            "",
            "    - { target: work_item.board, lens: board }\n",
        );
        let errs = said(&ext);
        assert!(
            errs.contains("only an extension that brings a `work_items` provider may replace `work_item.board`"),
            "{errs}"
        );
        assert!(ext.ui.replacements.is_empty());
    }

    #[test]
    fn a_target_is_replaced_once_per_extension() {
        let ext = load(
            "private",
            PROVIDER,
            "    - { target: work_item.board, lens: board }\n    - { target: work_item.board, lens: board }\n",
        );
        assert!(said(&ext).contains("already replaced"), "{:?}", ext.errors);
        assert_eq!(ext.ui.replacements.len(), 1);
    }

    #[test]
    fn a_shared_extension_may_not_declare_replacements() {
        let ext = load(
            "shared",
            "",
            "    - { target: work_item.board, lens: board }\n",
        );
        assert!(
            ext.errors
                .join("\n")
                .contains("`ui.replacements` is experimental"),
            "{:?}",
            ext.errors
        );
        assert!(ext.ui.replacements.is_empty());
    }
}
