//! The core sub-components an extension may replace (`ui.replacements`;
//! `.context/extensions.md` "Replacements"): each a named target of one
//! capability with a **props contract** — the params its replacement lens
//! gets, and must declare, instead of any host state.
//! (The viewer's stream isn't a prop: any lens declaring `stream_id` gets
//! it, as everywhere.)
//! Whole pages are never replaceable.
//!
//! One table for the extension loader (which checks a declaration against
//! it) and the project config (whose `replacementsOff` names targets).

/// One replaceable sub-component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Replaceable {
    /// Its name: `<capability's noun>.<component>`.
    pub target: &'static str,
    /// The capability it belongs to.
    pub capability: &'static str,
    /// Whether only the extension of a provider of `capability` may
    /// replace it — the provider of the thing it shows (a work item's),
    /// else the capability's **active** one. An unbound target (one core
    /// owns, as the history graph) may be replaced by any extension; the
    /// person chooses among several (`replacements:`).
    pub bound_to_provider: bool,
    /// What the replacement is given, by name; its lens declares each.
    pub props: &'static [&'static str],
    /// What a person calls it ("board"): the one name the UI shows it by.
    pub label: &'static str,
}

/// Every replaceable sub-component.
pub const REPLACEABLE: &[Replaceable] = &[
    // The Board's columns of cards (`WorkBoard`): `scope` is `thread`,
    // `backlog` or `all`; `thread_id` the thread when `scope` is `thread`,
    // else null.
    Replaceable {
        target: "work_item.board",
        capability: "work_items",
        bound_to_provider: true,
        props: &["scope", "thread_id"],
        label: "board",
    },
    // A work item's state control (`WorkItemPage`'s State; its Move To is
    // never replaced, so an item can always move — tsk919): `ref` is the
    // item (P10). The item's own provider's extension replaces it,
    // whichever is active (tsk918).
    Replaceable {
        target: "work_item.detail.state",
        capability: "work_items",
        bound_to_provider: true,
        props: &["ref"],
        label: "state control",
    },
    // The commit history's graph (`GitHistoryPage`, the Git dashboard):
    // `head_sha` the stream's head, `reveal_sha` a commit to scroll to and
    // mark (or null), `limit` how many commits. Core's own is a plain
    // list; the graph is oxplow-bundled's component.
    Replaceable {
        target: "vcs.history.graph",
        capability: "vcs",
        bound_to_provider: false,
        props: &["head_sha", "reveal_sha", "limit"],
        label: "history graph",
    },
];

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
