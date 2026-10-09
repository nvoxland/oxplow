//! The effort policy (`.context/work-tracking.md`): when a thread's effort
//! opens, closes and links, as a reaction to core's events. A policy is an
//! [`EffortPolicy`] — it composes commands and core runs them — and the
//! project chooses one like a work list (`activeProviders.effort_policy`:
//! `oxplow`, the default; `none`, which leaves efforts to people and
//! agents; or a provider instance's id).
//!
//! [`EffortPolicyConsumer`] is the dispatcher: it offers the active policy
//! each of [`EVENTS`] and runs what it composes, in order, as the effect
//! `effort_policy:<id>` — the public `effort.*` and `work_item.*` commands,
//! with every gate they meet — stopping at the first that fails. Switching
//! the effort policy (`capability.switched`) closes every open effort
//! (`switch`): that is core's rule, whichever policy is active after.
//!
//! [`CommitOrSwitch`] is the built-in (`oxplow:commit-or-switch`). It uses
//! only what any policy could: the `v_*` models and those commands.
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
//! Rule 3, close on landing: an `effort.landed` with `complete` (a commit
//! holds all the effort's files as it left them) closes the effort as a
//! commit. A partial landing leaves it open; a reset or a branch switch
//! lands nothing, so it closes nothing. Work after the commit is the next
//! effort's (rule 2 opens it, adopting no further back than the close).

use oxplow_domain::refs::build::{effort_ref, thread_ref};
use std::sync::{Arc, RwLock, Weak};

use async_trait::async_trait;
use oxplow_config::OxplowConfig;
use oxplow_db::SqlCell;
use oxplow_domain::effort_policy::{EffortPolicy, EffortPolicyRegistry, PolicyEvent, EVENTS};
use oxplow_domain::events::schema::{
    CapabilitySwitched, CapabilitySwitchedV1, EffortLanded, EffortLandedV1, EffortLinked,
    EffortLinkedV1, EffortOpenedV2, EventType as _, ThreadCheckpoint, ThreadCheckpointV1,
    WorkItemStateChanged, WorkItemStateChangedV1,
};
use oxplow_domain::work_items::CanonicalState;
use oxplow_domain::{Actor, CommandCall, DomainError, EffortId, StoredEvent, ThreadId};
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

/// How deep a parent chain is followed before giving up (a cycle).
const MAX_DEPTH: usize = 32;

/// The built-in policy's entry (`capabilities::BUILT_INS`).
pub const BUILT_IN: &str = "oxplow:commit-or-switch";

/// The effect policy `id`'s commands run as: `effect:effort_policy:<id>`.
pub fn actor(id: &str) -> Actor {
    Actor::Effect {
        effect: format!("{CAPABILITY}:{id}"),
        thread_id: None,
        stream_id: None,
    }
}

/// Whether `source` is a policy's own run: reacting to it would loop.
fn is_a_policys(source: &str) -> bool {
    source.starts_with(&format!("effect:{CAPABILITY}:"))
}

fn storage(e: impl std::fmt::Display) -> DomainError {
    DomainError::Storage(e.to_string())
}

fn call(name: &str, input: Value) -> CommandCall {
    CommandCall {
        name: name.to_string(),
        input,
    }
}

/// What a policy is offered of `event`.
pub fn policy_event(event: &StoredEvent) -> PolicyEvent {
    let env = &event.envelope;
    PolicyEvent {
        id: env.id.to_string(),
        event_type: env.event_type.clone(),
        v: env.v,
        seq: event.seq,
        source: env.source.clone(),
        subject: env.subject.clone(),
        payload: env.payload.clone(),
        anchors: env.anchors.clone(),
    }
}

/// A thread's open effort, as `v_effort` has it.
struct Open {
    id: EffortId,
    work_item: Option<String>,
}

/// The built-in effort policy (`oxplow:commit-or-switch`): rules 1–3,
/// reading through the `v_*` models and composing `effort.*` /
/// `work_item.*` calls.
pub struct CommitOrSwitch {
    /// The id it's declared under.
    pub id: String,
    pub sql: SqlGateway,
}

impl CommitOrSwitch {
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

    async fn is_open(&self, effort: EffortId) -> Result<bool, DomainError> {
        let rows = self
            .rows(
                "SELECT 1 FROM v_effort WHERE id = ?1 AND ended_at IS NULL",
                vec![SqlCell::Int(effort.value())],
            )
            .await?;
        Ok(!rows.is_empty())
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

    async fn started(
        &self,
        event: &PolicyEvent,
        item_ref: &str,
    ) -> Result<Vec<CommandCall>, DomainError> {
        let thread = match event.anchors.thread_id {
            Some(t) => Some(t),
            None => self.item(item_ref).await?.and_then(|(_, t)| t),
        };
        let Some(thread) = thread else {
            return Ok(Vec::new());
        };
        let open_new = || {
            call(
                crate::commands::effort::OPEN,
                json!({ "thread": thread_ref(thread), "work_item": item_ref }),
            )
        };
        let Some(open) = self.open_on(thread).await? else {
            return Ok(vec![open_new()]);
        };
        Ok(match open.work_item.as_deref() {
            Some(linked) if linked == item_ref => Vec::new(),
            Some(linked) if self.is_under(linked, item_ref).await? => Vec::new(),
            Some(linked) if !self.is_under(item_ref, linked).await? => vec![
                call(
                    crate::commands::effort::CLOSE,
                    json!({ "effort": effort_ref(open.id), "reason": "switch" }),
                ),
                open_new(),
            ],
            _ => vec![call(
                crate::commands::effort::LINK,
                json!({ "effort": effort_ref(open.id), "work_item": item_ref }),
            )],
        })
    }

    async fn finished(&self, item_ref: &str) -> Result<Vec<CommandCall>, DomainError> {
        close_open(
            &self.sql,
            "SELECT id FROM v_effort WHERE work_item = ?1 AND ended_at IS NULL",
            vec![SqlCell::Text(item_ref.into())],
        )
        .await
    }

    /// A turn changed `thread`'s worktree: open an effort adopting the turn,
    /// unless one is open.
    async fn changed(&self, thread: ThreadId, turn: i64) -> Result<Vec<CommandCall>, DomainError> {
        if self.open_on(thread).await?.is_some() {
            return Ok(Vec::new());
        }
        let rows = self
            .rows(
                "SELECT started_at FROM v_agent_turn WHERE id = ?1",
                vec![SqlCell::Int(turn)],
            )
            .await?;
        let Some(started) = rows.first().and_then(|r| r.first()).and_then(text) else {
            return Ok(Vec::new());
        };
        Ok(vec![call(
            crate::commands::effort::OPEN,
            json!({ "thread": thread_ref(thread), "adopt_since": started }),
        )])
    }

    /// Work was linked to `item_ref`: a `todo` item moves to in progress.
    async fn linked(&self, item_ref: &str) -> Result<Vec<CommandCall>, DomainError> {
        Ok(match self.item(item_ref).await? {
            Some((Some(CanonicalState::Todo), _)) => vec![call(
                crate::commands::work_item::NAME,
                json!({ "ref": item_ref, "to": "in_progress" }),
            )],
            _ => Vec::new(),
        })
    }
}

#[async_trait]
impl EffortPolicy for CommitOrSwitch {
    fn id(&self) -> &str {
        &self.id
    }

    async fn react(&self, event: &PolicyEvent) -> Result<Vec<CommandCall>, DomainError> {
        let payload = event.payload.clone();
        match event.event_type.as_str() {
            t if t == WorkItemStateChanged::TYPE => {
                let changed: WorkItemStateChangedV1 =
                    serde_json::from_value(payload).map_err(storage)?;
                match changed.to {
                    CanonicalState::InProgress => self.started(event, &changed.work_item).await,
                    CanonicalState::Done | CanonicalState::Canceled => {
                        self.finished(&changed.work_item).await
                    }
                    CanonicalState::Todo | CanonicalState::Blocked => Ok(Vec::new()),
                }
            }
            t if t == EffortLinked::TYPE => {
                let linked: EffortLinkedV1 = serde_json::from_value(payload).map_err(storage)?;
                match linked.work_item {
                    Some(item) => self.linked(&item).await,
                    None => Ok(Vec::new()),
                }
            }
            t if t == EffortLanded::TYPE => {
                let landed: EffortLandedV1 = serde_json::from_value(payload).map_err(storage)?;
                if !landed.complete {
                    return Ok(Vec::new());
                }
                match event.anchors.effort_id {
                    Some(effort) if self.is_open(effort).await? => Ok(vec![call(
                        crate::commands::effort::CLOSE,
                        json!({ "effort": effort_ref(effort), "reason": "commit" }),
                    )]),
                    _ => Ok(Vec::new()),
                }
            }
            t if t == ThreadCheckpoint::TYPE => {
                let checkpoint: ThreadCheckpointV1 =
                    serde_json::from_value(payload).map_err(storage)?;
                if !checkpoint.changed || checkpoint.writing_tools == 0 {
                    return Ok(Vec::new());
                }
                match (event.anchors.thread_id, event.anchors.turn_id) {
                    (Some(thread), Some(turn)) => self.changed(thread, turn).await,
                    _ => Ok(Vec::new()),
                }
            }
            _ => {
                let opened: EffortOpenedV2 = serde_json::from_value(payload).map_err(storage)?;
                match opened.work_item {
                    Some(item) => self.linked(&item).await,
                    None => Ok(Vec::new()),
                }
            }
        }
    }
}

/// Close calls (`switch`) for the open efforts `sql` selects the ids of.
async fn close_open(
    gateway: &SqlGateway,
    sql: &str,
    params: Vec<SqlCell>,
) -> Result<Vec<CommandCall>, DomainError> {
    Ok(gateway
        .query_sql(sql, params, None)
        .await?
        .rows
        .into_iter()
        .filter_map(|row| match row.first() {
            Some(SqlCell::Int(id)) => Some(call(
                crate::commands::effort::CLOSE,
                json!({ "effort": effort_ref(EffortId::new(*id)), "reason": "switch" }),
            )),
            _ => None,
        })
        .collect())
}

fn text(cell: &SqlCell) -> Option<String> {
    match cell {
        SqlCell::Text(s) => Some(s.clone()),
        _ => None,
    }
}

/// Register the policies the extensions declare (`implementations:`)
/// under their declared ids, replacing the ones declared before: the
/// built-in ([`BUILT_IN`]) and policies written as scripts
/// (`effort_policy_script::ScriptedPolicy`, each gated on a person's
/// approval as it is now). A running provider instance stays registered.
pub fn register_built_ins(
    registry: &EffortPolicyRegistry,
    declared: &[crate::capabilities::Implementation],
    sql: &SqlGateway,
    approvals: &Arc<crate::exec_consent::ApprovalStore>,
    project_dir: &std::path::Path,
) {
    use crate::capabilities::Source;
    let built: Vec<Arc<dyn EffortPolicy>> = declared
        .iter()
        .filter(|i| i.capability == CAPABILITY)
        .filter_map(|i| -> Option<Arc<dyn EffortPolicy>> {
            match &i.source {
                Source::BuiltIn(BUILT_IN) => Some(Arc::new(CommitOrSwitch {
                    id: i.id.clone(),
                    sql: sql.clone(),
                })),
                Source::Script {
                    entry,
                    tree,
                    script,
                    needs,
                } => {
                    let program = crate::exec_consent::effort_policy_program(
                        tree,
                        i.extension.as_deref().unwrap_or_default(),
                        &i.id,
                        entry,
                        needs,
                    );
                    Some(Arc::new(crate::effort_policy_script::ScriptedPolicy {
                        id: i.id.clone(),
                        script: script.as_str().into(),
                        needs: needs.clone(),
                        sql: sql.clone(),
                        gate: crate::exec_consent::script_gate(
                            approvals.clone(),
                            project_dir.to_path_buf(),
                            program,
                        ),
                    }))
                }
                _ => None,
            }
        })
        .collect();
    registry.set_declared(built);
}

/// The dispatcher: offers the active effort policy each of [`EVENTS`] and
/// runs what it composes; closes every open effort when the effort policy
/// is switched.
pub struct EffortPolicyConsumer {
    pub bus: Weak<CommandBus>,
    pub sql: SqlGateway,
    pub config: Arc<RwLock<OxplowConfig>>,
    pub capabilities: Arc<crate::capabilities::CapabilityRegistry>,
    pub policies: Arc<EffortPolicyRegistry>,
}

impl EffortPolicyConsumer {
    fn bus(&self) -> Result<Arc<CommandBus>, DomainError> {
        self.bus
            .upgrade()
            .ok_or_else(|| DomainError::Invariant("the command bus is gone".into()))
    }

    /// Run `calls`, in order, as `actor`, stopping at the first that fails.
    async fn run(&self, actor: &Actor, calls: Vec<CommandCall>) -> Result<(), DomainError> {
        let bus = self.bus()?;
        for c in calls {
            bus.run(actor, &c.name, c.input, false)
                .await
                .map_err(|e| storage(format!("{}: {e}", c.name)))?;
        }
        Ok(())
    }
}

#[async_trait]
impl AsyncEventConsumer for EffortPolicyConsumer {
    fn name(&self) -> &'static str {
        NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == CapabilitySwitched::TYPE || EVENTS.iter().any(|(t, _)| *t == event_type)
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        // A switch of the effort policy closes what the policy before it
        // opened — core's rule, whichever is active now.
        if event.envelope.event_type == CapabilitySwitched::TYPE {
            let switched: CapabilitySwitchedV1 =
                serde_json::from_value(event.envelope.payload.clone()).map_err(storage)?;
            if switched.capability != CAPABILITY {
                return Ok(());
            }
            let calls = close_open(
                &self.sql,
                "SELECT id FROM v_effort WHERE ended_at IS NULL",
                vec![],
            )
            .await?;
            return self.run(&Actor::System, calls).await;
        }
        // A policy's own runs are its own doing: reacting to them would loop.
        if is_a_policys(&event.envelope.source) {
            return Ok(());
        }
        let config = crate::config_service::read_config(&self.config);
        let id = self.capabilities.active(&config, CAPABILITY);
        if id == NONE {
            return Ok(());
        }
        let Some(policy) = self.policies.get(&id) else {
            return Ok(());
        };
        let calls = policy.react(&policy_event(event)).await?;
        if calls.is_empty() {
            return Ok(());
        }
        let bus = self.bus()?;
        crate::extension_commands::check_calls(&*bus, &calls)
            .map_err(|e| storage(format!("effort policy `{id}`: {e}")))?;
        self.run(&actor(&id), calls).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{services_with_effort, EffortFixture};
    use oxplow_domain::StreamId;
    use oxplow_tasks::work_item_ref;

    async fn settle(fx: &EffortFixture) {
        fx.svc
            .event_pump
            .settle(&[NAME], std::time::Duration::from_secs(5))
            .await;
    }

    fn agent(fx: &EffortFixture) -> Actor {
        Actor::Agent {
            session_id: None,
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
                json!({ "effort": effort_ref(fx.effort) }),
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
        let fx = crate::test_fixtures::services_with_task_effort().await;
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
                json!({ "effort": effort_ref(fx.effort), "work_item": todo }),
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

    /// A policy that records what it was offered and composes `calls`
    /// for an item started — a provider instance's stand-in.
    struct Recorder {
        id: &'static str,
        seen: std::sync::Mutex<Vec<String>>,
        calls: Vec<CommandCall>,
    }

    #[async_trait]
    impl EffortPolicy for Recorder {
        fn id(&self) -> &str {
            self.id
        }
        async fn react(&self, event: &PolicyEvent) -> Result<Vec<CommandCall>, DomainError> {
            self.seen.lock().unwrap().push(event.event_type.clone());
            Ok(if event.payload["to"] == "in_progress" {
                self.calls.clone()
            } else {
                Vec::new()
            })
        }
    }

    /// Make `id` an effort policy implementation that runs, and the
    /// project's choice.
    fn choose(fx: &EffortFixture, id: &str) {
        fx.svc.capabilities.set_external(
            crate::capabilities::Implementation {
                capability: CAPABILITY.into(),
                id: id.into(),
                title: id.into(),
                extension: Some("x".into()),
                source: crate::capabilities::Source::External,
                features: Value::Null,
                fields: Value::Array(Vec::new()),
                id_pattern: None,
                config: json!({}),
            },
            true,
        );
        fx.svc
            .config
            .write()
            .unwrap()
            .active_providers
            .insert(CAPABILITY.into(), id.into());
    }

    /// What the built-in composes runs as its own effect,
    /// `effect:effort_policy:<id>` — what it logs says which policy did it.
    #[tokio::test]
    async fn a_policys_commands_run_as_its_own_effect() {
        let fx = services_with_effort().await;
        close_fixture_effort(&fx).await;
        let filed = item(&fx, "filed here", None, true).await;
        transition(&fx, &Actor::Human, &filed, "in_progress").await;
        let actors: Vec<String> = fx
            .svc
            .db
            .read(|c| {
                let mut st = c
                    .prepare("SELECT source FROM event_log WHERE type = 'effort.opened'")
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = st
                    .query_map([], |r| r.get(0))
                    .map_err(oxplow_db::map_sql_err)?;
                rows.collect::<rusqlite::Result<_>>()
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        // The fixture's own effort was a person's; the next, the policy's.
        assert_eq!(actors.last().unwrap(), "effect:effort_policy:oxplow");
    }

    /// The dispatcher offers events to the active policy only — a
    /// registered one that isn't chosen hears nothing — and runs what it
    /// composes as its effect. A catalog reload restates the declared
    /// built-ins and keeps it registered.
    #[tokio::test]
    async fn only_the_active_policy_is_offered_events_and_a_reload_keeps_an_instance() {
        let fx = services_with_effort().await;
        close_fixture_effort(&fx).await;
        let rec = Arc::new(Recorder {
            id: "rec",
            seen: std::sync::Mutex::default(),
            calls: vec![call(
                crate::commands::effort::OPEN,
                json!({ "thread": thread_ref(fx.thread) }),
            )],
        });
        fx.svc.effort_policies.register(rec.clone());
        let first = item(&fx, "first", None, true).await;
        transition(&fx, &Actor::Human, &first, "in_progress").await;
        assert!(
            rec.seen.lock().unwrap().is_empty(),
            "oxplow is the active one"
        );
        transition(&fx, &Actor::Human, &first, "done").await;

        choose(&fx, "rec");
        crate::capabilities::refresh(&fx.svc).await.unwrap();
        assert!(
            fx.svc.effort_policies.has("rec"),
            "a reload keeps an instance"
        );
        assert!(fx.svc.effort_policies.has("oxplow"));
        let second = item(&fx, "second", None, true).await;
        transition(&fx, &Actor::Human, &second, "in_progress").await;
        // Filing the item and starting it.
        assert_eq!(
            *rec.seen.lock().unwrap(),
            vec!["work_item.state_changed", "work_item.state_changed"]
        );
        let open: Vec<_> = efforts(&fx)
            .await
            .into_iter()
            .filter(|e| e.2.as_deref() == Some("open"))
            .collect();
        assert_eq!(open.len(), 1, "rec opened one, unlinked");
        assert_eq!(open[0].1, None);
    }

    /// A composed call that isn't a command fails the run before anything
    /// runs: the item's state stands and no effort opens.
    #[tokio::test]
    async fn a_call_that_isnt_a_command_runs_nothing() {
        let fx = services_with_effort().await;
        close_fixture_effort(&fx).await;
        fx.svc.effort_policies.register(Arc::new(Recorder {
            id: "rec",
            seen: std::sync::Mutex::default(),
            calls: vec![
                call(
                    crate::commands::effort::OPEN,
                    json!({ "thread": thread_ref(fx.thread) }),
                ),
                call("nope.no.cmd", json!({})),
            ],
        }));
        choose(&fx, "rec");
        let filed = item(&fx, "filed", None, true).await;
        transition(&fx, &Actor::Human, &filed, "in_progress").await;
        assert_eq!(state(&fx, &filed).await, "in_progress");
        assert!(efforts(&fx)
            .await
            .iter()
            .all(|e| e.2.as_deref() != Some("open")));
    }

    /// With the policy `none`, starting and finishing items leaves efforts
    /// alone.
    #[tokio::test]
    async fn the_none_policy_leaves_efforts_alone() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
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

    /// Switching the effort policy closes the open efforts (`switch`): the
    /// policy they were opened under is no longer the one.
    #[tokio::test]
    async fn switching_the_policy_closes_the_open_efforts() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let mut config = crate::config_service::read_config(&fx.svc.config);
        fx.svc
            .capabilities
            .publish(&config, &fx.svc.db)
            .await
            .unwrap();
        config
            .active_providers
            .insert(CAPABILITY.into(), NONE.into());
        fx.svc
            .capabilities
            .publish(&config, &fx.svc.db)
            .await
            .unwrap();
        settle(&fx).await;
        assert_eq!(
            efforts(&fx).await,
            vec![(
                fx.effort.value(),
                Some(work_item_ref(fx.task)),
                Some("switch".into())
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
        crate::thread_checkpoint::tests::turn(&fx, Some(("made.txt", "x")), &["edit"]).await;
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
                "SELECT count(*) FROM v_tool_call WHERE kind = 'edit' AND effort_id = ?1",
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
        crate::thread_checkpoint::tests::turn(&fx, Some(("theirs.txt", "x")), &["read"]).await;
        crate::thread_checkpoint::tests::turn(&fx, None, &["shell"]).await;
        settle(&fx).await;
        assert_eq!(efforts(&fx).await, before);

        let fx = crate::thread_checkpoint::tests::with_baseline().await;
        crate::thread_checkpoint::tests::turn(&fx, Some(("made.txt", "x")), &["edit"]).await;
        settle(&fx).await;
        assert_eq!(
            efforts(&fx).await,
            vec![(fx.effort.value(), None, Some("open".into()))]
        );
    }
}
