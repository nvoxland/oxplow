//! The work-items conformance suite (P5.C2, P7.A1; `.context/work-items.md`):
//! what every provider must do, as plain functions over the `work_item.*`
//! commands (the [`WorkItems`] client — the one write surface, so the
//! suite exercises exactly what a person and an agent run) and a
//! [`WorkItemsProbe`] that reads what the host recorded. It runs in-tree
//! against oxplow's own provider, and against an external provider
//! through the host (P5.D3) and the conformance kit (P5.D5).
//!
//! Each check is a [`Finding`] when it fails; an empty list passes.

use async_trait::async_trait;
use std::sync::Arc;

use oxplow_domain::work_items::{CanonicalState, ExternalVerbs, WorkItemRecord, WorkItemsFeatures};
use oxplow_domain::Actor;

use crate::commands::work_item::WorkItemUpdateInput;
use crate::work_items::{NewItem, WorkItems};

/// A failed check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub check: &'static str,
    pub message: String,
}

/// What a run of the suite came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiteRun {
    /// The failed checks; none passes.
    pub findings: Vec<Finding>,
    /// The items it filed and didn't delete (a provider without `delete`,
    /// or a run stopped by a failed delete): what is left in the
    /// provider's own system for a person to clean up.
    pub left: Vec<String>,
}

/// What the suite reads back from the host: the item's `v_work_item` row
/// and the events naming it.
#[async_trait]
pub trait WorkItemsProbe: Send + Sync {
    /// Settle the host's projections (the event pump) before a read.
    async fn settle(&self);
    async fn record(&self, item_ref: &str) -> Option<WorkItemRecord>;
    /// The type of every logged event whose subject names `item_ref`.
    async fn event_types(&self, item_ref: &str) -> Vec<String>;
    /// Read `provider` back through its collectors (`provider.sync`), its
    /// records restating its items; `false` when it has none to read
    /// (oxplow's own, a provider that declares no collector).
    async fn sync(&self, provider: &str) -> Result<bool, String>;
    /// `provider`'s verbs as the host calls them (an external provider's),
    /// for what the bus never sends itself: a write re-sent with its
    /// idempotency key.
    async fn verbs(&self, provider: &str) -> Option<Arc<dyn ExternalVerbs>>;
    /// The refs of `provider`'s live rows titled `title`.
    async fn titled(&self, provider: &str, title: &str) -> Vec<String>;
    /// The host's active work-items provider: where every create files.
    async fn active(&self) -> String;
    /// `item_ref`'s links as the interface reads them (`v_work_item_link`):
    /// `(target, link_type)`.
    async fn links(&self, item_ref: &str) -> Vec<(String, String)>;
    /// `item_ref`'s comment bodies as the interface reads them.
    async fn comments(&self, item_ref: &str) -> Vec<String>;
    /// `item_ref`'s list and rank as the interface reads them
    /// (`v_work_item.thread_id`, `.rank`).
    async fn placement(&self, item_ref: &str) -> (Option<i64>, Option<f64>);
}

/// Run every check against `provider` (its id and declared features) as
/// `actor`, writing through `items`, on the host's active tracker (the
/// suite checks it's `provider`). `native` is the provider's own fields
/// for the items it files, `None` for none; an agent `actor` files them on
/// its thread, so `in_progress` claims it on oxplow.
/// A create that must keep the item (the suite checks a list that keeps
/// its items; none is a sink, checked on its own): its ref.
async fn filed(
    items: &crate::work_items::WorkItems,
    actor: &Actor,
    item: crate::work_items::NewItem,
) -> Result<String, oxplow_domain::CommandError> {
    items
        .create(actor, item)
        .await?
        .ok_or_else(|| oxplow_domain::CommandError::Failed {
            message: "the create kept nothing (no ref)".into(),
        })
}

/// How many `work_item.state_changed` events name `item`.
async fn state_changes(probe: &dyn WorkItemsProbe, item: &str) -> usize {
    probe
        .event_types(item)
        .await
        .iter()
        .filter(|t| *t == "work_item.state_changed")
        .count()
}

pub async fn suite(
    items: &WorkItems,
    provider: &str,
    features: WorkItemsFeatures,
    native: Option<serde_json::Value>,
    probe: &dyn WorkItemsProbe,
    actor: &Actor,
) -> SuiteRun {
    let mut findings = Vec::new();
    let mut fail = |check: &'static str, message: String| findings.push(Finding { check, message });
    let prefix = format!("work_item:{provider}:");
    // Every create files on the active tracker (tsk1058): the host runs the
    // suite with `provider` active.
    let active = probe.active().await;
    if active != provider {
        fail(
            "create",
            format!(
                "`{provider}` isn't the active work-items provider (`{active}` is): the suite \
                 files on it"
            ),
        );
        return SuiteRun {
            findings,
            left: Vec::new(),
        };
    }
    let new = |title: &str, parent_ref: Option<String>| NewItem {
        title: title.into(),
        body: "made by the conformance suite".into(),
        parent_ref,
        native: native.clone(),
        ..NewItem::default()
    };
    let mut created: Vec<String> = Vec::new();

    // 1. A new item is a row in its canonical `todo` state.
    let item = match filed(items, actor, new("conformance item", None)).await {
        Ok(r) => r,
        Err(e) => {
            fail("create", format!("create failed: {e}"));
            return SuiteRun {
                findings,
                left: Vec::new(),
            };
        }
    };
    created.push(item.clone());
    if !item.starts_with(&prefix) {
        fail("create", format!("`{item}` isn't a `{prefix}…` ref"));
    }
    probe.settle().await;
    if state_changes(probe, &item).await == 0 {
        fail(
            "state_changed",
            format!("creating `{item}` logged no work_item.state_changed"),
        );
    }
    match probe.record(&item).await {
        None => fail("create", format!("no v_work_item row for `{item}`")),
        Some(r) if r.title != "conformance item" || r.state != CanonicalState::Todo => fail(
            "create",
            format!(
                "row is {:?} / {:?}, want the title and `todo`",
                r.title, r.state
            ),
        ),
        Some(_) => {}
    }

    // 2. An update changes the fields it names, and only those.
    match items
        .update(
            actor,
            WorkItemUpdateInput {
                item_ref: item.clone(),
                title: Some("conformance item, renamed".into()),
                ..WorkItemUpdateInput::default()
            },
        )
        .await
    {
        Err(e) => fail("update", format!("update failed: {e}")),
        Ok(_) => {
            probe.settle().await;
            match probe.record(&item).await {
                Some(r)
                    if r.title == "conformance item, renamed"
                        && r.body == "made by the conformance suite" => {}
                other => fail("update", format!("after renaming, the row is {other:?}")),
            }
        }
    }

    // 3. Every canonical state round-trips, and core logs each move as
    //    `work_item.state_changed`, whoever the provider. Moving again to
    //    the native state the row reports lands on the same state.
    let mut was = CanonicalState::Todo;
    for state in CanonicalState::ALL {
        let changes_before = state_changes(probe, &item).await;
        if let Err(e) = items.transition(actor, &item, state, None).await {
            fail("transition", format!("to {}: {e}", state.as_str()));
            continue;
        }
        probe.settle().await;
        if state != was && state_changes(probe, &item).await <= changes_before {
            fail(
                "state_changed",
                format!(
                    "moving from {} to {} logged no work_item.state_changed",
                    was.as_str(),
                    state.as_str()
                ),
            );
        }
        was = state;
        let row = probe.record(&item).await;
        let got = row.as_ref().map(|r| r.state);
        if got != Some(state) {
            fail(
                "transition",
                format!("after moving to {}, the row says {got:?}", state.as_str()),
            );
        }
        if let Some(native) = row.map(|r| r.native_state) {
            match items.transition(actor, &item, state, Some(&native)).await {
                Err(e) => fail(
                    "native_state",
                    format!(
                        "moving to {} with its own native state `{native}` failed: {e}",
                        state.as_str()
                    ),
                ),
                Ok(_) => {
                    probe.settle().await;
                    let again = probe.record(&item).await;
                    if again.as_ref().map(|r| (r.state, r.native_state.as_str()))
                        != Some((state, native.as_str()))
                    {
                        fail(
                            "native_state",
                            format!(
                                "after moving to {} / `{native}`, the row is {again:?}",
                                state.as_str()
                            ),
                        );
                    }
                }
            }
        }
    }

    // 4. A parent resolves with `hierarchy`, and is refused without it.
    let child = filed(items, actor, new("conformance child", Some(item.clone()))).await;
    match (features.hierarchy, child) {
        (true, Ok(child)) => {
            created.push(child.clone());
            probe.settle().await;
            let parent = probe.record(&child).await.and_then(|r| r.parent_ref);
            if parent.as_deref() != Some(item.as_str()) {
                fail("hierarchy", format!("child's parent_ref is {parent:?}"));
            }
        }
        (true, Err(e)) => fail("hierarchy", format!("a child was refused: {e}")),
        (false, Ok(child)) => {
            created.push(child.clone());
            fail(
                "hierarchy",
                format!("`{child}` took a parent though the provider has no hierarchy"),
            )
        }
        (false, Err(_)) => {}
    }

    // 5. Links and comments follow the features.
    let other = filed(items, actor, new("conformance other", None))
        .await
        .unwrap_or_default();
    if !other.is_empty() {
        created.push(other.clone());
    }
    let linked = items.link(actor, &item, &other, "relates_to").await;
    if linked.is_ok() != features.links {
        fail(
            "links",
            format!("link: {linked:?} with links = {}", features.links),
        );
    }
    let commented = items.comment(actor, &item, "a conformance comment").await;
    if commented.is_ok() != features.comments {
        fail(
            "comments",
            format!(
                "comment: {commented:?} with comments = {}",
                features.comments
            ),
        );
    }
    // …and read back through the interface.
    probe.settle().await;
    if features.links && linked.is_ok() {
        let links = probe.links(&item).await;
        if !links.contains(&(other.clone(), "relates_to".into())) {
            fail(
                "links",
                format!("the link isn't in v_work_item_link: {links:?}"),
            );
        }
    }
    if features.comments && commented.is_ok() {
        let comments = probe.comments(&item).await;
        if !comments.iter().any(|c| c == "a conformance comment") {
            fail(
                "comments",
                format!("the comment isn't in v_work_item_comment: {comments:?}"),
            );
        }
    }

    // 5b. Ordering and lists follow the features, and read back.
    if !other.is_empty() {
        let reordered = items.reorder_before(actor, &other, &item).await;
        if reordered.is_ok() != features.ordering {
            fail(
                "ordering",
                format!(
                    "reorder: {reordered:?} with ordering = {}",
                    features.ordering
                ),
            );
        }
        if features.ordering && reordered.is_ok() {
            probe.settle().await;
            let (_, before) = probe.placement(&other).await;
            let (_, after) = probe.placement(&item).await;
            if !matches!((before, after), (Some(b), Some(a)) if b < a) {
                fail(
                    "ordering",
                    format!("reordered before it, `{other}` ranks {before:?} against {after:?}"),
                );
            }
        }
        let moved = items.move_to_backlog(actor, &other).await;
        if moved.is_ok() != features.lists {
            fail(
                "lists",
                format!("move: {moved:?} with lists = {}", features.lists),
            );
        }
        if features.lists && moved.is_ok() {
            probe.settle().await;
            if let (Some(thread), _) = probe.placement(&other).await {
                fail(
                    "lists",
                    format!("moved to the backlog, it's on thread {thread}"),
                );
            }
        }
    }

    // 6. What happened is in the log, naming the item: every write that
    //    changed it logged an event about it (the provider's own kinds —
    //    oxplow's `work_item.created` / `edited` / `transitioned`, an
    //    external provider's `work_item.recorded`; a move to the state it
    //    is in may log nothing).
    probe.settle().await;
    let logged = probe.event_types(&item).await.len();
    let writes = 2
        + CanonicalState::ALL.len()
        + usize::from(features.links)
        + usize::from(features.comments);
    if logged < writes {
        fail(
            "events",
            format!("{logged} events name `{item}` after {writes} writes to it"),
        );
    }

    // 7. Reading the provider back restates what its writes recorded:
    //    after a sync, every item is the row it was.
    let written: Vec<(String, Option<WorkItemRecord>)> = {
        let mut rows = Vec::new();
        for r in &created {
            rows.push((r.clone(), probe.record(r).await));
        }
        rows
    };
    match probe.sync(provider).await {
        Err(e) => fail("sync", format!("reading it back failed: {e}")),
        Ok(false) => {}
        Ok(true) => {
            probe.settle().await;
            for (r, before) in &written {
                let after = probe.record(r).await;
                if &after != before {
                    fail(
                        "sync",
                        format!("reading `{r}` back changed its row from {before:?} to {after:?}"),
                    );
                }
            }
        }
    }

    // 8. A provider that declares `idempotent_writes` keeps it (P10): a
    //    create sent twice with one key is one item, answered alike —
    //    after a read back too — and another key is another item.
    if features.idempotent_writes {
        match probe.verbs(provider).await {
            None => fail(
                "idempotent_writes",
                "declared, but the host has no verbs to send a key to".into(),
            ),
            Some(verbs) => {
                // Its own title, so a provider's leftovers from another run
                // (one without `delete`, a run that failed) don't count.
                let title = format!(
                    "conformance keyed item {}",
                    &uuid::Uuid::new_v4().simple().to_string()[..8]
                );
                let mut input = serde_json::json!({
                    "title": title,
                    "body": "made by the conformance suite",
                });
                if let Some(native) = &native {
                    input["native"] = native.clone();
                }
                let key = format!("conformance:{}", uuid::Uuid::new_v4().simple());
                let other_key = format!("conformance:{}", uuid::Uuid::new_v4().simple());
                let first = verbs
                    .invoke(actor, "create", input.clone(), Some(key.clone()))
                    .await;
                let again = verbs
                    .invoke(actor, "create", input.clone(), Some(key.clone()))
                    .await;
                let other = verbs
                    .invoke(actor, "create", input.clone(), Some(other_key))
                    .await;
                // Across a restart of its process: a provider that keeps its
                // keys only in memory would make it again (tsk916).
                verbs.restart().await;
                let restarted = verbs.invoke(actor, "create", input, Some(key)).await;
                let item_of = |r: &serde_json::Value| r["ref"].as_str().map(str::to_string);
                // Whatever was made is cleaned up below, however it went.
                for r in [&first, &again, &other, &restarted]
                    .into_iter()
                    .filter_map(|o| o.as_ref().ok().and_then(|o| item_of(&o.result)))
                {
                    if !created.contains(&r) {
                        created.push(r);
                    }
                }
                match (first, again, other, restarted) {
                    (Ok(first), Ok(again), Ok(other), Ok(restarted)) => {
                        if first.result != again.result {
                            fail(
                                "idempotent_writes",
                                format!(
                                    "one key sent twice answered {} then {}",
                                    first.result, again.result
                                ),
                            );
                        }
                        if first.result != restarted.result {
                            fail(
                                "idempotent_writes",
                                format!(
                                    "one key sent again after a restart answered {} then {}",
                                    first.result, restarted.result
                                ),
                            );
                        }
                        if item_of(&first.result) == item_of(&other.result) {
                            fail("idempotent_writes", "another key gave the same item".into());
                        }
                        if let Ok(true) = probe.sync(provider).await {
                            probe.settle().await;
                            let rows = probe.titled(provider, &title).await;
                            if rows.len() != 2 {
                                fail(
                                    "idempotent_writes",
                                    format!(
                                        "after two keys (one sent three times) it has {} items: \
                                         {rows:?}",
                                        rows.len()
                                    ),
                                );
                            }
                        }
                    }
                    (first, again, other, restarted) => fail(
                        "idempotent_writes",
                        format!(
                            "a keyed create failed: {first:?} / {again:?} / {other:?} / \
                             {restarted:?}"
                        ),
                    ),
                }
            }
        }
    }

    // 9. Delete follows its feature, and cleans up what the suite made
    //    when the provider can (a person confirms it).
    let mut deleted_refs: Vec<String> = Vec::new();
    for r in created.iter().rev() {
        let deleted = items.delete(&Actor::Human, r, true).await;
        if deleted.is_ok() != features.delete {
            fail(
                "delete",
                format!(
                    "delete `{r}`: {deleted:?} with delete = {}",
                    features.delete
                ),
            );
            break;
        }
        if deleted.is_ok() {
            deleted_refs.push(r.clone());
            probe.settle().await;
            if probe.record(r).await.is_some() {
                fail("delete", format!("`{r}` is still a live row after delete"));
            }
        }
    }
    SuiteRun {
        findings,
        left: created
            .into_iter()
            .filter(|r| !deleted_refs.contains(r))
            .collect(),
    }
}

fn sql(e: rusqlite::Error) -> oxplow_domain::DomainError {
    oxplow_domain::DomainError::Storage(e.to_string())
}

/// [`WorkItemsProbe`] over the app's database.
pub struct ServicesProbe<'a>(pub &'a crate::Services);

#[async_trait]
impl WorkItemsProbe for ServicesProbe<'_> {
    async fn settle(&self) {
        self.0.efforts.settle_lifecycle().await;
        // The in-transaction consumers (the `work_items.project`
        // projection) run on the pump's next pass; run it now.
        if let Err(e) = self.0.event_pump.run_once().await {
            tracing::warn!(error = %e, "conformance probe: pump run failed");
        }
    }

    async fn record(&self, item_ref: &str) -> Option<WorkItemRecord> {
        let item_ref = item_ref.to_string();
        self.0
            .db
            .read(move |c| {
                use rusqlite::OptionalExtension;
                let row: Option<(
                    String,
                    String,
                    String,
                    String,
                    String,
                    String,
                    Option<String>,
                )> = c
                    .query_row(
                        "SELECT ref, title, body, state, native_state, native, parent_ref
                     FROM work_item WHERE ref = ?1 AND deleted_at IS NULL",
                        [item_ref],
                        |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, String>(2)?,
                                r.get::<_, String>(3)?,
                                r.get::<_, String>(4)?,
                                r.get::<_, String>(5)?,
                                r.get::<_, Option<String>>(6)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(sql)?;
                Ok(row)
            })
            .await
            .ok()
            .flatten()
            .and_then(
                |(item_ref, title, body, state, native_state, native, parent_ref)| {
                    Some(WorkItemRecord {
                        item_ref,
                        title,
                        body,
                        state: serde_json::from_value(serde_json::Value::String(state)).ok()?,
                        native_state,
                        native: serde_json::from_str(&native).ok()?,
                        parent_ref,
                        deleted: false,
                        rank: None,
                        links: None,
                        comments: None,
                    })
                },
            )
    }

    async fn sync(&self, provider: &str) -> Result<bool, String> {
        let svc = self.0;
        let instance = svc
            .extension_catalog
            .get(&svc.layout.project_dir)
            .iter()
            .filter(|e| e.enabled)
            .find_map(|e| {
                e.providers
                    .iter()
                    .find(|s| s.id == provider)
                    .map(|s| s.approval_name(&e.name))
            });
        let Some(instance) = instance else {
            return Ok(false);
        };
        let reads = svc
            .providers
            .get(&instance)
            .await
            .is_some_and(|i| !i.declared.collectors.is_empty());
        if !reads {
            return Ok(false);
        }
        svc.commands
            .run(
                &Actor::Human,
                crate::providers::sync::SYNC,
                serde_json::json!({ "instance": instance }),
                true,
            )
            .await
            .map(|_| true)
            .map_err(|e| e.to_string())
    }

    async fn verbs(&self, provider: &str) -> Option<Arc<dyn ExternalVerbs>> {
        self.0.work_items.get(provider).ok()?.external
    }

    async fn active(&self) -> String {
        self.0.work_items.active()
    }

    async fn titled(&self, provider: &str, title: &str) -> Vec<String> {
        let (prefix, title) = (format!("work_item:{provider}:%"), title.to_string());
        self.0
            .db
            .read(move |c| {
                let mut stmt = c
                    .prepare("SELECT ref FROM v_work_item WHERE ref LIKE ?1 AND title = ?2")
                    .map_err(sql)?;
                let rows = stmt
                    .query_map([prefix, title], |r| r.get::<_, String>(0))
                    .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
                    .map_err(sql)?;
                Ok(rows)
            })
            .await
            .unwrap_or_default()
    }

    async fn links(&self, item_ref: &str) -> Vec<(String, String)> {
        let item_ref = item_ref.to_string();
        self.0
            .db
            .read(move |c| {
                let mut stmt = c
                    .prepare("SELECT to_ref, link_type FROM v_work_item_link WHERE from_ref = ?1")
                    .map_err(sql)?;
                let rows = stmt
                    .query_map([item_ref], |r| Ok((r.get(0)?, r.get(1)?)))
                    .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
                    .map_err(sql)?;
                Ok(rows)
            })
            .await
            .unwrap_or_default()
    }

    async fn comments(&self, item_ref: &str) -> Vec<String> {
        let item_ref = item_ref.to_string();
        self.0
            .db
            .read(move |c| {
                let mut stmt = c
                    .prepare("SELECT body FROM v_work_item_comment WHERE ref = ?1")
                    .map_err(sql)?;
                let rows = stmt
                    .query_map([item_ref], |r| r.get(0))
                    .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
                    .map_err(sql)?;
                Ok(rows)
            })
            .await
            .unwrap_or_default()
    }

    async fn placement(&self, item_ref: &str) -> (Option<i64>, Option<f64>) {
        let item_ref = item_ref.to_string();
        self.0
            .db
            .read(move |c| {
                use rusqlite::OptionalExtension;
                c.query_row(
                    "SELECT thread_id, rank FROM v_work_item WHERE ref = ?1",
                    [item_ref],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(sql)
            })
            .await
            .ok()
            .flatten()
            .unwrap_or((None, None))
    }

    async fn event_types(&self, item_ref: &str) -> Vec<String> {
        let item_ref = item_ref.to_string();
        self.0
            .db
            .read(move |c| {
                let mut stmt = c
                    .prepare(
                        "SELECT e.type FROM event_log e, json_each(e.subject) s
                         WHERE s.value = ?1 ORDER BY e.seq",
                    )
                    .map_err(sql)?;
                let rows = stmt
                    .query_map([item_ref], |r| r.get::<_, String>(0))
                    .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
                    .map_err(sql)?;
                Ok(rows)
            })
            .await
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::ThreadId;

    /// oxplow's own tasks pass the suite (P5.C2's red: the suite's first
    /// run).
    #[tokio::test]
    async fn oxplow_tasks_are_a_conforming_provider() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let provider = fx.svc.work_items.get("oxplow").unwrap();
        // The writer thread, so moving to in_progress may claim.
        let actor = Actor::Agent {
            thread_id: Some(ThreadId::new(fx.thread.value())),
            stream_id: None,
        };
        let items = WorkItems::new(fx.svc.commands.clone());
        let findings = suite(
            &items,
            &provider.id,
            provider.features,
            None,
            &ServicesProbe(&fx.svc),
            &actor,
        )
        .await;
        assert_eq!(findings.findings, vec![]);
        // oxplow's own deletes: the suite leaves nothing behind.
        assert_eq!(findings.left, Vec::<String>::new());
    }
}
