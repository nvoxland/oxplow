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
use oxplow_domain::work_items::{CanonicalState, WorkItemRecord, WorkItemsFeatures};
use oxplow_domain::Actor;

use crate::commands::work_item::WorkItemUpdateInput;
use crate::work_items::{NewItem, WorkItems};

/// A failed check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub check: &'static str,
    pub message: String,
}

/// What the suite reads back from the host: the item's `v_work_item` row,
/// its open efforts and the events naming it.
#[async_trait]
pub trait WorkItemsProbe: Send + Sync {
    /// Settle the host's projections (the event pump) before a read.
    async fn settle(&self);
    async fn record(&self, item_ref: &str) -> Option<WorkItemRecord>;
    async fn open_efforts(&self, item_ref: &str) -> usize;
    /// The type of every logged event whose subject names `item_ref`.
    async fn event_types(&self, item_ref: &str) -> Vec<String>;
}

/// Run every check against `provider` (its id and declared features) as
/// `actor`, writing through `items`. `native` is the provider's own
/// fields for the items the suite files (oxplow: the actor's thread, so
/// `in_progress` claims it), `None` for none.
pub async fn suite(
    items: &WorkItems,
    provider: &str,
    features: WorkItemsFeatures,
    native: Option<serde_json::Value>,
    probe: &dyn WorkItemsProbe,
    actor: &Actor,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut fail = |check: &'static str, message: String| findings.push(Finding { check, message });
    let prefix = format!("work_item:{provider}:");
    let new = |title: &str, parent_ref: Option<String>| NewItem {
        provider: Some(provider.to_string()),
        title: title.into(),
        body: "made by the conformance suite".into(),
        parent_ref,
        native: native.clone(),
        ..NewItem::default()
    };
    let mut created: Vec<String> = Vec::new();

    // 1. A new item is a row in its canonical `todo` state.
    let item = match items.create(actor, new("conformance item", None)).await {
        Ok(r) => r,
        Err(e) => {
            fail("create", format!("create failed: {e}"));
            return findings;
        }
    };
    created.push(item.clone());
    if !item.starts_with(&prefix) {
        fail("create", format!("`{item}` isn't a `{prefix}…` ref"));
    }
    probe.settle().await;
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

    // 3. Every canonical state round-trips; entering in_progress opens
    //    exactly one effort iff the provider says it does. Moving again
    //    to the native state the row reports lands on the same state.
    for state in CanonicalState::ALL {
        if let Err(e) = items.transition(actor, &item, state, None).await {
            fail("transition", format!("to {}: {e}", state.as_str()));
            continue;
        }
        probe.settle().await;
        let row = probe.record(&item).await;
        let got = row.as_ref().map(|r| r.state);
        if got != Some(state) {
            fail(
                "transition",
                format!("after moving to {}, the row says {got:?}", state.as_str()),
            );
        }
        if state == CanonicalState::InProgress {
            let open = probe.open_efforts(&item).await;
            let want = usize::from(features.in_progress_opens_effort);
            if open != want {
                fail(
                    "in_progress_opens_effort",
                    format!("{open} open efforts after in_progress, want {want}"),
                );
            }
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
    let child = items
        .create(actor, new("conformance child", Some(item.clone())))
        .await;
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
    let other = items
        .create(actor, new("conformance other", None))
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

    // 7. Delete follows its feature, and cleans up what the suite made
    //    when the provider can (a person confirms it).
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
            probe.settle().await;
            if probe.record(r).await.is_some() {
                fail("delete", format!("`{r}` is still a live row after delete"));
            }
        }
    }
    findings
}

fn sql(e: rusqlite::Error) -> oxplow_domain::DomainError {
    oxplow_domain::DomainError::Storage(e.to_string())
}

/// [`WorkItemsProbe`] over the app's database.
pub struct ServicesProbe<'a>(pub &'a crate::Services);

#[async_trait]
impl WorkItemsProbe for ServicesProbe<'_> {
    async fn settle(&self) {
        self.0.tasks.settle_lifecycle().await;
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
                    })
                },
            )
    }

    async fn open_efforts(&self, item_ref: &str) -> usize {
        let item_ref = item_ref.to_string();
        self.0
            .db
            .read(move |c| {
                c.query_row(
                    "SELECT count(*) FROM effort WHERE work_item = ?1 AND ended_at IS NULL",
                    [item_ref],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(sql)
            })
            .await
            .map(|n| n as usize)
            .unwrap_or(0)
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
            Some(serde_json::json!({ "thread": fx.thread.to_string() })),
            &ServicesProbe(&fx.svc),
            &actor,
        )
        .await;
        assert_eq!(findings, vec![]);
    }
}
