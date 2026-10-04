//! The core sub-components an extension may replace (`ui.replacements`,
//! experimental; `.context/target-architecture.md` §11.2): each a named
//! target of one capability with a **props contract** — the params its
//! replacement lens gets, and must declare, instead of any host state.
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
    /// The capability whose provider's extension may replace it: the
    /// provider of the thing the component shows when it shows one (a
    /// work item's), else the capability's **active** provider.
    pub capability: &'static str,
    /// What the replacement is given, by name; its lens declares each.
    pub props: &'static [&'static str],
}

/// Every replaceable sub-component.
pub const REPLACEABLE: &[Replaceable] = &[
    // The Board's columns of cards (`WorkBoard`): `scope` is `thread`,
    // `backlog` or `all`; `thread_id` the thread when `scope` is `thread`,
    // else null.
    Replaceable {
        target: "work_item.board",
        capability: "work_items",
        props: &["scope", "thread_id"],
    },
    // A work item's state control (`WorkItemPage`'s State and Move To):
    // `ref` is the item (P10). The item's own provider's extension
    // replaces it, whichever is active (tsk918).
    Replaceable {
        target: "work_item.detail.state",
        capability: "work_items",
        props: &["ref"],
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
