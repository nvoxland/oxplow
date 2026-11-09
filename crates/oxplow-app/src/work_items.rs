//! The work-items capability in the app (`.context/work-items.md`):
//!
//! - [`built_in_provider`], the factory for a built-in work list
//!   (`oxplow:tasks`, oxplow's own tasks, from `oxplow-tasks`), and
//!   [`prepare_built_ins`], which registers the ones the project's
//!   extensions declare;
//! - [`WorkItems`], a typed client over the `work_item.*` commands: the
//!   one write surface for every provider (the conformance suite and
//!   `task_writes` use it);
//! - [`WorkItemsProjection`], the pump consumer (`work_items.project`)
//!   that upserts every list's items into `work_item` from its
//!   `work_item.recorded` events, and keeps who each is assigned to
//!   (`work_item_assignment`) from core's `work_item.assigned`.

use std::sync::Arc;

use oxplow_domain::events::schema::{
    EventType, WorkItemAssigned, WorkItemAssignedV1, WorkItemRecorded, WorkItemRecordedV3,
};
use oxplow_domain::work_items::{
    provider_of, Bucket, WorkItemVerbs, WorkItemsFeatures, WorkItemsProvider,
};
use oxplow_domain::{Actor, CommandError, CommandOutcome, DomainError, StoredEvent};
use serde_json::{json, Value};

use crate::commands::{work_item, CommandBus};
use crate::event_pump::EventConsumer;

/// oxplow's own provider name: `oxplow:oxplow/work_item?id=tsk<n>&provider=oxplow`.
pub const PROVIDER: &str = oxplow_domain::work_items::OXPLOW;
/// oxplow's tasks' built-in entry (`capabilities::BUILT_INS`).
pub const BUILT_IN: &str = "oxplow:tasks";

/// A built-in work list's provider, by its entry (`capabilities::BUILT_INS`):
/// registered under the provider id its refs carry, with its features and
/// ids as the built-in declares them and its verbs its own. `None` for an
/// entry that isn't a work list.
pub fn built_in_provider(entry: &str, db: &oxplow_db::Database) -> Option<WorkItemsProvider> {
    let built_in = crate::capabilities::built_in(entry)?;
    if built_in.capability != "work_items" {
        return None;
    }
    let id = built_in.provider?;
    let verbs: Arc<dyn WorkItemVerbs> = match entry {
        BUILT_IN => Arc::new(oxplow_tasks::OxplowTasks::new(db.clone())),
        _ => return None,
    };
    let has = |f: &str| built_in.features.contains(&f);
    Some(WorkItemsProvider {
        id: id.into(),
        features: WorkItemsFeatures {
            hierarchy: has("hierarchy"),
            comments: has("comments"),
            links: has("links"),
            delete: has("delete"),
            idempotent_writes: has("idempotent_writes"),
        },
        verbs,
        id_pattern: built_in.id_pattern.map(str::to_string),
        sink: false,
        stages: crate::capabilities::built_in_stage_decls(built_in),
    })
}

/// Register the built-in work lists `declared` names (the project's
/// extensions' `implementations:`), and unregister the ones it no longer
/// does: a built-in list is reachable only while an extension declares
/// it; its data stays.
pub struct PreparedWorkItems {
    changes: Vec<PreparedWorkItemChange>,
}

enum PreparedWorkItemChange {
    Register(WorkItemsProvider),
    Remove(&'static str),
}

impl PreparedWorkItems {
    pub fn publish(self, registry: &oxplow_domain::work_items::WorkItemsRegistry) {
        for change in self.changes {
            match change {
                PreparedWorkItemChange::Register(provider) => registry.register(provider),
                PreparedWorkItemChange::Remove(id) => registry.unregister(id),
            }
        }
    }
}

pub fn prepare_built_ins(
    declared: &[crate::capabilities::Implementation],
    db: &oxplow_db::Database,
) -> PreparedWorkItems {
    let mut changes = Vec::new();
    for built in crate::capabilities::BUILT_INS
        .iter()
        .filter(|built| built.capability == "work_items")
    {
        let named = declared.iter().any(|implementation| {
            implementation.capability == "work_items"
                && matches!(implementation.source, crate::capabilities::Source::BuiltIn(entry) if entry == built.entry)
        });
        if named {
            if let Some(provider) = built_in_provider(built.entry, db) {
                changes.push(PreparedWorkItemChange::Register(provider));
            }
        } else if let Some(id) = built.provider {
            changes.push(PreparedWorkItemChange::Remove(id));
        }
    }
    PreparedWorkItems { changes }
}

/// None as a work list: a sink. Every verb succeeds with `{ tracked:
/// false }` and keeps nothing; it has every feature and takes any item's
/// ref and any id, so nothing that writes work items is refused while no
/// list is active, and the interface reads empty.
pub fn none_provider() -> WorkItemsProvider {
    WorkItemsProvider {
        id: oxplow_domain::capability::NONE.into(),
        features: WorkItemsFeatures {
            hierarchy: true,
            comments: true,
            links: true,
            delete: true,
            idempotent_writes: true,
        },
        verbs: Arc::new(Sink),
        id_pattern: Some(".+".into()),
        sink: true,
        stages: oxplow_domain::work_items::bucket_stages(),
    }
}

/// [`none_provider`]'s verbs: kept nowhere.
struct Sink;

#[async_trait::async_trait]
impl WorkItemVerbs for Sink {
    async fn invoke(
        &self,
        _call: oxplow_domain::work_items::VerbCall,
        _verb: &str,
        _input: serde_json::Value,
    ) -> Result<oxplow_domain::work_items::VerbOutcome, oxplow_domain::CommandError> {
        Ok(oxplow_domain::work_items::VerbOutcome {
            result: serde_json::json!({ "tracked": false }),
            events: Vec::new(),
            inverse: None,
        })
    }

    async fn restart(&self) {}
}

/// A new item, as `oxplow.work_item.create` takes it.
#[derive(Debug, Clone, Default)]
pub struct NewItem {
    pub title: String,
    pub body: String,
    pub parent_uri: Option<String>,
    /// A stage of the list, or a bucket for its default.
    pub stage: Option<String>,
    pub native: Option<Value>,
}

/// The `work_item.*` commands, typed: each call is one run through the
/// bus as `actor` — dispatched to the item's provider, audited and
/// policy-checked like any other.
#[derive(Clone)]
pub struct WorkItems {
    bus: Arc<CommandBus>,
}

impl WorkItems {
    pub fn new(bus: Arc<CommandBus>) -> Self {
        Self { bus }
    }

    async fn run(
        &self,
        actor: &Actor,
        name: &str,
        input: Value,
    ) -> Result<CommandOutcome, CommandError> {
        self.bus.run(actor, name, input, false).await
    }

    /// File an item; its ref.
    /// File `item` on the active work list: its ref, or `None` when the
    /// list keeps nothing (none, a sink).
    pub async fn create(
        &self,
        actor: &Actor,
        item: NewItem,
    ) -> Result<Option<String>, CommandError> {
        let input = serde_json::to_value(oxplow_domain::work_items::WorkItemCreateInput {
            title: item.title,
            body: (!item.body.is_empty()).then_some(item.body),
            parent_uri: item.parent_uri,
            stage: item.stage,
            native: item.native,
            assignee: None,
            links: Vec::new(),
        })
        .expect("input serializes");
        let out = self.run(actor, work_item::CREATE, input).await?;
        if out.result["tracked"] == serde_json::json!(false) {
            return Ok(None);
        }
        out.result["uri"]
            .as_str()
            .map(|r| Some(r.to_string()))
            .ok_or_else(|| CommandError::Failed {
                message: "oxplow.work_item.create returned no uri".into(),
            })
    }

    pub async fn update(
        &self,
        actor: &Actor,
        input: oxplow_domain::work_items::WorkItemUpdateInput,
    ) -> Result<CommandOutcome, CommandError> {
        let input = serde_json::to_value(input).expect("input serializes");
        self.run(actor, work_item::UPDATE, input).await
    }

    pub async fn transition(
        &self,
        actor: &Actor,
        item_ref: &str,
        to: &str,
    ) -> Result<CommandOutcome, CommandError> {
        self.run(actor, work_item::NAME, json!({ "uri": item_ref, "to": to }))
            .await
    }

    pub async fn link(
        &self,
        actor: &Actor,
        from: &str,
        to: &str,
        link_type: &str,
    ) -> Result<CommandOutcome, CommandError> {
        self.run(
            actor,
            work_item::LINK,
            json!({ "uri": from, "target": to, "link_type": link_type }),
        )
        .await
    }

    /// Remove `from`'s `link_type` link to `to`.
    pub async fn unlink(
        &self,
        actor: &Actor,
        from: &str,
        to: &str,
        link_type: &str,
    ) -> Result<CommandOutcome, CommandError> {
        self.run(
            actor,
            work_item::UNLINK,
            json!({ "uri": from, "target": to, "link_type": link_type }),
        )
        .await
    }

    pub async fn comment(
        &self,
        actor: &Actor,
        item_ref: &str,
        body: &str,
    ) -> Result<CommandOutcome, CommandError> {
        self.run(
            actor,
            work_item::COMMENT,
            json!({ "uri": item_ref, "body": body }),
        )
        .await
    }

    /// Put `item_ref` just before `before` in its assignee's queue (core's,
    /// for every list).
    pub async fn reorder_before(
        &self,
        actor: &Actor,
        item_ref: &str,
        before: &str,
    ) -> Result<CommandOutcome, CommandError> {
        self.run(
            actor,
            work_item::REORDER,
            json!({ "uri": item_ref, "before": before }),
        )
        .await
    }

    /// Assign `item_ref` to `to` (a thread's or the person's URI; `None`:
    /// nobody) — core's, for every list.
    pub async fn assign(
        &self,
        actor: &Actor,
        item_ref: &str,
        to: Option<&str>,
    ) -> Result<CommandOutcome, CommandError> {
        self.run(
            actor,
            work_item::ASSIGN,
            json!({ "uri": item_ref, "to": to }),
        )
        .await
    }

    /// Destructive: `confirmed` is the person's confirmation (an agent's
    /// is ignored — its run is proposed).
    pub async fn delete(
        &self,
        actor: &Actor,
        item_ref: &str,
        confirmed: bool,
    ) -> Result<CommandOutcome, CommandError> {
        self.bus
            .run(
                actor,
                work_item::DELETE,
                json!({ "uri": item_ref }),
                confirmed,
            )
            .await
    }

    /// The undo of a run (its audit row).
    pub async fn undo(&self, actor: &Actor, audit_id: i64) -> Result<CommandOutcome, CommandError> {
        self.bus.undo(actor, audit_id, false).await
    }
}

/// Upserts a list's item into `work_item` from its `work_item.recorded`
/// event, by ref — idempotent, so a replay restates the same row; every
/// list's, oxplow's own tasks' included. Keeps who each item is assigned
/// to and its place in their queue (`work_item_assignment`) from core's
/// `work_item.assigned`; an item recorded done or canceled has no
/// assignee, whichever list closed it.
pub struct WorkItemsProjection;

impl WorkItemsProjection {
    pub const NAME: &'static str = "work_items.project";
}

impl EventConsumer for WorkItemsProjection {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == WorkItemRecorded::TYPE || event_type == WorkItemAssigned::TYPE
    }

    fn handle(&self, conn: &rusqlite::Connection, event: &StoredEvent) -> Result<(), DomainError> {
        if event.envelope.event_type == WorkItemAssigned::TYPE {
            let assigned: WorkItemAssignedV1 =
                serde_json::from_value(event.envelope.payload.clone()).map_err(|e| {
                    DomainError::Invalid(format!("work_item.assigned payload: {e}"))
                })?;
            return assign_tx(conn, &assigned, &event.envelope.at.to_string());
        }
        let WorkItemRecordedV3 { item } = serde_json::from_value(event.envelope.payload.clone())
            .map_err(|e| DomainError::Invalid(format!("work_item.recorded payload: {e}")))?;
        let provider = provider_of(&item.uri)
            .map_err(|e| DomainError::Invalid(e.to_string()))?
            .to_string();
        let at = event.envelope.at.to_string();
        let bucket = stage_bucket_tx(conn, &provider, &item.stage)?;
        conn.execute(
            "INSERT INTO work_item (uri, provider, title, body, state, stage, native,
                                    parent_uri, created_at, updated_at, deleted_at, closed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, CASE WHEN ?10 THEN ?9 END,
                     CASE WHEN ?5 IN ('done', 'canceled') THEN ?9 END)
             ON CONFLICT(uri) DO UPDATE SET
                title = excluded.title, body = excluded.body, state = excluded.state,
                stage = excluded.stage, native = excluded.native,
                parent_uri = excluded.parent_uri, updated_at = excluded.updated_at,
                -- When it first closed (as recorded); reopened, open again.
                closed_at = CASE WHEN excluded.closed_at IS NULL THEN NULL
                                 ELSE coalesce(work_item.closed_at, excluded.closed_at) END,
                deleted_at = CASE WHEN ?10 THEN coalesce(work_item.deleted_at, ?9) END",
            rusqlite::params![
                item.uri,
                provider,
                item.title,
                item.body,
                bucket.as_str(),
                item.stage,
                item.native.to_string(),
                item.parent_uri,
                at,
                item.deleted,
            ],
        )
        .map_err(|e| DomainError::Storage(e.to_string()))?;
        let storage = |e: rusqlite::Error| DomainError::Storage(e.to_string());
        // What the record states, it restates whole; what it leaves out,
        // the host keeps.
        // Closed, by whichever list: nobody's to work on.
        if bucket.is_closed() {
            conn.execute(
                "DELETE FROM work_item_assignment WHERE uri = ?1",
                [&item.uri],
            )
            .map_err(storage)?;
        }
        if let Some(links) = &item.links {
            // The set is restated whole, but a link already there keeps
            // when it was made: only the ones the record dropped go.
            for l in links {
                // The target's kind, read through the grammar: a link to
                // what isn't a ref is the provider's mistake, and dropped.
                let Some(kind) = oxplow_domain::uri::build::resource_of(&l.target) else {
                    tracing::warn!(item = %item.uri, target = %l.target, "a link's target isn't a ref");
                    continue;
                };
                conn.execute(
                    "INSERT OR IGNORE INTO work_item_link (from_uri, to_uri, to_kind, link_type, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![item.uri, l.target, kind, l.link_type, at],
                )
                .map_err(|e| DomainError::Invalid(format!("link `{}`: {e}", l.link_type)))?;
            }
            let kept: Vec<String> = links
                .iter()
                .map(|l| format!("{}\n{}", l.target, l.link_type))
                .collect();
            conn.execute(
                "DELETE FROM work_item_link
                 WHERE from_uri = ?1
                   AND to_uri || char(10) || link_type NOT IN (SELECT value FROM json_each(?2))",
                rusqlite::params![item.uri, serde_json::to_string(&kept).unwrap_or_default()],
            )
            .map_err(storage)?;
        }
        if let Some(comments) = &item.comments {
            conn.execute("DELETE FROM work_item_comment WHERE uri = ?1", [&item.uri])
                .map_err(storage)?;
            for c in comments {
                conn.execute(
                    "INSERT INTO work_item_comment (id, uri, body, author, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![
                        format!("{}#{}", item.uri, c.id),
                        item.uri,
                        c.body,
                        c.author,
                        c.created_at.as_deref().unwrap_or(&at),
                    ],
                )
                .map_err(storage)?;
            }
        }
        Ok(())
    }
}

/// The bucket of `provider`'s stage `stage`, as the list declares it
/// (`capability_provider.stages_json`): a record names its stage, and what
/// it means to core is the list's declaration. A stage the list doesn't
/// declare is refused (the event dead-letters), naming the ones it does.
fn stage_bucket_tx(
    conn: &rusqlite::Connection,
    provider: &str,
    stage: &str,
) -> Result<Bucket, DomainError> {
    let storage = |e: rusqlite::Error| DomainError::Storage(e.to_string());
    let declared: Vec<(String, String)> = conn
        .prepare(
            "SELECT json_extract(s.value, '$.id'), json_extract(s.value, '$.bucket')
               FROM capability_provider c, json_each(c.stages_json) s
              WHERE c.capability = 'work_items' AND c.provider = ?1",
        )
        .and_then(|mut stmt| {
            stmt.query_map([provider], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(storage)?;
    declared
        .iter()
        .find(|(id, _)| id == stage)
        .and_then(|(_, b)| Bucket::named(b))
        .ok_or_else(|| {
            DomainError::Invalid(format!(
                "`{provider}` recorded stage `{stage}`, which it doesn't declare (its stages: {})",
                declared
                    .iter()
                    .map(|(id, _)| id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })
}

/// Keep `assigned` in `work_item_assignment`: the item goes to `to`'s
/// queue (nobody's when `None`) next to the item it names, or at its end,
/// and that queue is renumbered. A closed item has no assignee: its row
/// goes. Idempotent: the same event places the item at the same place.
fn assign_tx(
    conn: &rusqlite::Connection,
    assigned: &WorkItemAssignedV1,
    at: &str,
) -> Result<(), DomainError> {
    use rusqlite::OptionalExtension as _;
    let storage = |e: rusqlite::Error| DomainError::Storage(e.to_string());
    let item = &assigned.work_item;
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT provider, state FROM work_item WHERE uri = ?1",
            [item],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(storage)?;
    let Some((provider, state)) = row else {
        // Not recorded (yet): nothing to place.
        return Ok(());
    };
    if matches!(state.as_str(), "done" | "canceled") {
        conn.execute("DELETE FROM work_item_assignment WHERE uri = ?1", [item])
            .map_err(storage)?;
        return Ok(());
    }
    // The queue it joins: the list's open items assigned to `to`.
    let mut stmt = conn
        .prepare(
            "SELECT w.uri FROM work_item w
             LEFT JOIN work_item_assignment a ON a.uri = w.uri
             WHERE w.provider = ?1 AND w.deleted_at IS NULL
               AND w.state NOT IN ('done', 'canceled')
               AND a.assignee IS ?2 AND w.uri <> ?3
             ORDER BY a.rank IS NULL, a.rank, w.created_at, w.uri",
        )
        .map_err(storage)?;
    let mut queue: Vec<String> = stmt
        .query_map(rusqlite::params![provider, assigned.to, item], |r| r.get(0))
        .map_err(storage)?
        .collect::<rusqlite::Result<_>>()
        .map_err(storage)?;
    let index = match (&assigned.before, &assigned.after) {
        (Some(b), _) => queue.iter().position(|u| u == b),
        (None, Some(a)) => queue.iter().position(|u| u == a).map(|i| i + 1),
        (None, None) => None,
    }
    .unwrap_or(queue.len());
    queue.insert(index, item.clone());
    for (rank, uri) in queue.iter().enumerate() {
        let assigned_at = (uri == item).then_some(at);
        conn.execute(
            "INSERT INTO work_item_assignment (uri, assignee, rank, assigned_at)
             VALUES (?1, ?2, ?3, coalesce(?4, ?5))
             ON CONFLICT(uri) DO UPDATE SET
                assignee = excluded.assignee, rank = excluded.rank,
                assigned_at = coalesce(?4, work_item_assignment.assigned_at)",
            rusqlite::params![uri, assigned.to, rank as f64, assigned_at, at],
        )
        .map_err(storage)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::work_items::WorkItemRecord;
    use oxplow_domain::Envelope;

    /// `sql`'s rows as JSON, over the published datasets.
    async fn rows(svc: &crate::Services, sql: &str) -> serde_json::Value {
        let out = svc.sql.query_sql(sql, vec![], None).await.unwrap();
        serde_json::to_value(out.rows).unwrap()
    }

    /// The interface carries what any list's screens need — who an item
    /// is assigned to, its rank, when it closed, its links and comments — and
    /// shows only the active list's items: with none, nothing.
    #[tokio::test]
    async fn the_interface_reads_the_active_lists_items() {
        use crate::commands::work_item as w;
        use oxplow_domain::Actor;
        use serde_json::json;
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let svc = &fx.svc;
        let task = oxplow_tasks::work_item_ref(fx.task);
        let run = |name: &'static str, input: serde_json::Value| {
            let svc = svc.clone();
            async move {
                svc.commands
                    .run(&Actor::Human, name, input, false)
                    .await
                    .unwrap()
            }
        };
        let child = run(
            w::CREATE,
            json!({ "title": "child", "parent_uri": task, "assignee": fx.thread.to_string() }),
        )
        .await
        .result["uri"]
            .as_str()
            .unwrap()
            .to_string();
        run(
            w::LINK,
            json!({ "uri": child, "target": task, "link_type": "blocks" }),
        )
        .await;
        run(w::COMMENT, json!({ "uri": task, "body": "Looks right." })).await;
        run(w::NAME, json!({ "uri": child, "to": "done" })).await;
        assert_eq!(
            rows(
                svc,
                &format!(
                    "SELECT uri, assignee, rank IS NOT NULL, closed_at IS NOT NULL FROM v_work_item
                  WHERE uri IN ('{task}', '{child}') ORDER BY uri"
                )
            )
            .await,
            // Done, the child is nobody's: who worked on it is its efforts.
            json!([
                [task, oxplow_domain::uri::build::thread_uri(fx.thread), 1, 0],
                [child, null, 0, 1]
            ])
        );
        assert_eq!(
            rows(
                svc,
                "SELECT from_uri, to_uri, link_type FROM v_work_item_link"
            )
            .await,
            json!([[child, task, "blocks"]])
        );
        assert_eq!(
            rows(svc, "SELECT uri, body FROM v_work_item_comment").await,
            json!([[task, "Looks right."]])
        );
        svc.config
            .write()
            .unwrap()
            .personal_active_providers
            .insert("work_items".into(), oxplow_domain::capability::NONE.into());
        crate::test_fixtures::activate_with_problems(svc).await;
        for view in ["v_work_item", "v_work_item_link", "v_work_item_comment"] {
            assert_eq!(
                rows(svc, &format!("SELECT count(*) FROM {view}")).await,
                json!([[0]]),
                "{view}"
            );
        }
    }

    /// The fake as a listed work list, its stages declared, though it
    /// isn't running: what the projection reads a record's bucket from.
    async fn list_fake(svc: &crate::Services) {
        svc.db
            .transaction(|tx| {
                tx.execute(
                    "INSERT INTO capability_provider
                       (capability, provider, features_json, active, title, source, available,
                        capability_title, choosable, optional, stages_json)
                     VALUES ('work_items', 'fake', '{}', 0, 'fake', 'external', 0,
                             'Work list', 1, 1, ?1)",
                    [serde_json::json!([
                        { "id": "ready", "title": "Ready", "bucket": "todo" },
                        { "id": "in_progress", "title": "Doing", "bucket": "in_progress" },
                        { "id": "shipped", "title": "Shipped", "bucket": "done" }
                    ])
                    .to_string()],
                )
                .map_err(oxplow_db::map_sql_err)?;
                Ok(())
            })
            .await
            .unwrap();
    }

    fn recorded(item_ref: &str, title: &str, deleted: bool) -> Envelope {
        Envelope::typed::<WorkItemRecorded>(
            "provider:fake",
            &WorkItemRecordedV3 {
                item: WorkItemRecord {
                    uri: item_ref.into(),
                    title: title.into(),
                    body: String::new(),
                    // A stage both the fake and oxplow's tasks declare.
                    stage: "in_progress".into(),
                    native: json!({ "points": 3 }),
                    parent_uri: None,
                    deleted,
                    links: None,
                    comments: None,
                },
            },
        )
        .with_subject([item_ref])
    }

    async fn row(svc: &crate::Services, item_ref: &str) -> Option<(String, String, bool)> {
        let item_ref = item_ref.to_string();
        svc.db
            .read(move |c| {
                use rusqlite::OptionalExtension;
                c.query_row(
                    "SELECT title, state, deleted_at IS NOT NULL FROM work_item WHERE uri = ?1",
                    [item_ref],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(|e| DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap()
    }

    /// Another provider's items reach `work_item` from its
    /// `work_item.recorded` events, restated by ref; an oxplow record is
    /// refused (dead-lettered) — those rows are the task cores'.
    #[tokio::test]
    async fn a_providers_records_are_projected_by_ref() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let svc = &fx.svc;
        list_fake(svc).await;
        let r = "oxplow:oxplow/work_item?id=W-1&provider=fake";
        svc.event_log_store
            .append(recorded(r, "first", false))
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert_eq!(
            row(svc, r).await,
            Some(("first".into(), "in_progress".into(), false))
        );
        svc.event_log_store
            .append(recorded(r, "renamed", true))
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert_eq!(
            row(svc, r).await,
            Some(("renamed".into(), "in_progress".into(), true))
        );

        // oxplow's own tasks take the same path.
        let own = oxplow_tasks::work_item_ref(fx.task);
        svc.event_log_store
            .append(recorded(&own, "restated", false))
            .await
            .unwrap();
        let report = svc.event_pump.run_once().await.unwrap();
        assert_eq!(report.dead_lettered, 0);
        assert_eq!(row(svc, &own).await.unwrap().0, "restated");
        assert!(WorkItemsProjection.handles(WorkItemRecorded::TYPE));
    }

    fn recorded_with_links(item_ref: &str, links: &[(&str, &str)]) -> Envelope {
        let mut env = recorded(item_ref, "linked", false);
        let mut payload: WorkItemRecordedV3 = serde_json::from_value(env.payload.clone()).unwrap();
        payload.item.links = Some(
            links
                .iter()
                .map(
                    |(target, link_type)| oxplow_domain::work_items::LinkRecord {
                        target: (*target).into(),
                        link_type: (*link_type).into(),
                    },
                )
                .collect(),
        );
        env.payload = serde_json::to_value(payload).unwrap();
        env
    }

    /// `oxplow:oxplow/work_item?id=W-1&provider=fake`'s links: `(to_uri, created_at)`.
    async fn links(svc: &crate::Services) -> Vec<(String, String)> {
        svc.db
            .read(|tx| {
                let mut stmt = tx
                    .prepare(
                        "SELECT to_uri, created_at FROM work_item_link
                         WHERE from_uri = 'oxplow:oxplow/work_item?id=W-1&provider=fake' ORDER BY to_uri",
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                    .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
    }

    /// A record restates an item's links whole, but a link that was
    /// already there keeps when it was made; one the record drops goes.
    #[tokio::test]
    async fn a_restated_link_keeps_when_it_was_made() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let svc = &fx.svc;
        list_fake(svc).await;
        let r = "oxplow:oxplow/work_item?id=W-1&provider=fake";
        let mut first = recorded_with_links(
            r,
            &[("oxplow:oxplow/work_item?id=W-2&provider=fake", "blocks")],
        );
        first.at = oxplow_domain::Timestamp::from_unix_ms(1_000_000);
        svc.event_log_store.append(first).await.unwrap();
        svc.event_pump.run_once().await.unwrap();
        let before = links(svc).await;
        assert_eq!(before.len(), 1);

        let mut again = recorded_with_links(
            r,
            &[
                ("oxplow:oxplow/work_item?id=W-2&provider=fake", "blocks"),
                ("oxplow:oxplow/work_item?id=W-3&provider=fake", "relates_to"),
            ],
        );
        again.at = oxplow_domain::Timestamp::from_unix_ms(2_000_000);
        svc.event_log_store.append(again).await.unwrap();
        svc.event_pump.run_once().await.unwrap();
        let after = links(svc).await;
        assert_eq!(after[0], before[0], "the kept link keeps its time");
        assert_eq!(after[1].0, "oxplow:oxplow/work_item?id=W-3&provider=fake");
        assert_ne!(
            after[1].1, before[0].1,
            "the new link has the record's time"
        );

        let mut dropped = recorded_with_links(
            r,
            &[("oxplow:oxplow/work_item?id=W-3&provider=fake", "relates_to")],
        );
        dropped.at = oxplow_domain::Timestamp::from_unix_ms(3_000_000);
        svc.event_log_store.append(dropped).await.unwrap();
        svc.event_pump.run_once().await.unwrap();
        let left = links(svc).await;
        assert_eq!(left, vec![after[1].clone()]);
    }

    fn assigned(item: &str, to: Option<&str>, before: Option<&str>) -> Envelope {
        Envelope::typed::<WorkItemAssigned>(
            "human",
            &WorkItemAssignedV1 {
                work_item: item.into(),
                from: None,
                to: to.map(str::to_string),
                before: before.map(str::to_string),
                after: None,
            },
        )
        .with_subject([item])
    }

    /// Who each item is assigned to and its rank, by uri, from the
    /// projection's own table (the provider isn't running, so the
    /// interface, which shows the active list, doesn't list them).
    async fn assignments(svc: &crate::Services) -> Vec<(String, Option<String>, f64)> {
        svc.db
            .read(|c| {
                let mut stmt = c
                    .prepare(
                        "SELECT uri, assignee, rank FROM work_item_assignment
                         WHERE uri LIKE '%provider=fake' ORDER BY rank, uri",
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .map_err(oxplow_db::map_sql_err)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
    }

    /// Assignment is core's, for any list's item: `work_item.assigned` puts
    /// an item in its assignee's queue (at the end, or before another
    /// there) and renumbers that queue; an item recorded done has none.
    #[tokio::test]
    async fn an_assignment_places_an_item_in_its_queue_and_a_close_releases_it() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let svc = &fx.svc;
        list_fake(svc).await;
        let (a, b, c) = (
            "oxplow:oxplow/work_item?id=W-1&provider=fake",
            "oxplow:oxplow/work_item?id=W-2&provider=fake",
            "oxplow:oxplow/work_item?id=W-3&provider=fake",
        );
        let person = oxplow_domain::uri::build::person_uri(None);
        for item in [a, b, c] {
            svc.event_log_store
                .append(recorded(item, item, false))
                .await
                .unwrap();
        }
        for event in [
            assigned(a, Some(&person), None),
            assigned(b, Some(&person), None),
            assigned(c, Some(&person), Some(a)),
        ] {
            svc.event_log_store.append(event).await.unwrap();
        }
        svc.event_pump.run_once().await.unwrap();
        let who = Some(person.clone());
        assert_eq!(
            assignments(svc).await,
            vec![
                (c.to_string(), who.clone(), 0.0),
                (a.to_string(), who.clone(), 1.0),
                (b.to_string(), who.clone(), 2.0),
            ]
        );

        // Released: among the unassigned; done: no assignment at all.
        svc.event_log_store
            .append(assigned(a, None, None))
            .await
            .unwrap();
        let mut done = recorded(b, b, false);
        let mut payload: WorkItemRecordedV3 = serde_json::from_value(done.payload.clone()).unwrap();
        payload.item.stage = "shipped".into();
        done.payload = serde_json::to_value(payload).unwrap();
        svc.event_log_store.append(done).await.unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert_eq!(
            assignments(svc).await,
            vec![(a.to_string(), None, 0.0), (c.to_string(), who, 0.0),]
        );
    }
}
