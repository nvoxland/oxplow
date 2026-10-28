//! The core sub-components an extension may replace (`ui.replacements`;
//! `.context/extensions.md` "Replacements"): each a named target of one
//! capability with a **versioned props contract** — the params its
//! replacement page gets, and must declare, instead of any host state.
//! (The viewer's stream isn't a prop: any page declaring `stream_id` gets
//! it, as everywhere.)
//! Whole pages are never replaceable.
//!
//! One table — `data/replaceable.json`, data not code — for the extension loader (which checks a declaration against
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
    /// What the replacement is given, by name; its page declares each.
    pub props: &'static [&'static str],
    /// Its host page's view params it shares: the replacement reads them
    /// (`init.view`), writes them (`setViewParams`) and hears them change,
    /// as the core component it replaces does. None: it keeps its own.
    pub view: &'static [&'static str],
    /// What a person calls it ("board"): the one name the UI shows it by.
    pub label: &'static str,
    /// What it is, for a person or an agent reading the table.
    pub about: &'static str,
}

#[derive(serde::Deserialize)]
struct Row {
    target: String,
    capability: String,
    bound_to_provider: bool,
    version: u32,
    props: Vec<String>,
    view: Vec<String>,
    label: String,
    #[serde(default)]
    about: String,
}

/// Leak a string for the life of the process: the table is read once.
fn keep(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn keep_all(v: Vec<String>) -> &'static [&'static str] {
    Box::leak(
        v.into_iter()
            .map(keep)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    )
}

/// Every replaceable sub-component, read from `data/replaceable.json`. The
/// list only shrinks (`fixtures/replaceable.json` pins its targets): a
/// target leaves it when a page stops being replaceable, and a new one is a
/// decision, not a line.
pub fn all() -> &'static [Replaceable] {
    static TABLE: std::sync::OnceLock<&'static [Replaceable]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let rows: Vec<Row> = serde_json::from_str(include_str!("../data/replaceable.json"))
            .expect("data/replaceable.json is the replaceable table");
        Box::leak(
            rows.into_iter()
                .map(|r| Replaceable {
                    target: keep(r.target),
                    capability: keep(r.capability),
                    bound_to_provider: r.bound_to_provider,
                    version: r.version,
                    props: keep_all(r.props),
                    view: keep_all(r.view),
                    label: keep(r.label),
                    about: keep(r.about),
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        )
    })
}

/// The `replacements:` value that keeps oxplow's own component.
pub const OXPLOWS_OWN: &str = "oxplow";

/// The replaceable sub-component named `target`.
pub fn replaceable(target: &str) -> Option<&'static Replaceable> {
    all().iter().find(|r| r.target == target)
}

/// Every target, for an error that lists them.
pub fn targets() -> String {
    all()
        .iter()
        .map(|r| r.target)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The replaceable targets only shrink: a new one is a decision (a page
    /// that may be replaced), not a line in the data. A target missing from
    /// the golden is new; a golden name missing from the table left — take
    /// it out of the golden too.
    #[test]
    fn the_replaceable_targets_only_shrink() {
        let golden: Vec<String> =
            serde_json::from_str(include_str!("../fixtures/replaceable.json"))
                .expect("the golden is a JSON list");
        let added: Vec<&str> = all()
            .iter()
            .map(|r| r.target)
            .filter(|t| !golden.iter().any(|g| g == t))
            .collect();
        assert!(added.is_empty(), "new replaceable targets {added:?}");
        let left: Vec<&String> = golden
            .iter()
            .filter(|g| !all().iter().any(|r| r.target == g.as_str()))
            .collect();
        assert!(
            left.is_empty(),
            "left the table: {left:?} — remove it from the golden"
        );
        assert!(
            all().iter().all(|r| !r.about.is_empty()),
            "each says what it is"
        );
    }

    /// Every target is its own name and label, versioned from 1, with
    /// at least one prop; `stream_id` is never one.
    #[test]
    fn the_table_holds() {
        let mut targets: Vec<&str> = all().iter().map(|r| r.target).collect();
        let mut labels: Vec<&str> = all().iter().map(|r| r.label).collect();
        targets.sort_unstable();
        labels.sort_unstable();
        targets.dedup();
        labels.dedup();
        assert_eq!(targets.len(), all().len());
        assert_eq!(labels.len(), all().len());
        for r in all() {
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
        for r in all() {
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
