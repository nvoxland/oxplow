//! The core sub-components an extension may replace (`ui.replacements`;
//! `.context/extensions.md` "Replacements"): each a named target of one
//! capability with a **versioned props contract** — the params its
//! replacement lens gets, and must declare, instead of any host state.
//! (The viewer's stream isn't a prop: any lens declaring `stream_id` gets
//! it, as everywhere.)
//! Whole pages are never replaceable.
//!
//! One table for the extension loader (which checks a declaration against
//! it), the project config (whose `replacements` names targets), and
//! `v_replaceable`, which publishes it so an author finds the contracts
//! where everything else is: in SQL.

/// One replaceable sub-component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Replaceable {
    /// Its name: `<capability's noun>.<component>`.
    pub target: &'static str,
    /// The capability it belongs to.
    pub capability: &'static str,
    /// Whether only the extension of a provider of `capability` may
    /// replace it — the provider of the thing it shows (a work item's),
    /// else the capability's **active** one. An unbound target may be
    /// replaced by any extension; the person chooses among several
    /// (`replacements:`).
    pub bound_to_provider: bool,
    /// The contract's version: bumped when `props` or their meaning
    /// change. A declaration may pin it (`version:`); one pinned to
    /// another version doesn't load.
    pub version: u32,
    /// What the replacement is given, by name; its lens declares each.
    pub props: &'static [&'static str],
    /// Its host page's view params it shares: the replacement reads them
    /// (`init.view`), writes them (`setViewParams`) and hears them change,
    /// as the core component it replaces does. None: it keeps its own.
    pub view: &'static [&'static str],
    /// What a person calls it ("board"): the one name the UI shows it by.
    pub label: &'static str,
}

/// Every replaceable sub-component.
pub const REPLACEABLE: &[Replaceable] = &[
    // The Board's columns of cards (the `board-cards` part of the `board`
    // document): `scope` is `thread`, `backlog` or `all`; `thread_id` the
    // viewed thread, which applies when `scope` is `thread`. A view of the capability as a whole, so any extension
    // may bring one: it reads `v_work_item` and runs the interface verbs
    // (`oxplow.work_item.transition`, `reorder`, `update`, …), whichever
    // list is active.
    Replaceable {
        target: "work_item.board",
        capability: "work_items",
        bound_to_provider: false,
        version: 1,
        props: &["scope", "thread_id"],
        view: &[],
        label: "board",
    },
    // The Tasks page's list (the `list` part of the `tasks` document):
    // `scope` is `thread`, `stream` or `all`; `thread_id` the thread when
    // `scope` is `thread`, else null. Its `stream_id` is the scope's
    // stream. The page's filters, summary and backlog parts stay oxplow's.
    Replaceable {
        target: "work_item.list",
        capability: "work_items",
        bound_to_provider: false,
        version: 1,
        props: &["scope", "thread_id"],
        view: &[],
        label: "list",
    },
    // A work item's description editor (the `body` part of the `work-item`
    // document): `uri` is the item. Its state, fields, About, links,
    // comments, activity and mounts stay oxplow's.
    Replaceable {
        target: "work_item.detail",
        capability: "work_items",
        bound_to_provider: false,
        version: 1,
        props: &["uri"],
        view: &[],
        label: "description editor",
    },
    // A work item's state control (the `state` part of the `work-item`
    // document; its Move To is never replaced, so an item can always
    // move): `uri` is the item. The item's own provider's extension
    // replaces it, whichever is active: it shows that provider's own
    // states.
    Replaceable {
        target: "work_item.detail.state",
        capability: "work_items",
        bound_to_provider: true,
        version: 1,
        props: &["uri"],
        view: &[],
        label: "state control",
    },
    // The commit history's graph (`GitHistoryPage`, the Git dashboard):
    // `head_sha` the stream's head, `reveal_sha` a commit to scroll to and
    // mark (or null), `limit` how many commits. Its view is Git History's
    // filters (`q`, `author`, `branch`; v2), which core's own list reads
    // too. Core's own is a plain list; the graph is oxplow-bundled's
    // component.
    Replaceable {
        target: "vcs.history.graph",
        capability: "vcs",
        bound_to_provider: false,
        version: 2,
        props: &["head_sha", "reveal_sha", "limit"],
        view: &["q", "author", "branch"],
        label: "history graph",
    },
    // An effort's verdict, the first line of its review page
    // (`DiffViewPage`): `effort_id` is the effort's row id. Core has no
    // verdict of its own: what counts as tested, covered or backed is a
    // review policy's, so with no replacement (or `oxplow` chosen) the
    // page shows none. oxplow-bundled's is the default.
    Replaceable {
        target: "effort.verdict",
        capability: "effort_policy",
        bound_to_provider: false,
        version: 1,
        props: &["effort_id"],
        view: &[],
        label: "effort verdict",
    },
];

/// The `replacements:` value that keeps oxplow's own component.
pub const OXPLOWS_OWN: &str = "oxplow";

/// The replaceable sub-component named `target`.
pub fn replaceable(target: &str) -> Option<&'static Replaceable> {
    REPLACEABLE.iter().find(|r| r.target == target)
}

/// Every target, for an error that lists them.
pub fn targets() -> String {
    REPLACEABLE
        .iter()
        .map(|r| r.target)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every target is its own name and label, versioned from 1, with
    /// at least one prop; `stream_id` is never one.
    #[test]
    fn the_table_holds() {
        let mut targets: Vec<&str> = REPLACEABLE.iter().map(|r| r.target).collect();
        let mut labels: Vec<&str> = REPLACEABLE.iter().map(|r| r.label).collect();
        targets.sort_unstable();
        labels.sort_unstable();
        targets.dedup();
        labels.dedup();
        assert_eq!(targets.len(), REPLACEABLE.len());
        assert_eq!(labels.len(), REPLACEABLE.len());
        for r in REPLACEABLE {
            assert!(r.version >= 1, "{}", r.target);
            assert!(!r.props.is_empty(), "{}", r.target);
            assert!(!r.props.contains(&"stream_id"), "{}", r.target);
            // A target is named for what its capability is about:
            // `work_item.*` for `work_items`, `effort.*` for
            // `effort_policy`, `vcs.*` for `vcs`.
            let noun = r.target.split('.').next().unwrap_or_default();
            assert!(r.capability.starts_with(noun), "{}", r.target);
        }
        assert_eq!(
            replaceable("work_item.board").map(|r| r.bound_to_provider),
            Some(false)
        );
        assert_eq!(
            replaceable("work_item.detail.state").map(|r| r.bound_to_provider),
            Some(true)
        );
        assert_eq!(
            replaceable("effort.verdict").map(|r| (r.bound_to_provider, r.props)),
            Some((false, &["effort_id"][..]))
        );
        assert!(replaceable("vcs.conflict.resolver").is_none());
    }

    /// A target's view is its host page's view params: never one of its
    /// props, and the history graph shares Git History's filters.
    #[test]
    fn a_view_key_is_the_host_pages_not_a_prop() {
        for r in REPLACEABLE {
            for key in r.view {
                assert!(!r.props.contains(key), "{}: {key}", r.target);
            }
        }
        assert_eq!(
            replaceable("vcs.history.graph").map(|r| (r.version, r.view)),
            Some((2, &["q", "author", "branch"][..]))
        );
        assert_eq!(
            replaceable("work_item.board").map(|r| r.view),
            Some(&[][..])
        );
    }
}
