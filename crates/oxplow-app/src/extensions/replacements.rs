//! `ui.replacements` (P9.A1, stable; `.context/extensions.md` →
//! "Replacements"): a page that takes the place of a named core
//! sub-component ([`oxplow_domain::replaceable`]).
//!
//! ```yaml
//! ui:
//!   replacements:
//!     - { target: work_item.board, page: board, version: 1 }
//! ```
//!
//! Checked here: the target exists, the page is the extension's and
//! declares every prop of the target's contract, a pinned `version:` is
//! the contract's, and — for a target bound to its capability's provider
//! — the extension brings a provider of it.
//! *Which* replacement renders is decided when the component renders (the
//! active provider changes without a reload; the person's choice for an
//! unbound target is config), so that rule is the renderer's.

use serde::{Deserialize, Serialize};

use super::manifest_v2::{at, item_lines, key_line, line_under};

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
    /// The capability the target belongs to.
    pub capability: String,
    /// Only while this extension's provider is the deciding one does it
    /// render; otherwise the person's choice among candidates decides.
    pub bound_to_provider: bool,
    /// The page that renders instead (`<extension>/<slug>`), given the
    /// target's props.
    pub page: String,
    /// The contract's version it was written against (the table's).
    pub version: u32,
    /// The host page's view params it shares (the table's `view`).
    pub view: Vec<String>,
    /// What a person calls the target (`board`), from its table.
    pub label: String,
}

/// A `ui.replacements` entry as the manifest holds it.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplacementFile {
    target: String,
    /// The page that renders instead: a slug of this extension.
    page: String,
    /// The props contract's version it was written against; checked
    /// against the table's, so a changed contract refuses it rather than
    /// handing it props it doesn't know.
    #[serde(default)]
    version: Option<u32>,
}

/// Parse `ui.replacements` against the extension's pages and the
/// capabilities its providers bring: the valid ones, and an error
/// (`file:line: …`) for each broken one.
pub fn parse_replacements(
    extension: &str,
    capabilities: &[String],
    pages: &[crate::pages::schema::Page],
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
    let lines = item_lines(manifest, "ui.replacements");
    for (i, item) in items.iter().enumerate() {
        let item_line = lines.get(i).copied().or(block);
        let r: ReplacementFile = match serde_yaml::from_value(item.clone()) {
            Ok(r) => r,
            Err(e) => {
                errors.push(at(file, item_line, format!("`ui.replacements`: {e}")));
                continue;
            }
        };
        let line = item_line;
        let target = oxplow_domain::replaceable::replaceable(&r.target);
        // What renders instead: its params are what the contract's props
        // are checked against.
        let name = format!("page `{}`", r.page);
        let declared: Option<Vec<String>> = pages
            .iter()
            .find(|p| p.slug == r.page)
            .map(|p| p.params.iter().map(|x| x.name.clone()).collect());
        let problem = match (target, declared.as_ref()) {
            (None, _) => Some(format!(
                "`target: {}` isn't a replaceable component ({})",
                r.target,
                oxplow_domain::replaceable::targets()
            )),
            (Some(_), None) => Some(format!("{name} isn't one of this extension's")),
            (Some(t), Some(params)) => {
                // The contract: the replacement is given these and nothing
                // of the host's, so it takes them all.
                let missing: Vec<String> = t
                    .props
                    .iter()
                    .filter(|p| !params.iter().any(|lp| lp == **p))
                    .map(|p| format!("`{p}`"))
                    .collect();
                if !missing.is_empty() {
                    Some(format!(
                        "{name} must declare {} (`params:`) — what `{}` gives its replacement",
                        missing.join(", "),
                        t.target
                    ))
                } else if r.version.is_some_and(|v| v != t.version) {
                    Some(format!(
                        "`{}` is at version {} of its props contract, not {} (`v_replaceable`)",
                        t.target,
                        t.version,
                        r.version.unwrap_or_default()
                    ))
                } else if t.bound_to_provider && !capabilities.iter().any(|c| c == t.capability) {
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
        let Some(t) = target else {
            continue;
        };
        out.push(UiReplacement {
            id: format!("{extension}/{}", t.target),
            extension: extension.to_string(),
            target: t.target.to_string(),
            capability: t.capability.to_string(),
            bound_to_provider: t.bound_to_provider,
            page: format!("{extension}/{}", r.page),
            version: t.version,
            view: t.view.iter().map(|k| k.to_string()).collect(),
            label: t.label.to_string(),
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

    const BOARD: &str = "title: Board\nparams: [{ name: scope }, { name: thread_id }]\ndata:\n  rows: { sql: \"SELECT uri, title, state FROM v_work_item\" }\nview: { viz: table, from: rows }\n";

    /// Extension `tracker`, with `providers` (YAML, or none),
    /// a `board` page taking the Board's whole contract, a `partial` one
    /// taking only `scope`, and `replacements`.
    fn load(providers: &str, replacements: &str) -> Extension {
        let d = tempfile::tempdir().unwrap();
        write(
            d.path(),
            ".oxplow/extensions/tracker/extension.yaml",
            &format!(
                "manifest: 2\nname: tracker\nengine: \">=0.1\"\nintent:\n  purpose: p\n  examples: [{{ name: a }}]\n{providers}ui:\n  replacements:\n{replacements}"
            ),
        );
        // The provider's own files, so it loads (the fake's declarations).
        write(
            d.path(),
            ".oxplow/extensions/tracker/provider.json",
            &serde_json::to_string_pretty(&oxplow_provider_fake::declarations()).unwrap(),
        );
        write(
            d.path(),
            ".oxplow/extensions/tracker/bin/provider",
            "#!/bin/sh\nexit 1\n",
        );
        write(
            d.path(),
            ".oxplow/extensions/tracker/pages/board.yaml",
            BOARD,
        );
        write(
            d.path(),
            ".oxplow/extensions/tracker/pages/partial.yaml",
            "title: Partial\nparams: [{ name: scope }]\ndata:\n  rows: { sql: \"SELECT 1 AS n\" }\nview: { viz: table, from: rows }\n",
        );
        write(
            d.path(),
            ".oxplow/extensions/tracker/pages/state.yaml",
            "title: State\nparams: [{ name: uri }]\ndata:\n  rows: { sql: \"SELECT :uri AS uri\" }\nview: { viz: table, from: rows }\n",
        );
        write(
            d.path(),
            ".oxplow/extensions/tracker/pages/graph.yaml",
            "title: Graph\nparams: [{ name: head_sha }, { name: reveal_sha }, { name: limit }]\ndata:\n  rows: { sql: \"SELECT :head_sha AS sha\" }\nview: { viz: table, from: rows }\n",
        );
        write(
            d.path(),
            ".oxplow/extensions/tracker/pages/verdict.yaml",
            "title: Verdict\nparams: [{ name: effort_id }]\ndata:\n  rows: { sql: \"SELECT :effort_id AS effort_id\" }\nview: { viz: table, from: rows }\n",
        );
        write(
            d.path(),
            ".oxplow/extensions/tracker/pages/board-page.yaml",
            "title: Board\nparams: [{ name: scope }, { name: thread_id }]\ndata:\n  items: { sql: \"SELECT uri, title, state FROM v_work_item\" }\nview: { viz: table, from: items }\n",
        );
        load_extensions(d.path())
            .into_iter()
            .find(|e| e.name == "tracker")
            .unwrap()
    }

    /// A page renders instead, checked against the target's contract; a
    /// `lens:` is no longer a field.
    #[test]
    fn a_page_replaces_a_target_taking_its_whole_contract() {
        let ext = load(
            PROVIDER,
            "    - { target: work_item.board, page: board-page }\n",
        );
        assert_eq!(
            said(&ext),
            "",
            "pages: {:?}, errors: {:?}",
            ext.page_files.iter().map(|p| &p.id).collect::<Vec<_>>(),
            ext.errors
        );
        assert_eq!(ext.ui.replacements[0].page, "tracker/board-page");
        let ext = load(PROVIDER, "    - { target: work_item.board, page: nope }\n");
        assert!(
            said(&ext).contains("page `nope` isn't one of this extension's"),
            "{}",
            said(&ext)
        );
        let ext = load(PROVIDER, "    - { target: work_item.board, lens: board }\n");
        assert!(
            said(&ext).contains("unknown field `lens`"),
            "{}",
            said(&ext)
        );
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
    fn a_replacement_names_a_known_target_and_a_page_taking_its_whole_contract() {
        let ext = load(PROVIDER, "    - { target: work_item.board, page: board }\n");
        assert_eq!(said(&ext), "");
        assert_eq!(
            ext.ui.replacements,
            vec![super::UiReplacement {
                id: "tracker/work_item.board".into(),
                extension: "tracker".into(),
                target: "work_item.board".into(),
                capability: "work_items".into(),
                page: "tracker/board".into(),
                version: 1,
                view: vec![],
                label: "board".into(),
                bound_to_provider: false,
            }]
        );
        for (entry, says) in [
            (
                "{ target: vcs.conflict.resolver, page: board }",
                "isn't a replaceable component (work_item.board, work_item.list, work_item.detail, work_item.detail.state, vcs.history.graph, effort.verdict)",
            ),
            (
                "{ target: work_item.board, page: board, version: 2 }",
                "`work_item.board` is at version 1 of its props contract, not 2",
            ),
            (
                "{ target: work_item.board, page: nope }",
                "page `nope` isn't one of this extension's",
            ),
            (
                "{ target: work_item.board, page: partial }",
                "must declare `thread_id`",
            ),
            (
                "{ target: work_item.board, page: board, order: 1 }",
                "unknown field",
            ),
        ] {
            let ext = load(PROVIDER, &format!("    - {entry}\n"));
            let errs = said(&ext);
            assert!(
                errs.contains(says) && errs.contains("extension.yaml:"),
                "{entry}: {errs}"
            );
            assert!(ext.ui.replacements.is_empty(), "{entry}");
        }
    }

    /// P10 (K4): the second replaceable component — a work item's state
    /// control — is given the item's `ref`, and its page must take it.
    #[test]
    fn a_detail_state_replacement_takes_ref() {
        let ext = load(
            PROVIDER,
            "    - { target: work_item.detail.state, page: state }\n",
        );
        assert_eq!(said(&ext), "");
        assert_eq!(
            ext.ui
                .replacements
                .iter()
                .map(|r| (r.target.as_str(), r.page.as_str()))
                .collect::<Vec<_>>(),
            vec![("work_item.detail.state", "tracker/state")]
        );
        let ext = load(
            PROVIDER,
            "    - { target: work_item.detail.state, page: board }\n",
        );
        assert!(said(&ext).contains("must declare `uri`"), "{}", said(&ext));
    }

    /// An effort's verdict is a review policy's: any extension may bring
    /// one, with no provider, and its page takes the effort's `effort_id`.
    #[test]
    fn an_effort_verdict_replacement_takes_effort_id_and_needs_no_provider() {
        let ext = load("", "    - { target: effort.verdict, page: verdict }\n");
        assert_eq!(said(&ext), "");
        assert_eq!(
            ext.ui
                .replacements
                .iter()
                .map(|r| (
                    r.target.as_str(),
                    r.capability.as_str(),
                    r.bound_to_provider
                ))
                .collect::<Vec<_>>(),
            vec![("effort.verdict", "effort_policy", false)]
        );
        let ext = load("", "    - { target: effort.verdict, page: state }\n");
        assert!(
            said(&ext).contains("must declare `effort_id`"),
            "{}",
            said(&ext)
        );
    }

    /// Every replaceable target has a name a person reads (Settings →
    /// Integrations, the replaced badge): the desktop's
    /// `REPLACEABLE_LABELS` names each one.
    /// Every target has a label a person reads, and no two share one.
    #[test]
    fn every_replaceable_target_has_its_own_label() {
        let labels: Vec<&str> = oxplow_domain::replaceable::REPLACEABLE
            .iter()
            .map(|r| r.label)
            .collect();
        assert!(labels.iter().all(|l| !l.trim().is_empty()), "{labels:?}");
        let distinct: std::collections::BTreeSet<_> = labels.iter().collect();
        assert_eq!(distinct.len(), labels.len(), "{labels:?}");
    }

    /// A bound target (the state control: a provider's own states) needs
    /// the extension to bring that provider; the Board, the list and the
    /// detail editor are views of the capability as a whole, so any
    /// extension may bring one, pinning the contract's version or not.
    #[test]
    fn only_a_bound_target_needs_the_extension_to_bring_its_provider() {
        let ext = load(
            "",
            "    - { target: work_item.detail.state, page: state }\n",
        );
        let errs = said(&ext);
        assert!(
            errs.contains("only an extension that brings a `work_items` provider may replace `work_item.detail.state`"),
            "{errs}"
        );
        assert!(ext.ui.replacements.is_empty());
        let ext = load("",
            "    - { target: work_item.board, page: board, version: 1 }\n    - { target: work_item.list, page: board }\n    - { target: work_item.detail, page: state }\n",
        );
        assert_eq!(said(&ext), "");
        assert_eq!(
            ext.ui
                .replacements
                .iter()
                .map(|r| (r.target.as_str(), r.bound_to_provider, r.version))
                .collect::<Vec<_>>(),
            vec![
                ("work_item.board", false, 1),
                ("work_item.list", false, 1),
                ("work_item.detail", false, 1)
            ]
        );
    }

    #[test]
    fn a_target_is_replaced_once_per_extension() {
        let ext = load(PROVIDER,
            "    - { target: work_item.board, page: board }\n    - { target: work_item.board, page: board }\n",
        );
        assert!(said(&ext).contains("already replaced"), "{:?}", ext.errors);
        assert_eq!(ext.ui.replacements.len(), 1);
    }

    /// A target core owns (its capability's implementation is oxplow's
    /// own, `vcs`) isn't bound to a provider: any extension — a shared
    /// one, oxplow-bundled — may replace it, and the person chooses among
    /// several (`replacements:`). It carries the host page's view it
    /// shares, Git History's filters.
    #[test]
    fn an_unbound_target_needs_no_provider_and_a_shared_extension_may_replace_it() {
        let ext = load("", "    - { target: vcs.history.graph, page: graph }\n");
        assert_eq!(said(&ext), "");
        assert_eq!(
            ext.ui
                .replacements
                .iter()
                .map(|r| (r.target.as_str(), r.bound_to_provider, r.view.join(",")))
                .collect::<Vec<_>>(),
            vec![("vcs.history.graph", false, "q,author,branch".to_string())]
        );
    }

    /// The reference's table of replaceable views, from the table: the
    /// two can't drift. `OXPLOW_BLESS=1` rewrites it.
    #[test]
    fn the_reference_lists_every_replaceable_view() {
        let mut table = String::from(
            "| Target | What it is | Bound to a provider | Version | Props | View |\n|---|---|---|---|---|---|\n",
        );
        for r in oxplow_domain::replaceable::REPLACEABLE {
            table.push_str(&format!(
                "| `{}` | {} | {} | {} | {} | {} |\n",
                r.target,
                r.label,
                if r.bound_to_provider { "yes" } else { "no" },
                r.version,
                r.props
                    .iter()
                    .map(|p| format!("`{p}`"))
                    .collect::<Vec<_>>()
                    .join(", "),
                if r.view.is_empty() {
                    "—".to_string()
                } else {
                    r.view
                        .iter()
                        .map(|k| format!("`{k}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            ));
        }
        crate::test_fixtures::generated_section(
            "docs/reference/extensions.md",
            "replaceables",
            &table,
        );
    }

    /// Replacements are stable: a shared extension may declare one; a
    /// bound target still needs its provider.
    #[test]
    fn a_shared_extension_may_declare_replacements() {
        let ext = load(PROVIDER, "    - { target: work_item.board, page: board }\n");
        assert_eq!(said(&ext), "");
        assert_eq!(ext.ui.replacements.len(), 1);
    }
}
