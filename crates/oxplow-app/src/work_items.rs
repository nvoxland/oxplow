//! The work-items capability in the app (`.context/work-items.md`):
//!
//! - [`built_in_provider`], the factory for a built-in work list
//!   (`oxplow:tasks`, oxplow's own tasks, from `oxplow-tasks`), and
//!   [`register_built_ins`], which registers the ones the project's
//!   extensions declare;
//! - [`WorkItems`], a typed client over the `work_item.*` commands: the
//!   one write surface for every provider (the conformance suite and
//!   `task_writes` use it);
//! - [`WorkItemsProjection`], the pump consumer (`work_items.project`)
//!   that upserts every list's items into `work_item` from its
//!   `work_item.recorded` events.

use std::sync::Arc;

use oxplow_domain::events::schema::{EventType, WorkItemRecorded, WorkItemRecordedV2};
use oxplow_domain::work_items::{
    provider_of, CanonicalState, List, WorkItemVerbs, WorkItemsFeatures, WorkItemsProvider,
};
use oxplow_domain::{Actor, CommandError, CommandOutcome, DomainError, StoredEvent};
use serde_json::{json, Value};

use crate::commands::{work_item, CommandBus};
use crate::event_pump::EventConsumer;

/// oxplow's own provider name: `work_item:oxplow:tsk<n>`.
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
            ordering: has("ordering"),
            lists: has("lists"),
        },
        verbs,
        id_pattern: built_in.id_pattern.map(str::to_string),
        sink: false,
    })
}

/// Register the built-in work lists `declared` names (the project's
/// extensions' `implementations:`), and unregister the ones it no longer
/// does: a built-in list is reachable only while an extension declares
/// it; its data stays.
pub fn register_built_ins(
    registry: &oxplow_domain::work_items::WorkItemsRegistry,
    declared: &[crate::capabilities::Implementation],
    db: &oxplow_db::Database,
) {
    for b in crate::capabilities::BUILT_INS
        .iter()
        .filter(|b| b.capability == "work_items")
    {
        let named: Vec<&crate::capabilities::Implementation> = declared
            .iter()
            .filter(|i| matches!(i.source, crate::capabilities::Source::BuiltIn(e) if e == b.entry))
            .collect();
        match (named.is_empty(), b.provider) {
            (false, _) => {
                if let Some(provider) = built_in_provider(b.entry, db) {
                    registry.register(provider);
                }
            }
            (true, Some(id)) => registry.unregister(id),
            (true, None) => {}
        }
    }
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
            ordering: true,
            lists: true,
        },
        verbs: Arc::new(Sink),
        id_pattern: Some(".+".into()),
        sink: true,
    }
}

/// [`none_provider`]'s verbs: kept nowhere.
struct Sink;

#[async_trait::async_trait]
impl WorkItemVerbs for Sink {
    async fn invoke(
        &self,
        _actor: &oxplow_domain::Actor,
        _verb: &str,
        _input: serde_json::Value,
        _idempotency_key: Option<String>,
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
    pub parent_ref: Option<String>,
    pub state: Option<CanonicalState>,
    pub native_state: Option<String>,
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
            parent_ref: item.parent_ref,
            state: item.state,
            native_state: item.native_state,
            native: item.native,
            thread: None,
        })
        .expect("input serializes");
        let out = self.run(actor, work_item::CREATE, input).await?;
        if out.result["tracked"] == serde_json::json!(false) {
            return Ok(None);
        }
        out.result["ref"]
            .as_str()
            .map(|r| Some(r.to_string()))
            .ok_or_else(|| CommandError::Failed {
                message: "oxplow.work_item.create returned no ref".into(),
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
        to: CanonicalState,
        native_state: Option<&str>,
    ) -> Result<CommandOutcome, CommandError> {
        let mut input = json!({ "ref": item_ref, "to": to });
        if let Some(n) = native_state {
            input["native_state"] = n.into();
        }
        self.run(actor, work_item::NAME, input).await
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
            json!({ "ref": from, "target": to, "link_type": link_type }),
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
            json!({ "ref": item_ref, "body": body }),
        )
        .await
    }

    /// Put `item_ref` just before `before` on its list (`ordering`).
    pub async fn reorder_before(
        &self,
        actor: &Actor,
        item_ref: &str,
        before: &str,
    ) -> Result<CommandOutcome, CommandError> {
        self.run(
            actor,
            work_item::REORDER,
            json!({ "ref": item_ref, "before": before }),
        )
        .await
    }

    /// Move `item_ref` to the backlog (`lists`).
    pub async fn move_to_backlog(
        &self,
        actor: &Actor,
        item_ref: &str,
    ) -> Result<CommandOutcome, CommandError> {
        self.run(
            actor,
            work_item::MOVE,
            json!({ "ref": item_ref, "to": "backlog" }),
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
                json!({ "ref": item_ref }),
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
/// list's, oxplow's own tasks' included.
pub struct WorkItemsProjection;

impl WorkItemsProjection {
    pub const NAME: &'static str = "work_items.project";
}

impl EventConsumer for WorkItemsProjection {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == WorkItemRecorded::TYPE
    }

    fn handle(&self, conn: &rusqlite::Connection, event: &StoredEvent) -> Result<(), DomainError> {
        let WorkItemRecordedV2 { item } = serde_json::from_value(event.envelope.payload.clone())
            .map_err(|e| DomainError::Invalid(format!("work_item.recorded payload: {e}")))?;
        let provider = provider_of(&item.item_ref)
            .map_err(|e| DomainError::Invalid(e.to_string()))?
            .to_string();
        let at = event.envelope.at.to_string();
        conn.execute(
            "INSERT INTO work_item (ref, provider, title, body, state, native_state, native,
                                    parent_ref, created_at, updated_at, deleted_at,
                                    thread_id, closed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, CASE WHEN ?10 THEN ?9 END, ?11,
                     CASE WHEN ?5 IN ('done', 'canceled') THEN ?9 END)
             ON CONFLICT(ref) DO UPDATE SET
                title = excluded.title, body = excluded.body, state = excluded.state,
                native_state = excluded.native_state, native = excluded.native,
                parent_ref = excluded.parent_ref, updated_at = excluded.updated_at,
                -- When it first closed (as recorded); reopened, open again.
                closed_at = CASE WHEN excluded.closed_at IS NULL THEN NULL
                                 ELSE coalesce(work_item.closed_at, excluded.closed_at) END,
                deleted_at = CASE WHEN ?10 THEN coalesce(work_item.deleted_at, ?9) END",
            rusqlite::params![
                item.item_ref,
                provider,
                item.title,
                item.body,
                item.state.as_str(),
                item.native_state,
                item.native.to_string(),
                item.parent_ref,
                at,
                item.deleted,
                // The list it's on: the thread that filed it, at its
                // first record only (tsk1041); a restatement keeps it.
                event.envelope.anchors.thread_id.map(|t| t.value()),
            ],
        )
        .map_err(|e| DomainError::Storage(e.to_string()))?;
        let storage = |e: rusqlite::Error| DomainError::Storage(e.to_string());
        // What the record states, it restates whole; what it leaves out,
        // the host keeps.
        if let Some(list) = &item.list {
            let thread = match list {
                List::Backlog => None,
                List::Thread(raw) => Some(
                    raw.parse::<oxplow_domain::ThreadId>()
                        .map_err(|e| DomainError::Invalid(format!("list `{raw}`: {e}")))?
                        .value(),
                ),
            };
            conn.execute(
                "UPDATE work_item SET thread_id = ?2 WHERE ref = ?1",
                rusqlite::params![item.item_ref, thread],
            )
            .map_err(storage)?;
        }
        if let Some(rank) = item.rank {
            conn.execute(
                "UPDATE work_item SET rank = ?2 WHERE ref = ?1",
                rusqlite::params![item.item_ref, rank],
            )
            .map_err(storage)?;
        }
        if let Some(links) = &item.links {
            // The set is restated whole, but a link already there keeps
            // when it was made: only the ones the record dropped go.
            for l in links {
                conn.execute(
                    "INSERT OR IGNORE INTO work_item_link (from_ref, to_ref, link_type, created_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![item.item_ref, l.target, l.link_type, at],
                )
                .map_err(|e| DomainError::Invalid(format!("link `{}`: {e}", l.link_type)))?;
            }
            let kept: Vec<String> = links
                .iter()
                .map(|l| format!("{}\n{}", l.target, l.link_type))
                .collect();
            conn.execute(
                "DELETE FROM work_item_link
                 WHERE from_ref = ?1
                   AND to_ref || char(10) || link_type NOT IN (SELECT value FROM json_each(?2))",
                rusqlite::params![
                    item.item_ref,
                    serde_json::to_string(&kept).unwrap_or_default()
                ],
            )
            .map_err(storage)?;
        }
        if let Some(comments) = &item.comments {
            conn.execute(
                "DELETE FROM work_item_comment WHERE ref = ?1",
                [&item.item_ref],
            )
            .map_err(storage)?;
            for c in comments {
                conn.execute(
                    "INSERT INTO work_item_comment (id, ref, body, author, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![
                        format!("{}#{}", item.item_ref, c.id),
                        item.item_ref,
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

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::work_items::WorkItemRecord;
    use oxplow_domain::Envelope;

    /// `sql`'s rows as JSON, over the published models.
    async fn rows(svc: &crate::Services, sql: &str) -> serde_json::Value {
        let out = svc.sql.query_sql(sql, vec![], None).await.unwrap();
        serde_json::to_value(out.rows).unwrap()
    }

    /// The interface carries what any list's screens need — the list an
    /// item is on, its rank, when it closed, its links and comments — and
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
            json!({ "title": "child", "parent_ref": task, "thread": fx.thread.to_string() }),
        )
        .await
        .result["ref"]
            .as_str()
            .unwrap()
            .to_string();
        run(
            w::LINK,
            json!({ "ref": child, "target": task, "link_type": "blocks" }),
        )
        .await;
        run(w::COMMENT, json!({ "ref": task, "body": "Looks right." })).await;
        run(w::NAME, json!({ "ref": child, "to": "done" })).await;
        assert_eq!(
            rows(
                svc,
                &format!(
                "SELECT ref, thread_id, rank IS NOT NULL, closed_at IS NOT NULL FROM v_work_item
                  WHERE ref IN ('{task}', '{child}') ORDER BY ref"
            )
            )
            .await,
            json!([
                [task, fx.thread.value(), 1, 0],
                [child, fx.thread.value(), 1, 1]
            ])
        );
        assert_eq!(
            rows(
                svc,
                "SELECT from_ref, to_ref, link_type FROM v_work_item_link"
            )
            .await,
            json!([[child, task, "blocks"]])
        );
        assert_eq!(
            rows(svc, "SELECT ref, body FROM v_work_item_comment").await,
            json!([[task, "Looks right."]])
        );
        svc.config
            .write()
            .unwrap()
            .personal_active_providers
            .insert("work_items".into(), oxplow_domain::capability::NONE.into());
        crate::capabilities::refresh(svc).await.unwrap();
        for view in ["v_work_item", "v_work_item_link", "v_work_item_comment"] {
            assert_eq!(
                rows(svc, &format!("SELECT count(*) FROM {view}")).await,
                json!([[0]]),
                "{view}"
            );
        }
    }

    fn recorded(item_ref: &str, title: &str, deleted: bool) -> Envelope {
        Envelope::typed::<WorkItemRecorded>(
            "provider:fake",
            &WorkItemRecordedV2 {
                item: WorkItemRecord {
                    item_ref: item_ref.into(),
                    title: title.into(),
                    body: String::new(),
                    state: CanonicalState::InProgress,
                    native_state: "Doing".into(),
                    native: json!({ "points": 3 }),
                    parent_ref: None,
                    deleted,
                    rank: None,
                    links: None,
                    comments: None,
                    list: None,
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
                    "SELECT title, state, deleted_at IS NOT NULL FROM work_item WHERE ref = ?1",
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
        let r = "work_item:fake:W-1";
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

    /// tsk1041: an outside tracker's item keeps the thread that filed it
    /// (the record's thread anchor, at its first record), so "This thread"
    /// lists it; a later restatement from elsewhere doesn't move it.
    fn recorded_with_links(item_ref: &str, links: &[(&str, &str)]) -> Envelope {
        let mut env = recorded(item_ref, "linked", false);
        let mut payload: WorkItemRecordedV2 = serde_json::from_value(env.payload.clone()).unwrap();
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

    /// `work_item:fake:W-1`'s links: `(to_ref, created_at)`.
    async fn links(svc: &crate::Services) -> Vec<(String, String)> {
        svc.db
            .read(|tx| {
                let mut stmt = tx
                    .prepare(
                        "SELECT to_ref, created_at FROM work_item_link
                         WHERE from_ref = 'work_item:fake:W-1' ORDER BY to_ref",
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
        let r = "work_item:fake:W-1";
        let mut first = recorded_with_links(r, &[("work_item:fake:W-2", "blocks")]);
        first.at = oxplow_domain::Timestamp::from_unix_ms(1_000_000);
        svc.event_log_store.append(first).await.unwrap();
        svc.event_pump.run_once().await.unwrap();
        let before = links(svc).await;
        assert_eq!(before.len(), 1);

        let mut again = recorded_with_links(
            r,
            &[
                ("work_item:fake:W-2", "blocks"),
                ("work_item:fake:W-3", "relates_to"),
            ],
        );
        again.at = oxplow_domain::Timestamp::from_unix_ms(2_000_000);
        svc.event_log_store.append(again).await.unwrap();
        svc.event_pump.run_once().await.unwrap();
        let after = links(svc).await;
        assert_eq!(after[0], before[0], "the kept link keeps its time");
        assert_eq!(after[1].0, "work_item:fake:W-3");
        assert_ne!(
            after[1].1, before[0].1,
            "the new link has the record's time"
        );

        let mut dropped = recorded_with_links(r, &[("work_item:fake:W-3", "relates_to")]);
        dropped.at = oxplow_domain::Timestamp::from_unix_ms(3_000_000);
        svc.event_log_store.append(dropped).await.unwrap();
        svc.event_pump.run_once().await.unwrap();
        let left = links(svc).await;
        assert_eq!(left, vec![after[1].clone()]);
    }

    #[tokio::test]
    async fn an_outside_item_keeps_the_thread_that_filed_it() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let svc = &fx.svc;
        let r = "work_item:fake:W-7";
        let anchored = |title: &str| {
            recorded(r, title, false).with_anchors(oxplow_domain::Anchors {
                thread_id: Some(fx.thread),
                ..Default::default()
            })
        };
        svc.event_log_store.append(anchored("Kiwi")).await.unwrap();
        svc.event_log_store
            .append(recorded(r, "Kiwi, renamed by a sync", false))
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        // The projection's row (the provider isn't running, so the
        // interface, which shows the active list, doesn't list it).
        let thread: Option<i64> = svc
            .db
            .read(move |c| {
                c.query_row("SELECT thread_id FROM work_item WHERE ref = ?1", [r], |r| {
                    r.get(0)
                })
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(thread, Some(fx.thread.value()));
        // An oxplow task's thread is the task's own.
        let own = oxplow_tasks::work_item_ref(fx.task);
        let out = svc
            .sql
            .query_sql(
                "SELECT thread_id FROM v_work_item WHERE ref = ?1",
                vec![oxplow_db::SqlCell::Text(own)],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(out.rows).unwrap(),
            json!([[fx.thread.value()]])
        );
    }
}
