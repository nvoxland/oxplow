//! The default effort policy (`.context/work-tracking.md`): when a
//! thread's effort opens, closes and links, as a reaction to core's events.
//! It uses only what any policy could: the public `effort.*` and
//! `work_item.*` commands, run as its own effect actor, and the `v_*`
//! models.
//!
//! Rule 1, linking and task switch:
//! - an item moved to `in_progress` links the thread's open effort, opening
//!   one if none is open. The thread is the mover's (an agent's), else the
//!   item's. A descendant of the linked item refines the link; an ancestor
//!   leaves it; an unrelated item closes the effort (`switch`) and opens
//!   the next;
//! - an item moved to `done` or `canceled` closes the efforts linked to it;
//! - work linked to a `todo` item moves the item to `in_progress`. Nothing
//!   is ever marked done.
//!
//! Rule 2, open on change: a `thread.checkpoint` showing the worktree
//! changed in a turn that ran a tool able to change it opens an unlinked
//! effort on a thread with none open, adopting the turn (its tool calls,
//! tokens and runs move into it). A turn that only read never opens one,
//! so a person's own edits meanwhile don't either.
//!
//! The project chooses it like a work list: `activeProviders.effort_policy`
//! is `oxplow` (the default) or `none`, which leaves efforts to people and
//! agents.

use std::sync::{Arc, RwLock, Weak};

use async_trait::async_trait;
use oxplow_config::OxplowConfig;
use oxplow_db::SqlCell;
use oxplow_domain::events::schema::{
    EffortLinked, EffortLinkedV1, EffortOpened, EffortOpenedV2, EventType as _, ThreadCheckpoint,
    ThreadCheckpointV1, WorkItemStateChanged, WorkItemStateChangedV1,
};
use oxplow_domain::work_items::CanonicalState;
use oxplow_domain::{Actor, DomainError, EffortId, StoredEvent, ThreadId};
use serde_json::{json, Value};

use crate::commands::CommandBus;
use crate::event_pump::AsyncEventConsumer;
use crate::sql_gateway::SqlGateway;

/// The consumer's name (its checkpoint key; what callers settle on).
pub const NAME: &str = "effort.policy";
/// The capability a project picks its effort policy for.
pub const CAPABILITY: &str = "effort_policy";
/// The choice that turns the policy off.
pub const NONE: &str = "none";
/// The effect the policy's commands run as.
pub const EFFECT: &str = "oxplow:effort-policy";

/// How deep a parent chain is followed before giving up (a cycle).
const MAX_DEPTH: usize = 32;

pub struct EffortPolicyConsumer {
    pub bus: Weak<CommandBus>,
    pub sql: SqlGateway,
    pub config: Arc<RwLock<OxplowConfig>>,
}

fn actor() -> Actor {
    Actor::Effect {
        effect: EFFECT.into(),
    }
}

fn storage(e: impl std::fmt::Display) -> DomainError {
    DomainError::Storage(e.to_string())
}

/// A thread's open effort, as `v_effort` has it.
struct Open {
    id: EffortId,
    work_item: Option<String>,
}

impl EffortPolicyConsumer {
    fn active(&self) -> bool {
        let config = crate::config_service::read_config(&self.config);
        crate::capabilities::active_provider(&config, CAPABILITY) != NONE
    }

    async fn run(&self, name: &str, input: Value) -> Result<(), DomainError> {
        let bus = self
            .bus
            .upgrade()
            .ok_or_else(|| DomainError::Invariant("the command bus is gone".into()))?;
        bus.run(&actor(), name, input, false)
            .await
            .map(|_| ())
            .map_err(|e| storage(format!("{name}: {e}")))
    }

    async fn rows(
        &self,
        sql: &str,
        params: Vec<SqlCell>,
    ) -> Result<Vec<Vec<SqlCell>>, DomainError> {
        Ok(self.sql.query_sql(sql, params, None).await?.rows)
    }

    async fn open_on(&self, thread: ThreadId) -> Result<Option<Open>, DomainError> {
        let rows = self
            .rows(
                "SELECT id, work_item FROM v_effort WHERE thread_id = ?1 AND ended_at IS NULL",
                vec![SqlCell::Int(thread.value())],
            )
            .await?;
        Ok(rows.into_iter().next().and_then(|row| match &row[..] {
            [SqlCell::Int(id), item] => Some(Open {
                id: EffortId::new(*id),
                work_item: text(item),
            }),
            _ => None,
        }))
    }

    /// The item's state and thread, if it is a live item.
    async fn item(
        &self,
        item_ref: &str,
    ) -> Result<Option<(Option<CanonicalState>, Option<ThreadId>)>, DomainError> {
        let rows = self
            .rows(
                "SELECT state, thread_id FROM v_work_item WHERE ref = ?1",
                vec![SqlCell::Text(item_ref.into())],
            )
            .await?;
        Ok(rows.into_iter().next().map(|row| {
            let state = row
                .first()
                .and_then(text)
                .and_then(|s| serde_json::from_value(Value::String(s)).ok());
            let thread = match row.get(1) {
                Some(SqlCell::Int(t)) => Some(ThreadId::new(*t)),
                _ => None,
            };
            (state, thread)
        }))
    }

    /// Whether `descendant` sits under `ancestor` in its provider's tree.
    async fn is_under(&self, descendant: &str, ancestor: &str) -> Result<bool, DomainError> {
        let mut at = descendant.to_string();
        for _ in 0..MAX_DEPTH {
            let rows = self
                .rows(
                    "SELECT parent_ref FROM v_work_item WHERE ref = ?1",
                    vec![SqlCell::Text(at.clone())],
                )
                .await?;
            match rows.first().and_then(|r| r.first()).and_then(text) {
                Some(parent) if parent == ancestor => return Ok(true),
                Some(parent) => at = parent,
                None => return Ok(false),
            }
        }
        Ok(false)
    }

    async fn started(&self, event: &StoredEvent, item_ref: &str) -> Result<(), DomainError> {
        let thread = match event.envelope.anchors.thread_id {
            Some(t) => Some(t),
            None => self.item(item_ref).await?.and_then(|(_, t)| t),
        };
        let Some(thread) = thread else {
            return Ok(());
        };
        let open = self.open_on(thread).await?;
        let Some(open) = open else {
            return self
                .run(
                    crate::commands::effort::OPEN,
                    json!({ "thread": thread.to_string(), "work_item": item_ref }),
                )
                .await;
        };
        match open.work_item.as_deref() {
            Some(linked) if linked == item_ref => Ok(()),
            Some(linked) if self.is_under(linked, item_ref).await? => Ok(()),
            Some(linked) if !self.is_under(item_ref, linked).await? => {
                self.run(
                    crate::commands::effort::CLOSE,
                    json!({ "effort": open.id.to_string(), "reason": "switch" }),
                )
                .await?;
                self.run(
                    crate::commands::effort::OPEN,
                    json!({ "thread": thread.to_string(), "work_item": item_ref }),
                )
                .await
            }
            _ => {
                self.run(
                    crate::commands::effort::LINK,
                    json!({ "effort": open.id.to_string(), "work_item": item_ref }),
                )
                .await
            }
        }
    }

    async fn finished(&self, item_ref: &str) -> Result<(), DomainError> {
        let rows = self
            .rows(
                "SELECT id FROM v_effort WHERE work_item = ?1 AND ended_at IS NULL",
                vec![SqlCell::Text(item_ref.into())],
            )
            .await?;
        for row in rows {
            if let Some(SqlCell::Int(id)) = row.first() {
                self.run(
                    crate::commands::effort::CLOSE,
                    json!({ "effort": EffortId::new(*id).to_string(), "reason": "switch" }),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// A turn changed `thread`'s worktree: open an effort adopting the turn,
    /// unless one is open.
    async fn changed(&self, thread: ThreadId, turn: i64) -> Result<(), DomainError> {
        if self.open_on(thread).await?.is_some() {
            return Ok(());
        }
        let rows = self
            .rows(
                "SELECT started_at FROM v_agent_turn WHERE id = ?1",
                vec![SqlCell::Int(turn)],
            )
            .await?;
        let Some(started) = rows.first().and_then(|r| r.first()).and_then(text) else {
            return Ok(());
        };
        self.run(
            crate::commands::effort::OPEN,
            json!({ "thread": thread.to_string(), "adopt_since": started }),
        )
        .await
    }

    /// Work was linked to `item_ref`: a `todo` item moves to in progress.
    async fn linked(&self, item_ref: &str) -> Result<(), DomainError> {
        if let Some((Some(CanonicalState::Todo), _)) = self.item(item_ref).await? {
            self.run(
                crate::commands::work_item::NAME,
                json!({ "ref": item_ref, "to": "in_progress" }),
            )
            .await?;
        }
        Ok(())
    }
}

fn text(cell: &SqlCell) -> Option<String> {
    match cell {
        SqlCell::Text(s) => Some(s.clone()),
        _ => None,
    }
}

#[async_trait]
impl AsyncEventConsumer for EffortPolicyConsumer {
    fn name(&self) -> &'static str {
        NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == WorkItemStateChanged::TYPE
            || event_type == EffortLinked::TYPE
            || event_type == EffortOpened::TYPE
            || event_type == ThreadCheckpoint::TYPE
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        // Its own runs are its own doing: reacting to them would loop.
        if !self.active() || event.envelope.source == actor().source() {
            return Ok(());
        }
        let payload = event.envelope.payload.clone();
        match event.envelope.event_type.as_str() {
            t if t == WorkItemStateChanged::TYPE => {
                let changed: WorkItemStateChangedV1 =
                    serde_json::from_value(payload).map_err(storage)?;
                match changed.to {
                    CanonicalState::InProgress => self.started(event, &changed.work_item).await,
                    CanonicalState::Done | CanonicalState::Canceled => {
                        self.finished(&changed.work_item).await
                    }
                    CanonicalState::Todo | CanonicalState::Blocked => Ok(()),
                }
            }
            t if t == EffortLinked::TYPE => {
                let linked: EffortLinkedV1 = serde_json::from_value(payload).map_err(storage)?;
                match linked.work_item {
                    Some(item) => self.linked(&item).await,
                    None => Ok(()),
                }
            }
            t if t == ThreadCheckpoint::TYPE => {
                let checkpoint: ThreadCheckpointV1 =
                    serde_json::from_value(payload).map_err(storage)?;
                if !checkpoint.changed || checkpoint.writing_tools == 0 {
                    return Ok(());
                }
                match (
                    event.envelope.anchors.thread_id,
                    event.envelope.anchors.turn_id,
                ) {
                    (Some(thread), Some(turn)) => self.changed(thread, turn).await,
                    _ => Ok(()),
                }
            }
            _ => {
                let opened: EffortOpenedV2 = serde_json::from_value(payload).map_err(storage)?;
                match opened.work_item {
                    Some(item) => self.linked(&item).await,
                    None => Ok(()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{services_with_effort, EffortFixture};
    use oxplow_domain::refs::build::work_item_ref;
    use oxplow_domain::StreamId;

    async fn settle(fx: &EffortFixture) {
        fx.svc
            .event_pump
            .settle(&[NAME], std::time::Duration::from_secs(5))
            .await;
    }

    fn agent(fx: &EffortFixture) -> Actor {
        Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: Some(StreamId::new(1)),
        }
    }

    /// A `todo` item, on `thread` or the backlog, filed by a person.
    async fn item(fx: &EffortFixture, title: &str, parent: Option<&str>, thread: bool) -> String {
        let mut input = json!({ "title": title });
        if let Some(parent) = parent {
            input["parent_ref"] = json!(parent);
        }
        if thread {
            input["thread"] = json!(fx.thread.to_string());
        }
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                crate::commands::work_item::CREATE,
                input,
                false,
            )
            .await
            .unwrap();
        out.result["ref"].as_str().unwrap().to_string()
    }

    async fn transition(fx: &EffortFixture, actor: &Actor, item: &str, to: &str) {
        fx.svc
            .commands
            .run(
                actor,
                crate::commands::work_item::NAME,
                json!({ "ref": item, "to": to }),
                false,
            )
            .await
            .unwrap();
        settle(fx).await;
    }

    /// (effort id, work item, closed_by) of each effort on the thread, oldest
    /// first.
    async fn efforts(fx: &EffortFixture) -> Vec<(i64, Option<String>, Option<String>)> {
        fx.svc
            .sql
            .query_sql(
                "SELECT id, work_item, CASE WHEN ended_at IS NULL THEN 'open' ELSE closed_by END
                   FROM v_effort WHERE thread_id = ?1 ORDER BY id",
                vec![SqlCell::Int(fx.thread.value())],
                None,
            )
            .await
            .unwrap()
            .rows
            .into_iter()
            .map(|r| {
                (
                    match r[0] {
                        SqlCell::Int(i) => i,
                        _ => 0,
                    },
                    text(&r[1]),
                    text(&r[2]),
                )
            })
            .collect()
    }

    async fn state(fx: &EffortFixture, item: &str) -> String {
        let rows = fx
            .svc
            .sql
            .query_sql(
                "SELECT state FROM v_work_item WHERE ref = ?1",
                vec![SqlCell::Text(item.into())],
                None,
            )
            .await
            .unwrap()
            .rows;
        text(&rows[0][0]).unwrap()
    }

    async fn close_fixture_effort(fx: &EffortFixture) {
        fx.svc
            .commands
            .run(
                &Actor::Human,
                crate::commands::effort::CLOSE,
                json!({ "effort": fx.effort.to_string() }),
                false,
            )
            .await
            .unwrap();
        settle(fx).await;
    }

    /// A person starting an item filed on a thread opens that thread's
    /// effort, linked to it; an agent's start uses the agent's thread.
    #[tokio::test]
    async fn starting_an_item_opens_a_linked_effort_on_its_thread() {
        let fx = services_with_effort().await;
        close_fixture_effort(&fx).await;
        let filed = item(&fx, "filed here", None, true).await;
        transition(&fx, &Actor::Human, &filed, "in_progress").await;
        let open: Vec<_> = efforts(&fx)
            .await
            .into_iter()
            .filter(|e| e.2.as_deref() == Some("open"))
            .collect();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].1.as_deref(), Some(filed.as_str()));

        // Done closes it; the agent's start of a backlog item opens the next.
        transition(&fx, &Actor::Human, &filed, "done").await;
        let backlog = item(&fx, "backlog", None, false).await;
        transition(&fx, &agent(&fx), &backlog, "in_progress").await;
        let all = efforts(&fx).await;
        let last = all.last().unwrap();
        assert_eq!(last.1.as_deref(), Some(backlog.as_str()));
        assert_eq!(last.2.as_deref(), Some("open"));
        assert_eq!(
            all[all.len() - 2].2.as_deref(),
            Some("switch"),
            "done closed it"
        );
    }

    /// A child refines the link; its parent leaves it; an unrelated item
    /// closes the effort and opens the next.
    #[tokio::test]
    async fn a_child_refines_the_link_and_an_unrelated_item_switches() {
        let fx = services_with_effort().await;
        let parent = work_item_ref(fx.task);
        let child = item(&fx, "child", Some(&parent), true).await;
        transition(&fx, &agent(&fx), &child, "in_progress").await;
        assert_eq!(
            efforts(&fx).await,
            vec![(fx.effort.value(), Some(child.clone()), Some("open".into()))]
        );
        transition(&fx, &agent(&fx), &parent, "blocked").await;
        transition(&fx, &agent(&fx), &parent, "in_progress").await;
        assert_eq!(
            efforts(&fx).await,
            vec![(fx.effort.value(), Some(child.clone()), Some("open".into()))],
            "starting the parent again keeps the finer link"
        );
        let other = item(&fx, "other", None, false).await;
        transition(&fx, &agent(&fx), &other, "in_progress").await;
        let all = efforts(&fx).await;
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].2.as_deref(), Some("switch"));
        assert_eq!(all[1].1.as_deref(), Some(other.as_str()));
        assert_eq!(all[1].2.as_deref(), Some("open"));
    }

    /// Linking work to a `todo` item starts it, and only that: the effort
    /// stays the one linked.
    #[tokio::test]
    async fn linking_work_to_a_todo_item_starts_it() {
        let fx = services_with_effort().await;
        let todo = item(&fx, "todo", None, false).await;
        fx.svc
            .commands
            .run(
                &Actor::Human,
                crate::commands::effort::LINK,
                json!({ "effort": fx.effort.to_string(), "work_item": todo }),
                false,
            )
            .await
            .unwrap();
        settle(&fx).await;
        assert_eq!(state(&fx, &todo).await, "in_progress");
        assert_eq!(
            efforts(&fx).await,
            vec![(fx.effort.value(), Some(todo.clone()), Some("open".into()))]
        );
    }

    /// With the policy `none`, starting and finishing items leaves efforts
    /// alone.
    #[tokio::test]
    async fn the_none_policy_leaves_efforts_alone() {
        let fx = services_with_effort().await;
        fx.svc
            .config
            .write()
            .unwrap()
            .active_providers
            .insert(CAPABILITY.into(), NONE.into());
        let other = item(&fx, "other", None, false).await;
        transition(&fx, &agent(&fx), &other, "in_progress").await;
        transition(&fx, &agent(&fx), &work_item_ref(fx.task), "done").await;
        assert_eq!(
            efforts(&fx).await,
            vec![(
                fx.effort.value(),
                Some(work_item_ref(fx.task)),
                Some("open".into())
            )]
        );
    }

    /// Rule 2: a turn that changed the worktree and ran a tool that could
    /// have, on a thread with no open effort, opens an unlinked one that
    /// adopts the turn — its tool calls move into it.
    #[tokio::test]
    async fn a_turn_that_changed_the_worktree_opens_an_effort() {
        let fx = crate::thread_checkpoint::tests::with_baseline().await;
        close_fixture_effort(&fx).await;
        crate::thread_checkpoint::tests::turn(&fx, Some(("made.txt", "x")), &["Edit"]).await;
        settle(&fx).await;
        let open: Vec<_> = efforts(&fx)
            .await
            .into_iter()
            .filter(|e| e.2.as_deref() == Some("open"))
            .collect();
        assert_eq!(open.len(), 1, "{open:?}");
        assert_eq!(open[0].1, None, "unlinked");
        let adopted = fx
            .svc
            .sql
            .query_sql(
                "SELECT count(*) FROM v_tool_call WHERE tool = 'Edit' AND effort_id = ?1",
                vec![SqlCell::Int(open[0].0)],
                None,
            )
            .await
            .unwrap()
            .rows;
        assert_eq!(
            adopted[0][0],
            SqlCell::Int(1),
            "the turn's edit is the effort's"
        );
    }

    /// A turn that only read never opens one, even when the person changed
    /// the worktree meanwhile; nor does a turn that changed nothing; and an
    /// open effort stays the one.
    #[tokio::test]
    async fn reading_turns_and_open_efforts_open_nothing() {
        let fx = crate::thread_checkpoint::tests::with_baseline().await;
        close_fixture_effort(&fx).await;
        let before = efforts(&fx).await;
        crate::thread_checkpoint::tests::turn(&fx, Some(("theirs.txt", "x")), &["Read"]).await;
        crate::thread_checkpoint::tests::turn(&fx, None, &["Bash"]).await;
        settle(&fx).await;
        assert_eq!(efforts(&fx).await, before);

        let fx = crate::thread_checkpoint::tests::with_baseline().await;
        crate::thread_checkpoint::tests::turn(&fx, Some(("made.txt", "x")), &["Edit"]).await;
        settle(&fx).await;
        assert_eq!(
            efforts(&fx).await,
            vec![(
                fx.effort.value(),
                Some(work_item_ref(fx.task)),
                Some("open".into())
            )]
        );
    }
}
