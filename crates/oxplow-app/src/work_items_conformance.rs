//! The work-items conformance suite (P5.C2, `.context/work-items.md`):
//! what every [`WorkItemsProvider`] must do, as plain functions over the
//! trait and a [`WorkItemsProbe`] that reads what the host recorded. It
//! runs in-tree against oxplow's own provider, and against an external
//! provider through the host (P5.D3) and the conformance kit (P5.D5).
//!
//! Each check is a [`Finding`] when it fails; an empty list passes.

use async_trait::async_trait;
use oxplow_domain::work_items::{
    CanonicalState, NewWorkItem, Transition, WorkItemRecord, WorkItemsProvider,
};
use oxplow_domain::Actor;

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

/// Run every check against `provider` as `actor`.
pub async fn suite(
    provider: &dyn WorkItemsProvider,
    probe: &dyn WorkItemsProbe,
    actor: &Actor,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut fail = |check: &'static str, message: String| findings.push(Finding { check, message });
    let features = provider.features();
    let prefix = format!("work_item:{}:", provider.provider());

    // 1. A new item is a row in its canonical `todo` state.
    let item = match provider
        .create(
            actor,
            NewWorkItem {
                title: "conformance item".into(),
                body: "made by the conformance suite".into(),
                parent_ref: None,
            },
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            fail("create", format!("create failed: {e}"));
            return findings;
        }
    };
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

    // 2. Every canonical state round-trips; entering in_progress opens
    //    exactly one effort iff the provider says it does.
    for state in CanonicalState::ALL {
        if let Err(e) = provider
            .transition(actor, &item, Transition::Canonical(state))
            .await
        {
            fail("transition", format!("to {}: {e}", state.as_str()));
            continue;
        }
        probe.settle().await;
        let got = probe.record(&item).await.map(|r| r.state);
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
    }

    // 3. A parent resolves with `hierarchy`, and is refused without it.
    let child = provider
        .create(
            actor,
            NewWorkItem {
                title: "conformance child".into(),
                body: String::new(),
                parent_ref: Some(item.clone()),
            },
        )
        .await;
    match (features.hierarchy, child) {
        (true, Ok(child)) => {
            probe.settle().await;
            let parent = probe.record(&child).await.and_then(|r| r.parent_ref);
            if parent.as_deref() != Some(item.as_str()) {
                fail("hierarchy", format!("child's parent_ref is {parent:?}"));
            }
        }
        (true, Err(e)) => fail("hierarchy", format!("a child was refused: {e}")),
        (false, Ok(child)) => fail(
            "hierarchy",
            format!("`{child}` took a parent though the provider has no hierarchy"),
        ),
        (false, Err(_)) => {}
    }

    // 4. Links and comments follow the features.
    let other = provider
        .create(
            actor,
            NewWorkItem {
                title: "conformance other".into(),
                ..NewWorkItem::default()
            },
        )
        .await
        .unwrap_or_default();
    let linked = provider.link(actor, &item, &other, "relates_to").await;
    if linked.is_ok() != features.links {
        fail(
            "links",
            format!("link: {linked:?} with links = {}", features.links),
        );
    }
    let commented = provider
        .comment(actor, &item, "a conformance comment")
        .await;
    if commented.is_ok() != features.comments {
        fail(
            "comments",
            format!(
                "comment: {commented:?} with comments = {}",
                features.comments
            ),
        );
    }

    // 5. A ref of another provider is refused, naming this one.
    let foreign = "work_item:not-a-provider:1";
    match provider
        .transition(actor, foreign, Transition::Canonical(CanonicalState::Done))
        .await
    {
        Ok(()) => fail("foreign_ref", format!("`{foreign}` was accepted")),
        Err(e) if !e.to_string().contains(provider.provider()) => fail(
            "foreign_ref",
            format!("the refusal doesn't name `{}`: {e}", provider.provider()),
        ),
        Err(_) => {}
    }

    // 6. What happened is in the log, naming the item.
    probe.settle().await;
    let types = probe.event_types(&item).await;
    for want in ["work_item.created", "work_item.transitioned"] {
        if !types.iter().any(|t| t == want) {
            fail(
                "events",
                format!("no `{want}` event names `{item}`: {types:?}"),
            );
        }
    }
    if features.links && !types.iter().any(|t| t == "work_item.linked") {
        fail("events", format!("no `work_item.linked` names `{item}`"));
    }
    if features.comments && !types.iter().any(|t| t == "work_item.commented") {
        fail("events", format!("no `work_item.commented` names `{item}`"));
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
        let findings = suite(&*provider, &ServicesProbe(&fx.svc), &actor).await;
        assert_eq!(findings, vec![]);
    }
}
