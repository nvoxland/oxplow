//! `ui.mounts` (P9.A1, stable; `.context/extensions-slots.md` →
//! "Replacements"): a page that takes the place of a named core
//! sub-component ([`oxplow_domain::replaceable`]).
//!
//! ```yaml
//! ui:
//!   replacements:
//!     - { page: board, at: task.board, version: 1 }
//! ```
//!
//! Checked here: the target exists, the page is the extension's and
//! declares every prop of the target's contract, a pinned `version:` is
//! the contract's, and — for a target bound to the provider or store that
//! holds the thing it shows — the extension brings a provider of the
//! capability, or fills the model as a store.
//! *Which* replacement renders is decided when the component renders (the
//! active provider changes without a reload; the person's choice for an
//! unbound target is config), so that rule is the renderer's.

use oxplow_domain::diagnostic::Diagnostic;
use serde::{Deserialize, Serialize};

use super::manifest_v2::at;

/// A replacement (valid ones; invalid ones are in the extension's
/// `errors`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UiReplacement {
    /// `<extension>/<target>`.
    pub id: String,
    pub extension: String,
    /// The core sub-component it replaces (`task.board`).
    pub target: String,
    /// The capability the target belongs to; empty for a model's target.
    pub capability: String,
    /// The shared model the target shows items of (`oxplow/task_record`),
    /// for a target of a model rather than a capability.
    pub model: Option<String>,
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

/// The person's `replacements` value that keeps oxplow's own component.
pub const OXPLOWS_OWN: &str = "oxplow";

/// Which of `candidates` — the enabled extensions' replacements of one
/// target — stands in for it now, if any. `choice` is what the person said
/// for the target (`replacements: { <target>: <extension> | oxplow }`);
/// `oxplow` keeps oxplow's own wherever. A target bound to a provider (a
/// task's state: that tracker's own states) shows only the
/// replacement of `provider_extension`, the extension bringing the
/// provider whose thing it shows; none keeps oxplow's own. An unbound one
/// (the Board, the list, the history graph) shows the person's choice, or
/// with none the only candidate; several and no choice keep oxplow's own.
pub fn chosen<'a>(
    candidates: &[&'a UiReplacement],
    choice: Option<&str>,
    provider_extension: Option<&str>,
) -> Option<&'a UiReplacement> {
    let first = candidates.first()?;
    if choice == Some(OXPLOWS_OWN) {
        return None;
    }
    let decider = if first.bound_to_provider {
        provider_extension?
    } else {
        match (choice, candidates) {
            (Some(said), _) => said,
            (None, [only]) => only.extension.as_str(),
            (None, _) => return None,
        }
    };
    candidates.iter().copied().find(|c| c.extension == decider)
}

/// A `ui.mounts` entry at a core component, as `manifest_v2::mounts_of`
/// sorts it out.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplacementFile {
    target: String,
    /// The page that renders instead: a slug of this extension.
    page: String,
    /// The props contract's version it was written against; checked
    /// against the table's, so a changed contract refuses it rather than
    /// handing it props it doesn't know.
    version: Option<u32>,
}

impl ReplacementFile {
    pub fn new(target: String, page: String, version: Option<u32>) -> Self {
        Self {
            target,
            page,
            version,
        }
    }
}

/// Check the `ui.mounts` entries at core components against the
/// extension's pages and what it holds (`held`: the capabilities its
/// providers bring, and `model:<id>` for each model it fills as a store):
/// the valid ones, and an error (`file:line: …`) for each broken one.
pub fn parse_replacements(
    extension: &str,
    namespace: &str,
    held: &[String],
    pages: &[crate::pages::schema::Page],
    items: Vec<(Option<usize>, ReplacementFile)>,
    file: &str,
) -> (Vec<UiReplacement>, Vec<Diagnostic>) {
    let mut out: Vec<UiReplacement> = Vec::new();
    let mut errors = Vec::new();
    for (line, r) in items {
        let target = oxplow_domain::replaceable::replaceable(&r.target);
        // What renders instead: its params are what the contract's props
        // are checked against.
        let name = format!("page `{}`", r.page);
        let found = pages.iter().find(|p| p.slug == r.page);
        let declared: Option<Vec<String>> =
            found.map(|p| p.params.iter().map(|x| x.name.clone()).collect());
        let wrong_kind = found.and_then(|p| {
            crate::pages::refs::kind_problem(&crate::pages::refs::RefSite::Replacement, p)
        });
        let problem = match (target, declared.as_ref()) {
            (Some(_), Some(_)) if wrong_kind.is_some() => {
                Some(("replacement.unknown_page", wrong_kind.unwrap_or_default()))
            }
            (None, _) => Some((
                "replacement.unknown_target",
                format!(
                    "`at: {}` is neither a page's slot (`<extension>/<page>.<slot>`) nor a \
                     replaceable component ({})",
                    r.target,
                    oxplow_domain::replaceable::targets()
                ),
            )),
            (Some(_), None) => Some((
                "replacement.unknown_page",
                format!("{name} isn't one of this extension's"),
            )),
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
                    Some((
                        "replacement.missing_params",
                        format!(
                            "{name} must declare {} (`params:`) — what `{}` gives its replacement",
                            missing.join(", "),
                            t.target
                        ),
                    ))
                } else if r.version.is_some_and(|v| v != t.version) {
                    Some((
                        "replacement.version_mismatch",
                        format!(
                            "`{}` is at version {} of its props contract, not {} (`v_replaceable`)",
                            t.target,
                            t.version,
                            r.version.unwrap_or_default()
                        ),
                    ))
                } else if t.bound_to_provider && !held.iter().any(|c| *c == t.subject()) {
                    Some((
                        "replacement.provider_required",
                        match t.model {
                            Some(model) => format!(
                                "only an extension that fills `{model}` as a store (`fills:`) \
                                 may replace `{}`; it shows for the items of that store",
                                t.target
                            ),
                            None => format!(
                                "only an extension that brings a `{}` provider may replace `{}` \
                                 (`providers:`); it shows while that provider is the active one",
                                t.capability, t.target
                            ),
                        },
                    ))
                } else if out.iter().any(|o| o.target == t.target) {
                    Some((
                        "replacement.duplicate",
                        format!("`{}` is already replaced above", t.target),
                    ))
                } else {
                    None
                }
            }
        };
        if let Some((code, p)) = problem {
            errors.push(at(code, file, line, format!("`ui.mounts`: {p}")));
            continue;
        }
        let Some(t) = target else {
            continue;
        };
        out.push(UiReplacement {
            id: format!("{namespace}/{}", t.target),
            extension: extension.to_string(),
            target: t.target.to_string(),
            capability: t.capability.to_string(),
            model: t.model.map(str::to_string),
            bound_to_provider: t.bound_to_provider,
            page: format!("{namespace}/{}", r.page),
            version: t.version,
            view: t.view.iter().map(|k| k.to_string()).collect(),
            label: t.label.to_string(),
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

    /// `extension`'s replacement of `target`, bound to its provider or not.
    fn candidate(extension: &str, target: &str, bound: bool) -> super::UiReplacement {
        super::UiReplacement {
            id: format!("{extension}/{target}"),
            extension: extension.into(),
            target: target.into(),
            capability: "vcs".into(),
            model: None,
            bound_to_provider: bound,
            page: format!("{extension}/page"),
            version: 1,
            view: Vec::new(),
            label: target.into(),
        }
    }

    /// Whose page stands in, as the person and the thing shown say: an
    /// unbound target's only candidate, or the one the person chose;
    /// several and no choice, or `oxplow` chosen, keep oxplow's own; a bound
    /// one only the extension of the provider whose thing it shows.
    #[test]
    fn whose_replacement_shows() {
        let (a, b) = (
            candidate("a", "task.board", false),
            candidate("b", "task.board", false),
        );
        let named = |c: Option<&super::UiReplacement>| c.map(|c| c.extension.clone());
        assert_eq!(named(super::chosen(&[], None, None)), None);
        assert_eq!(named(super::chosen(&[&a], None, None)), Some("a".into()));
        assert_eq!(named(super::chosen(&[&a, &b], None, None)), None);
        assert_eq!(
            named(super::chosen(&[&a, &b], Some("b"), None)),
            Some("b".into())
        );
        assert_eq!(
            named(super::chosen(&[&a], Some(super::OXPLOWS_OWN), None)),
            None
        );
        assert_eq!(named(super::chosen(&[&a], Some("nobody"), None)), None);
        let (x, y) = (
            candidate("x", "task.detail.state", true),
            candidate("y", "task.detail.state", true),
        );
        assert_eq!(
            named(super::chosen(&[&x, &y], None, Some("y"))),
            Some("y".into())
        );
        assert_eq!(
            named(super::chosen(&[&x, &y], Some("x"), Some("y"))),
            Some("y".into())
        );
        assert_eq!(
            named(super::chosen(&[&x], None, None)),
            None,
            "whose item it is decides"
        );
        assert_eq!(
            named(super::chosen(&[&x], Some(super::OXPLOWS_OWN), Some("x"))),
            None
        );
    }

    const BOARD: &str = "title: Board\nparams: [{ name: scope }, { name: thread_id }]\nloaders:\n  rows: { sql: \"SELECT uri, title, mark FROM v_thread\" }\nview: { viz: table, from: rows }\n";

    /// Extension `tracker`, with `providers` (YAML, or none),
    /// a `board` page taking the Board's whole contract, a `partial` one
    /// taking only `scope`, and `replacements`.
    fn load(providers: &str, replacements: &str) -> Extension {
        let d = tempfile::tempdir().unwrap();
        write(
            d.path(),
            ".oxplow/extensions/tracker/extension.yaml",
            &format!(
                "manifest: 2\nname: tracker\nengine: \">=0.1\"\nintent:\n  purpose: p\n  examples: [{{ name: a }}]\n{providers}ui:\n  mounts:\n{replacements}"
            ),
        );
        write(
            d.path(),
            ".oxplow/extensions/tracker/pages/board.yaml",
            BOARD,
        );
        write(
            d.path(),
            ".oxplow/extensions/tracker/pages/partial.yaml",
            "title: Partial\nparams: [{ name: scope }]\nloaders:\n  rows: { sql: \"SELECT 1 AS n\" }\nview: { viz: table, from: rows }\n",
        );
        write(
            d.path(),
            ".oxplow/extensions/tracker/pages/state.yaml",
            "title: State\nparams: [{ name: uri }]\nloaders:\n  rows: { sql: \"SELECT :uri AS uri\" }\nview: { viz: table, from: rows }\n",
        );
        write(
            d.path(),
            ".oxplow/extensions/tracker/pages/graph.yaml",
            "title: Graph\nparams: [{ name: head_sha }, { name: reveal_sha }, { name: limit }]\nloaders:\n  rows: { sql: \"SELECT :head_sha AS sha\" }\nview: { viz: table, from: rows }\n",
        );
        write(
            d.path(),
            ".oxplow/extensions/tracker/pages/board-page.yaml",
            "title: Board\nparams: [{ name: scope }, { name: thread_id }]\nloaders:\n  items: { sql: \"SELECT uri, title, mark FROM v_thread\" }\nview: { viz: table, from: items }\n",
        );
        load_extensions(d.path())
            .into_iter()
            .find(|e| e.name == "tracker")
            .unwrap()
    }

    /// What a replacement's own checks said (the provider's missing
    /// program and declarations are another check's errors).
    fn said(ext: &Extension) -> String {
        ext.errors
            .iter()
            .filter(|e| e.message.contains("ui.mounts"))
            .map(|e| ext.shown(e))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Every replaceable target has a name a person reads (Settings →
    /// Integrations, the replaced badge): the desktop's
    /// `REPLACEABLE_LABELS` names each one.
    /// Every target has a label a person reads, and no two share one.
    #[test]
    fn every_replaceable_target_has_its_own_label() {
        let labels: Vec<&str> = oxplow_domain::replaceable::all()
            .iter()
            .map(|r| r.label)
            .collect();
        assert!(labels.iter().all(|l| !l.trim().is_empty()), "{labels:?}");
        let distinct: std::collections::BTreeSet<_> = labels.iter().collect();
        assert_eq!(distinct.len(), labels.len(), "{labels:?}");
    }

    #[test]
    fn a_target_is_replaced_once_per_extension() {
        let ext = load(
            "",
            "    - { page: graph, at: vcs.history.graph }\n    - { page: graph, at: vcs.history.graph }\n",
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
        let ext = load("", "    - { page: graph, at: vcs.history.graph }\n");
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
        for r in oxplow_domain::replaceable::all() {
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
}
