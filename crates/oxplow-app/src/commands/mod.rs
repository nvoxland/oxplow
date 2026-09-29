//! The command bus: the one write path (`.context/commands.md`;
//! `.context/target-architecture.md` §7).
//!
//! Every command runs the same pipeline — validate the input against the
//! spec's schema, check the invoker, apply the agent policy, require a
//! person's confirmation when the spec says so, run the handler, and
//! record `command.executed` in the event log plus a `command_audit` row
//! **in one transaction**. `undo(audit_id)` runs the recorded inverse
//! through the same pipeline.
//!
//! A handler is either `Tx` (runs inside the bus's transaction, so its
//! writes, the event and the audit commit together) or `BestEffort` (a
//! pre-existing service call with its own transactions, audited after
//! it returns). `BestEffort` exists only for handlers that predate the
//! bus; [`CommandBus::best_effort_count`] is asserted by a test so the
//! number trends to zero.

pub mod config_commands;
pub mod work_item;

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use oxplow_db::command_audit_store::{insert_tx, mark_undone_tx, set_event_id_tx, NewCommandAudit};
use oxplow_db::event_log_store::append_tx;
use oxplow_db::{CommandAudit, Database, SqliteCommandAuditStore, SqliteEventLogStore};
use oxplow_domain::events::schema::{
    CommandExecuted, CommandExecutedV1, CommandOutcome as Outcome,
};
use oxplow_domain::{
    Actor, Anchors, Atomicity, CommandCall, CommandError, CommandOutcome, CommandSpec, Envelope,
    InputValidator, Preview,
};
use oxplow_runtime::policy::PolicyDecision;
use parking_lot::RwLock;
use serde_json::Value;

use crate::agent_policy::AgentPolicy;
use crate::event_pump::EventPump;

/// What a handler produced.
#[derive(Default)]
pub struct HandlerOutput {
    /// Returned to the caller (`run_command`'s result).
    pub result: Value,
    /// The call that undoes this run; `None` for a command that declares
    /// `undoable: false`, or when there is nothing to undo.
    pub inverse: Option<CommandCall>,
    /// Domain events the run produced, appended after `command.executed`
    /// in the same transaction. A `BestEffort` handler's own transactions
    /// log their events themselves; this is for `Tx` handlers.
    pub events: Vec<Envelope>,
    /// Runs once the run has committed — the place for the in-memory
    /// broadcast that wakes the UI (`OxplowEvent`), never for a write.
    pub after_commit: Option<Box<dyn FnOnce() + Send + Sync>>,
}

type TxHandler = dyn Fn(&rusqlite::Connection, &Actor, Value) -> Result<HandlerOutput, CommandError>
    + Send
    + Sync;
type BestEffortFuture = Pin<Box<dyn Future<Output = Result<HandlerOutput, CommandError>> + Send>>;
type BestEffortHandler = dyn Fn(Actor, Value) -> BestEffortFuture + Send + Sync;

pub enum Handler {
    /// Runs inside the bus's transaction.
    Tx(Arc<TxHandler>),
    /// A pre-existing service call; the bus audits it after it returns.
    BestEffort(Arc<BestEffortHandler>),
}

type ConfirmFor = dyn Fn(&Value) -> oxplow_domain::Confirm + Send + Sync;

/// A registered command: its spec, compiled input schema and handler.
pub struct Command {
    pub spec: CommandSpec,
    pub handler: Handler,
    validator: InputValidator,
    /// Per-input confirmation, when the spec's `confirm` depends on the
    /// input (`config.set` on a human-only key). Overrides `spec.confirm`.
    confirm_for: Option<Arc<ConfirmFor>>,
}

impl Command {
    pub fn new(spec: CommandSpec, handler: Handler) -> Result<Self, CommandError> {
        CommandSpec::validate_name(&spec.name)?;
        let declared = match handler {
            Handler::Tx(_) => Atomicity::Tx,
            Handler::BestEffort(_) => Atomicity::BestEffort,
        };
        if declared != spec.atomicity {
            return Err(CommandError::Invalid {
                field: Some("/atomicity".into()),
                message: format!(
                    "`{}` declares {:?} but its handler is {:?}",
                    spec.name, spec.atomicity, declared
                ),
            });
        }
        let validator = InputValidator::compile(&spec.input_schema)?;
        Ok(Self {
            spec,
            handler,
            validator,
            confirm_for: None,
        })
    }

    pub fn with_confirm_for(mut self, f: Arc<ConfirmFor>) -> Self {
        self.confirm_for = Some(f);
        self
    }

    fn confirm(&self, input: &Value) -> oxplow_domain::Confirm {
        match &self.confirm_for {
            Some(f) => f(input),
            None => self.spec.confirm,
        }
    }
}

pub struct CommandBus {
    db: Database,
    log: SqliteEventLogStore,
    audit: SqliteCommandAuditStore,
    policy: Arc<AgentPolicy>,
    pump: Arc<EventPump>,
    commands: RwLock<BTreeMap<String, Arc<Command>>>,
}

impl CommandBus {
    pub fn new(
        db: Database,
        log: SqliteEventLogStore,
        policy: Arc<AgentPolicy>,
        pump: Arc<EventPump>,
    ) -> Self {
        Self {
            audit: SqliteCommandAuditStore::new(db.clone()),
            db,
            log,
            policy,
            pump,
            commands: RwLock::new(BTreeMap::new()),
        }
    }

    /// Register a command. A second command of the same name is refused:
    /// two handlers for one name is a bug, not an override.
    pub fn register(&self, command: Command) -> Result<(), CommandError> {
        let mut commands = self.commands.write();
        if commands.contains_key(&command.spec.name) {
            return Err(CommandError::Invalid {
                field: Some("/name".into()),
                message: format!("command `{}` is already registered", command.spec.name),
            });
        }
        commands.insert(command.spec.name.clone(), Arc::new(command));
        Ok(())
    }

    pub fn audit_store(&self) -> &SqliteCommandAuditStore {
        &self.audit
    }

    /// The specs `actor` may invoke, by name.
    pub fn list(&self, actor: &Actor) -> Vec<CommandSpec> {
        self.commands
            .read()
            .values()
            .filter(|c| c.spec.invokers.allows(actor.invoker()))
            .map(|c| c.spec.clone())
            .collect()
    }

    pub fn spec(&self, name: &str) -> Option<CommandSpec> {
        self.commands.read().get(name).map(|c| c.spec.clone())
    }

    /// The log the bus records into (tests read it back).
    pub fn log_for_tests(&self) -> &SqliteEventLogStore {
        &self.log
    }

    /// How many registered handlers are `BestEffort`. Trends to zero.
    pub fn best_effort_count(&self) -> usize {
        self.commands
            .read()
            .values()
            .filter(|c| matches!(c.handler, Handler::BestEffort(_)))
            .count()
    }

    /// Run `name` with `input` as `actor`. `confirmed` says a person has
    /// already confirmed this exact call; an agent's `confirmed` is
    /// ignored — an agent can never confirm.
    pub async fn run(
        &self,
        actor: &Actor,
        name: &str,
        input: Value,
        confirmed: bool,
    ) -> Result<CommandOutcome, CommandError> {
        let command =
            self.commands
                .read()
                .get(name)
                .cloned()
                .ok_or_else(|| CommandError::Unknown {
                    name: name.to_string(),
                })?;
        let spec = &command.spec;

        // 1. The input must match the schema.
        if let Err(err) = command.validator.check(&input) {
            self.audit_only(actor, spec, &input, Outcome::Invalid, Some(err.to_string()))
                .await;
            return Err(err);
        }
        // 2. The invoker must be admitted…
        if !spec.invokers.allows(actor.invoker()) {
            let err = CommandError::Denied {
                reason: format!(
                    "`{}` is not open to {:?} callers",
                    spec.name,
                    actor.invoker()
                ),
            };
            self.audit_only(actor, spec, &input, Outcome::Denied, Some(err.to_string()))
                .await;
            return Err(err);
        }
        // 3. …and an agent must also pass the agent policy.
        if let Actor::Agent { thread_id, .. } = actor {
            if let PolicyDecision::Deny { reason, .. } =
                self.policy.check_command(thread_id.as_ref(), spec)
            {
                let err = CommandError::Denied { reason };
                self.audit_only(actor, spec, &input, Outcome::Denied, Some(err.to_string()))
                    .await;
                return Err(err);
            }
        }
        // 4. A person confirms; an agent never can. Nothing is written.
        let confirmed = confirmed && !matches!(actor, Actor::Agent { .. });
        let confirm = command.confirm(&input);
        if confirm.required() && !confirmed {
            return Err(CommandError::NeedsConfirmation {
                preview: Box::new(Preview {
                    command: spec.name.clone(),
                    summary: spec.summary.clone(),
                    input,
                    destructive: matches!(confirm, oxplow_domain::Confirm::Destructive),
                }),
            });
        }
        // 5. Run, and record the run with its event in one transaction.
        let outcome = match &command.handler {
            Handler::Tx(handler) => {
                let handler = handler.clone();
                let actor_c = actor.clone();
                let spec_c = spec.clone();
                let input_c = input.clone();
                let schemas = self.log.schemas().clone();
                // A handler error must roll the transaction back, which
                // means returning `Err` from the closure; the structured
                // `CommandError` rides out through this slot.
                let failed: Arc<parking_lot::Mutex<Option<CommandError>>> = Arc::default();
                let failed_c = failed.clone();
                let ran = self
                    .db
                    .transaction(move |tx| {
                        let out = match handler(tx, &actor_c, input_c.clone()) {
                            Ok(out) => out,
                            Err(err) => {
                                *failed_c.lock() = Some(err);
                                return Err(oxplow_domain::DomainError::Invariant(
                                    "command handler failed; rolled back".into(),
                                ));
                            }
                        };
                        let recorded = record_tx(tx, &schemas, &actor_c, &spec_c, &input_c, &out)?;
                        Ok((out, recorded))
                    })
                    .await;
                match ran {
                    Ok((out, recorded)) => Ok(finish(out, recorded)),
                    Err(db_err) => Err(failed
                        .lock()
                        .take()
                        .unwrap_or_else(|| CommandError::from(db_err))),
                }
            }
            Handler::BestEffort(handler) => match handler(actor.clone(), input.clone()).await {
                Ok(out) => {
                    let recorded = self.record(actor, spec, &input, &out).await?;
                    Ok(finish(out, recorded))
                }
                Err(err) => Err(err),
            },
        };
        match outcome {
            Ok(done) => {
                self.pump.wake();
                Ok(done)
            }
            Err(err) => {
                self.audit_only(actor, spec, &input, Outcome::Error, Some(err.to_string()))
                    .await;
                Err(err)
            }
        }
    }

    /// Apply the inverse recorded for `audit_id`, as `actor`, through the
    /// normal pipeline (policy, confirmation and audit included). The
    /// original row is marked `undone_by` the new run.
    pub async fn undo(
        &self,
        actor: &Actor,
        audit_id: i64,
        confirmed: bool,
    ) -> Result<CommandOutcome, CommandError> {
        let row = self
            .audit
            .get(audit_id)
            .await?
            .ok_or_else(|| CommandError::Failed {
                message: format!("audit row {audit_id} not found"),
            })?;
        let inverse = undoable(&row)?;
        let outcome = self
            .run(actor, &inverse.name, inverse.input.clone(), confirmed)
            .await?;
        let done_by = outcome.audit_id;
        self.db
            .transaction(move |tx| mark_undone_tx(tx, audit_id, done_by))
            .await
            .map_err(CommandError::from)?;
        Ok(outcome)
    }

    /// Audit a run that wrote nothing (invalid input, a denied invoker, a
    /// failed handler). Best effort: an audit failure must not mask the
    /// error being reported.
    async fn audit_only(
        &self,
        actor: &Actor,
        spec: &CommandSpec,
        input: &Value,
        outcome: Outcome,
        error: Option<String>,
    ) {
        let row = NewCommandAudit {
            command: spec.name.clone(),
            actor_kind: actor.kind(),
            actor_id: actor.id(),
            thread_id: actor.thread_id(),
            input: input.clone(),
            outcome,
            error,
            inverse: None,
        };
        if let Err(e) = self
            .db
            .transaction(move |tx| insert_tx(tx, &row).map(|_| ()))
            .await
        {
            tracing::warn!(error = %e, command = %spec.name, "command audit write failed");
        }
    }

    /// The audit row and `command.executed` for a `BestEffort` run.
    async fn record(
        &self,
        actor: &Actor,
        spec: &CommandSpec,
        input: &Value,
        out: &HandlerOutput,
    ) -> Result<Recorded, CommandError> {
        let (actor, spec, input) = (actor.clone(), spec.clone(), input.clone());
        let inverse = out.inverse.clone();
        let events = out.events.clone();
        let schemas = self.log.schemas().clone();
        let shadow = HandlerOutput {
            result: Value::Null,
            inverse,
            events,
            after_commit: None,
        };
        self.db
            .transaction(move |tx| record_tx(tx, &schemas, &actor, &spec, &input, &shadow))
            .await
            .map_err(CommandError::from)
    }
}

/// What recording a successful run produced.
struct Recorded {
    audit_id: i64,
    event_id: oxplow_domain::EventId,
}

fn finish(mut out: HandlerOutput, recorded: Recorded) -> CommandOutcome {
    if let Some(after) = out.after_commit.take() {
        after();
    }
    CommandOutcome {
        result: out.result,
        audit_id: recorded.audit_id,
        event_id: recorded.event_id,
        inverse: out.inverse,
    }
}

/// Inside the run's transaction: the audit row, `command.executed`
/// pointing at it, the handler's domain events, and the row's event id.
fn record_tx(
    tx: &rusqlite::Connection,
    schemas: &oxplow_domain::EventSchemaRegistry,
    actor: &Actor,
    spec: &CommandSpec,
    input: &Value,
    out: &HandlerOutput,
) -> Result<Recorded, oxplow_domain::DomainError> {
    let inverse = if spec.undoable {
        out.inverse.clone()
    } else {
        None
    };
    let audit_id = insert_tx(
        tx,
        &NewCommandAudit {
            command: spec.name.clone(),
            actor_kind: actor.kind(),
            actor_id: actor.id(),
            thread_id: actor.thread_id(),
            input: input.clone(),
            outcome: Outcome::Ok,
            error: None,
            inverse: inverse.clone(),
        },
    )?;
    let executed = Envelope::typed::<CommandExecuted>(
        actor.source(),
        &CommandExecutedV1 {
            command: spec.name.clone(),
            actor_kind: actor.kind(),
            actor_id: actor.id(),
            outcome: Outcome::Ok,
            audit_id,
            undoable: inverse.is_some(),
        },
    )
    .with_anchors(Anchors {
        thread_id: actor.thread_id(),
        stream_id: match actor {
            Actor::Agent { stream_id, .. } => *stream_id,
            _ => None,
        },
        ..Anchors::default()
    })
    .with_subject([format!("command:{}", spec.name)]);
    append_tx(tx, schemas, &executed)?;
    for event in &out.events {
        let event = event.clone().with_cause(executed.id.clone());
        append_tx(tx, schemas, &event)?;
    }
    set_event_id_tx(tx, audit_id, &executed.id)?;
    Ok(Recorded {
        audit_id,
        event_id: executed.id,
    })
}

/// The inverse of an audited run, if it can still be applied.
fn undoable(row: &CommandAudit) -> Result<CommandCall, CommandError> {
    if row.outcome != Outcome::Ok {
        return Err(CommandError::Failed {
            message: format!("audit row {} did not complete; nothing to undo", row.id),
        });
    }
    if let Some(by) = row.undone_by {
        return Err(CommandError::Failed {
            message: format!("audit row {} was already undone by {by}", row.id),
        });
    }
    row.inverse.clone().ok_or_else(|| CommandError::Failed {
        message: format!("`{}` (audit {}) is not undoable", row.command, row.id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::events::schema::{ConfigChanged, ConfigChangedV1};
    use oxplow_domain::{Confirm, EventSchemaRegistry, Invokers, Lifecycle, ThreadId};
    use serde_json::json;

    fn bus() -> (Database, CommandBus) {
        let db = Database::in_memory();
        let log = SqliteEventLogStore::new(db.clone(), Arc::new(EventSchemaRegistry::core()));
        let pump = Arc::new(EventPump::new(db.clone(), log.clone(), vec![]));
        let bus = CommandBus::new(db.clone(), log, Arc::new(AgentPolicy::default()), pump);
        db.clone()
            .transaction(|tx| {
                tx.execute_batch("CREATE TABLE kv (k TEXT PRIMARY KEY, v TEXT NOT NULL)")
                    .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .now_or_never_ok();
        (db, bus)
    }

    trait NowOrNever {
        fn now_or_never_ok(self);
    }
    impl<F: Future<Output = Result<(), oxplow_domain::DomainError>>> NowOrNever for F {
        fn now_or_never_ok(self) {
            let rt = tokio::runtime::Handle::current();
            tokio::task::block_in_place(|| rt.block_on(self)).unwrap();
        }
    }

    fn kv_spec(name: &str, invokers: Invokers, confirm: Confirm) -> CommandSpec {
        CommandSpec {
            name: name.into(),
            summary: "Set a key in the test table.".into(),
            input_schema: json!({
                "type": "object",
                "required": ["k", "v"],
                "properties": { "k": { "type": "string" }, "v": { "type": "string" } },
                "additionalProperties": false
            }),
            invokers,
            confirm,
            undoable: true,
            lifecycle: Lifecycle::Stable,
            atomicity: Atomicity::Tx,
        }
    }

    /// A `Tx` handler: writes `k = v`, returns the inverse (restore the
    /// prior value) and a `config.changed` domain event.
    fn kv_set() -> Handler {
        Handler::Tx(Arc::new(|conn, _actor, input| {
            let k = input["k"].as_str().unwrap_or_default().to_string();
            let v = input["v"].as_str().unwrap_or_default().to_string();
            if v == "boom" {
                return Err(CommandError::Failed {
                    message: "refused value".into(),
                });
            }
            let before: Option<String> = conn
                .query_row("SELECT v FROM kv WHERE k = ?1", [&k], |r| r.get(0))
                .ok();
            conn.execute(
                "INSERT INTO kv (k, v) VALUES (?1, ?2) ON CONFLICT (k) DO UPDATE SET v = excluded.v",
                [&k, &v],
            )
            .map_err(|e| CommandError::Failed {
                message: e.to_string(),
            })?;
            if v == "half" {
                // A write happened, then the handler fails: it must roll back.
                return Err(CommandError::Failed {
                    message: "failed after writing".into(),
                });
            }
            Ok(HandlerOutput {
                result: json!({ "k": k, "v": v }),
                inverse: Some(CommandCall {
                    name: "kv.set".into(),
                    input: json!({ "k": k, "v": before.clone().unwrap_or_default() }),
                }),
                events: vec![Envelope::typed::<ConfigChanged>(
                    "test",
                    &ConfigChangedV1 {
                        key: k.clone(),
                        before: before.map(Value::String).unwrap_or(Value::Null),
                        after: Value::String(v),
                    },
                )],
                after_commit: None,
            })
        }))
    }

    async fn kv(db: &Database, k: &str) -> Option<String> {
        let k = k.to_string();
        db.transaction(move |tx| {
            Ok(tx
                .query_row("SELECT v FROM kv WHERE k = ?1", [&k], |r| {
                    r.get::<_, String>(0)
                })
                .ok())
        })
        .await
        .unwrap()
    }

    fn agent() -> Actor {
        Actor::Agent {
            thread_id: Some(ThreadId::new(7)),
            stream_id: None,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_tx_command_writes_its_state_audit_and_events_together() {
        let (db, bus) = bus();
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Never), kv_set()).unwrap(),
        )
        .unwrap();
        let out = bus
            .run(&agent(), "kv.set", json!({"k": "a", "v": "1"}), false)
            .await
            .unwrap();
        assert_eq!(out.result, json!({"k": "a", "v": "1"}));
        assert_eq!(kv(&db, "a").await.as_deref(), Some("1"));
        let audit = bus.audit_store().get(out.audit_id).await.unwrap().unwrap();
        assert_eq!(audit.command, "kv.set");
        assert_eq!(audit.thread_id, Some(ThreadId::new(7)));
        assert_eq!(audit.outcome, Outcome::Ok);
        assert_eq!(audit.event_id, Some(out.event_id.clone()));
        assert_eq!(
            audit.inverse.as_ref().unwrap().input,
            json!({"k": "a", "v": ""})
        );
        // command.executed then the handler's domain event, caused by it.
        let events = bus.log.read_after(0, 10).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].envelope.event_type, "command.executed");
        assert_eq!(events[0].envelope.payload["audit_id"], out.audit_id);
        assert_eq!(events[0].envelope.payload["actor_kind"], "agent");
        assert_eq!(events[0].envelope.source, "agent:thr7");
        assert_eq!(events[0].envelope.subject, vec!["command:kv.set"]);
        assert_eq!(events[1].envelope.event_type, "config.changed");
        assert_eq!(events[1].envelope.cause, Some(out.event_id));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failing_tx_handler_rolls_everything_back_and_is_audited_as_error() {
        let (db, bus) = bus();
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Never), kv_set()).unwrap(),
        )
        .unwrap();
        let err = bus
            .run(
                &Actor::Human,
                "kv.set",
                json!({"k": "a", "v": "half"}),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Failed { .. }), "{err:?}");
        assert_eq!(kv(&db, "a").await, None, "the handler's write rolled back");
        assert!(bus.log.read_after(0, 10).await.unwrap().is_empty());
        let recent = bus.audit_store().list_recent(5).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].outcome, Outcome::Error);
        assert!(recent[0]
            .error
            .as_deref()
            .unwrap()
            .contains("failed after writing"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn schema_rejection_names_the_field_and_is_audited_invalid() {
        let (_db, bus) = bus();
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Never), kv_set()).unwrap(),
        )
        .unwrap();
        let err = bus
            .run(&Actor::Human, "kv.set", json!({"k": "a", "v": 3}), false)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/v"),
            "{err:?}"
        );
        assert_eq!(
            bus.audit_store().list_recent(1).await.unwrap()[0].outcome,
            Outcome::Invalid
        );
        assert!(matches!(
            bus.run(&Actor::Human, "kv.nope", json!({}), false)
                .await
                .unwrap_err(),
            CommandError::Unknown { .. }
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn invokers_and_agent_policy_deny_before_anything_runs() {
        let (db, bus) = bus();
        bus.register(
            Command::new(
                kv_spec("kv.set", Invokers::HUMAN_ONLY, Confirm::Never),
                kv_set(),
            )
            .unwrap(),
        )
        .unwrap();
        let lens = Actor::Lens {
            lens_id: "acme/x".into(),
            on_behalf_of: Box::new(Actor::Human),
        };
        let err = bus
            .run(&lens, "kv.set", json!({"k": "a", "v": "1"}), false)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        let err = bus
            .run(&agent(), "kv.set", json!({"k": "a", "v": "1"}), false)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        assert_eq!(kv(&db, "a").await, None);
        let recent = bus.audit_store().list_recent(5).await.unwrap();
        assert_eq!(recent.len(), 2);
        assert!(recent.iter().all(|r| r.outcome == Outcome::Denied));
        // `list` shows each actor only what it may run.
        assert!(bus.list(&lens).is_empty());
        assert_eq!(bus.list(&Actor::Human).len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn confirmation_is_a_persons_move_and_an_agent_never_gets_past_it() {
        let (db, bus) = bus();
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Always), kv_set()).unwrap(),
        )
        .unwrap();
        // A person: asked first, then confirmed.
        let err = bus
            .run(&Actor::Human, "kv.set", json!({"k": "a", "v": "1"}), false)
            .await
            .unwrap_err();
        let CommandError::NeedsConfirmation { preview } = err else {
            panic!("{err:?}");
        };
        assert_eq!(preview.command, "kv.set");
        assert!(!preview.destructive);
        assert_eq!(kv(&db, "a").await, None);
        assert!(
            bus.audit_store().list_recent(5).await.unwrap().is_empty(),
            "nothing audited"
        );
        bus.run(&Actor::Human, "kv.set", json!({"k": "a", "v": "1"}), true)
            .await
            .unwrap();
        assert_eq!(kv(&db, "a").await.as_deref(), Some("1"));
        // An agent: `confirmed` is ignored; nothing is written.
        let err = bus
            .run(&agent(), "kv.set", json!({"k": "b", "v": "2"}), true)
            .await
            .unwrap_err();
        assert!(
            matches!(err, CommandError::NeedsConfirmation { .. }),
            "{err:?}"
        );
        assert_eq!(kv(&db, "b").await, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn undo_applies_the_inverse_and_marks_the_row_once() {
        let (db, bus) = bus();
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Never), kv_set()).unwrap(),
        )
        .unwrap();
        bus.run(&Actor::Human, "kv.set", json!({"k": "a", "v": "1"}), false)
            .await
            .unwrap();
        let second = bus
            .run(&Actor::Human, "kv.set", json!({"k": "a", "v": "2"}), false)
            .await
            .unwrap();
        assert_eq!(kv(&db, "a").await.as_deref(), Some("2"));
        let undo = bus
            .undo(&Actor::Human, second.audit_id, false)
            .await
            .unwrap();
        assert_eq!(kv(&db, "a").await.as_deref(), Some("1"));
        let row = bus
            .audit_store()
            .get(second.audit_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.undone_by, Some(undo.audit_id));
        let err = bus
            .undo(&Actor::Human, second.audit_id, false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("already undone"), "{err}");
        assert!(bus.undo(&Actor::Human, 9999, false).await.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn registration_checks_names_atomicity_and_collisions() {
        let (_db, bus) = bus();
        let mut wrong = kv_spec("kv.set", Invokers::ALL, Confirm::Never);
        wrong.atomicity = Atomicity::BestEffort;
        assert!(Command::new(wrong, kv_set()).is_err());
        assert!(Command::new(kv_spec("set", Invokers::ALL, Confirm::Never), kv_set()).is_err());
        bus.register(
            Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Never), kv_set()).unwrap(),
        )
        .unwrap();
        assert!(bus
            .register(
                Command::new(kv_spec("kv.set", Invokers::ALL, Confirm::Never), kv_set()).unwrap()
            )
            .is_err());
        assert_eq!(bus.best_effort_count(), 0);
    }
}
