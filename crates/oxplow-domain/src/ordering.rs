//! What runs after what when one event reaches several contributions
//! (`.context/extensions.md` "Ordering"): the `after:` of an effect or a
//! collector, resolved to refs and compiled into one order per event.
//!
//! Three things look alike and aren't:
//! - **ordering** (`after:`) says that on one event, this contribution
//!   runs once those have — a scheduling constraint, nothing more;
//! - **dependency** (`imports:`) says this extension reads another's
//!   contract, and holds it back when that contract is missing;
//! - **causality** (an event's `cause`) says which event's handling
//!   logged this one.
//!
//! An `after:` target is one of three refs: `effect:<ext>/<id>`,
//! `collector:<ext>/<id>` — any enabled extension's — or
//! `consumer:<name>`, one of core's own event consumers
//! (`change.analyze`, `collection`, …). A bare name in a manifest is this
//! extension's own effect or collector of that id, else a core consumer.
//! A target nothing declares, or a cycle, is a load error: an order
//! nobody can satisfy is a mistake, not a preference.

use std::collections::{BTreeMap, BTreeSet};

use crate::contribution::{ContributionKind, ContributionRef};

/// The ref kind of a core event consumer: `consumer:change.analyze`.
pub const CONSUMER: &str = "consumer";

/// A resolved ordering target.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum After {
    /// An effect or collector, any extension's.
    Contribution(ContributionRef),
    /// One of core's event consumers, by name.
    Consumer(String),
}

impl After {
    /// The ref as text: `effect:acme/notify`, `consumer:change.analyze`.
    pub fn to_ref(&self) -> String {
        match self {
            After::Contribution(r) => r.to_string(),
            After::Consumer(name) => crate::refs::build::consumer_ref(name),
        }
    }

    /// Parse a ref: only the three kinds an `after:` may name.
    pub fn parse(text: &str) -> Result<After, String> {
        let parsed = crate::refs::grammar::CanonicalRef::parse(text)
            .map_err(|e| format!("`{text}` isn't a ref: {e}"))?;
        if parsed.kind == CONSUMER {
            return Ok(After::Consumer(parsed.id));
        }
        let r: ContributionRef = text.parse()?;
        match r.kind() {
            ContributionKind::Effect | ContributionKind::Collector => Ok(After::Contribution(r)),
            other => Err(format!(
                "`{text}` is {} `{}`; `after` names an effect, a collector or a core consumer",
                article(other.spec().title),
                other.spec().title.to_lowercase()
            )),
        }
    }
}

fn article(word: &str) -> &'static str {
    match word.chars().next().map(|c| c.to_ascii_lowercase()) {
        Some('a' | 'e' | 'i' | 'o' | 'u') => "an",
        _ => "a",
    }
}

/// Resolve one `after:` entry as `extension` wrote it: a ref as is; a
/// bare name as this extension's own effect or collector of that id
/// (`own` says which it declares), else a core consumer (`consumers`).
pub fn resolve(
    extension: &str,
    entry: &str,
    own: &dyn Fn(ContributionKind, &str) -> bool,
    consumers: &[&str],
) -> Result<After, String> {
    if entry.contains(':') {
        return After::parse(entry);
    }
    for kind in [ContributionKind::Effect, ContributionKind::Collector] {
        if own(kind, entry) {
            let r: ContributionRef = format!("{}:{extension}/{entry}", kind.segment()).parse()?;
            return Ok(After::Contribution(r));
        }
    }
    if consumers.contains(&entry) {
        return Ok(After::Consumer(entry.to_string()));
    }
    Err(format!(
        "`{entry}` is none of this extension's effects or collectors and no core consumer; \
         name another extension's by its ref (`effect:<extension>/<id>`)"
    ))
}

/// One node of the order: a contribution and what it runs after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub id: ContributionRef,
    pub after: Vec<After>,
}

/// Why an order can't be compiled.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OrderError {
    /// `by` runs after `target`, which no enabled extension declares and
    /// core doesn't have.
    #[error("{by} runs after {target}, which nothing declares")]
    Unknown { by: ContributionRef, target: String },
    /// The contributions wait on each other.
    #[error("these contributions wait on each other: {}", cycle.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" → "))]
    Cycle { cycle: Vec<ContributionRef> },
}

/// The compiled order: each contribution's level (0 runs first) and the
/// targets it waits on, consumers included.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Order {
    levels: BTreeMap<ContributionRef, usize>,
    after: BTreeMap<ContributionRef, Vec<After>>,
}

impl Order {
    /// Compile `nodes` against `consumers`, the core consumer names that
    /// exist. Every target must be one of the nodes or a consumer; the
    /// contribution edges must form no cycle. Errors are per declaring
    /// contribution, every one found, so an author fixes them in one pass.
    pub fn compile(nodes: &[Node], consumers: &[&str]) -> Result<Order, Vec<OrderError>> {
        let known: BTreeSet<&ContributionRef> = nodes.iter().map(|n| &n.id).collect();
        let mut errors = Vec::new();
        let mut edges: BTreeMap<&ContributionRef, Vec<&ContributionRef>> = BTreeMap::new();
        for node in nodes {
            let mut preds = Vec::new();
            for target in &node.after {
                match target {
                    After::Contribution(r) if known.contains(r) => preds.push(r),
                    After::Consumer(name) if consumers.contains(&name.as_str()) => {}
                    other => errors.push(OrderError::Unknown {
                        by: node.id.clone(),
                        target: other.to_ref(),
                    }),
                }
            }
            edges.insert(&node.id, preds);
        }
        // Kahn's algorithm: a level is everything whose predecessors are
        // all placed. What's left when nothing is ready is in a cycle.
        let mut level_of: BTreeMap<ContributionRef, usize> = BTreeMap::new();
        let mut remaining: BTreeSet<&ContributionRef> = known.clone();
        let mut level = 0;
        while !remaining.is_empty() {
            let ready: Vec<&ContributionRef> = remaining
                .iter()
                .copied()
                .filter(|id| edges[id].iter().all(|p| level_of.contains_key(*p)))
                .collect();
            if ready.is_empty() {
                for cycle in cycles(&remaining, &edges) {
                    errors.push(OrderError::Cycle { cycle });
                }
                break;
            }
            for id in ready {
                level_of.insert(id.clone(), level);
                remaining.remove(id);
            }
            level += 1;
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(Order {
            levels: level_of,
            after: nodes
                .iter()
                .map(|n| (n.id.clone(), n.after.clone()))
                .collect(),
        })
    }

    /// `ids`, in the order they run: by level, then by ref within a level.
    pub fn sorted<'a>(
        &self,
        ids: impl IntoIterator<Item = &'a ContributionRef>,
    ) -> Vec<ContributionRef> {
        let mut out: Vec<ContributionRef> = ids.into_iter().cloned().collect();
        out.sort_by_key(|id| {
            (
                self.levels.get(id).copied().unwrap_or(usize::MAX),
                id.clone(),
            )
        });
        out
    }

    pub fn level(&self, id: &ContributionRef) -> Option<usize> {
        self.levels.get(id).copied()
    }

    /// What `id` waits on.
    pub fn after(&self, id: &ContributionRef) -> &[After] {
        self.after.get(id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The core consumers any of `ids` waits on: what the pump gates the
    /// event on before these run at all.
    pub fn consumers<'a>(&self, ids: impl IntoIterator<Item = &'a ContributionRef>) -> Vec<String> {
        let mut out = Vec::new();
        for id in ids {
            for a in self.after(id) {
                if let After::Consumer(name) = a {
                    if !out.contains(name) {
                        out.push(name.clone());
                    }
                }
            }
        }
        out
    }

    /// Every contribution and its level, for inspection.
    pub fn rows(&self) -> impl Iterator<Item = (&ContributionRef, usize, &[After])> {
        self.levels
            .iter()
            .map(|(id, level)| (id, *level, self.after(id)))
    }
}

/// The elementary cycles among `remaining` (each reported once, from its
/// smallest member), found by walking each node's predecessors.
fn cycles(
    remaining: &BTreeSet<&ContributionRef>,
    edges: &BTreeMap<&ContributionRef, Vec<&ContributionRef>>,
) -> Vec<Vec<ContributionRef>> {
    let mut found: Vec<Vec<ContributionRef>> = Vec::new();
    let mut seen: BTreeSet<&ContributionRef> = BTreeSet::new();
    for start in remaining.iter().copied() {
        if seen.contains(start) {
            continue;
        }
        // Follow predecessors until a node repeats: the repeat closes a cycle.
        let mut path: Vec<&ContributionRef> = vec![start];
        let mut at = start;
        while let Some(next) = edges[at].iter().copied().find(|p| remaining.contains(p)) {
            if let Some(pos) = path.iter().position(|p| *p == next) {
                let mut cycle: Vec<ContributionRef> =
                    path[pos..].iter().map(|p| (*p).clone()).collect();
                // Report from its smallest member so a cycle reads the same
                // whichever node it was found from.
                let min = cycle
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, c)| (*c).clone())
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                cycle.rotate_left(min);
                for member in &path[pos..] {
                    seen.insert(member);
                }
                if !found.contains(&cycle) {
                    found.push(cycle);
                }
                break;
            }
            path.push(next);
            at = next;
        }
        for member in path {
            seen.insert(member);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(text: &str) -> ContributionRef {
        text.parse().unwrap()
    }

    fn node(id: &str, after: &[&str]) -> Node {
        Node {
            id: r(id),
            after: after.iter().map(|a| After::parse(a).unwrap()).collect(),
        }
    }

    #[test]
    fn a_bare_name_is_this_extensions_own_then_a_core_consumer() {
        let own = |kind: ContributionKind, id: &str| {
            matches!(
                (kind, id),
                (ContributionKind::Effect, "notify") | (ContributionKind::Collector, "sync")
            )
        };
        let consumers = ["change.analyze"];
        assert_eq!(
            resolve("acme", "notify", &own, &consumers)
                .unwrap()
                .to_ref(),
            "effect:acme/notify"
        );
        assert_eq!(
            resolve("acme", "sync", &own, &consumers).unwrap().to_ref(),
            "collector:acme/sync"
        );
        assert_eq!(
            resolve("acme", "change.analyze", &own, &consumers).unwrap(),
            After::Consumer("change.analyze".into())
        );
        assert_eq!(
            resolve("acme", "effect:other/first", &own, &consumers)
                .unwrap()
                .to_ref(),
            "effect:other/first"
        );
        let err = resolve("acme", "nope", &own, &consumers).unwrap_err();
        assert!(err.contains("none of this extension's"), "{err}");
        let err = resolve("acme", "job:acme/review", &own, &consumers).unwrap_err();
        assert!(err.contains("`after` names an effect"), "{err}");
    }

    #[test]
    fn contributions_run_in_levels_after_what_they_name() {
        let order = Order::compile(
            &[
                node(
                    "effect:acme/third",
                    &["effect:acme/second", "consumer:change.analyze"],
                ),
                node("effect:acme/second", &["collector:other/first"]),
                node("collector:other/first", &[]),
                node("effect:acme/alone", &[]),
            ],
            &["change.analyze"],
        )
        .unwrap();
        assert_eq!(order.level(&r("collector:other/first")), Some(0));
        assert_eq!(order.level(&r("effect:acme/alone")), Some(0));
        assert_eq!(order.level(&r("effect:acme/second")), Some(1));
        assert_eq!(order.level(&r("effect:acme/third")), Some(2));
        let ids = [
            r("effect:acme/third"),
            r("effect:acme/alone"),
            r("effect:acme/second"),
        ];
        assert_eq!(
            order.sorted(&ids),
            vec![
                r("effect:acme/alone"),
                r("effect:acme/second"),
                r("effect:acme/third")
            ]
        );
        assert_eq!(order.consumers(&ids), vec!["change.analyze".to_string()]);
        assert_eq!(order.consumers(&ids[1..2]), Vec::<String>::new());
    }

    #[test]
    fn an_unknown_target_is_an_error_naming_who_waits() {
        let errors = Order::compile(
            &[node(
                "effect:acme/a",
                &["effect:acme/gone", "consumer:nope"],
            )],
            &["change.analyze"],
        )
        .unwrap_err();
        assert_eq!(
            errors,
            vec![
                OrderError::Unknown {
                    by: r("effect:acme/a"),
                    target: "effect:acme/gone".into()
                },
                OrderError::Unknown {
                    by: r("effect:acme/a"),
                    target: "consumer:nope".into()
                },
            ]
        );
        assert_eq!(
            errors[0].to_string(),
            "effect:acme/a runs after effect:acme/gone, which nothing declares"
        );
    }

    #[test]
    fn a_cycle_is_an_error_naming_its_members_once() {
        // A two-node cycle, a three-node one and a self-loop, beside a
        // node that's fine.
        let errors = Order::compile(
            &[
                node("effect:a/x", &["effect:a/y"]),
                node("effect:a/y", &["effect:a/x"]),
                node("effect:b/p", &["effect:b/r"]),
                node("effect:b/q", &["effect:b/p"]),
                node("effect:b/r", &["effect:b/q"]),
                node("effect:c/me", &["effect:c/me"]),
                node("effect:c/fine", &[]),
            ],
            &[],
        )
        .unwrap_err();
        let cycles: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
        assert_eq!(
            cycles,
            vec![
                "these contributions wait on each other: effect:a/x → effect:a/y",
                "these contributions wait on each other: effect:b/p → effect:b/r → effect:b/q",
                "these contributions wait on each other: effect:c/me",
            ]
        );
    }

    #[test]
    fn a_node_downstream_of_a_cycle_is_not_itself_reported() {
        let errors = Order::compile(
            &[
                node("effect:a/x", &["effect:a/y"]),
                node("effect:a/y", &["effect:a/x"]),
                node("effect:a/after", &["effect:a/x"]),
            ],
            &[],
        )
        .unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(matches!(&errors[0], OrderError::Cycle { cycle } if cycle.len() == 2));
    }
}
