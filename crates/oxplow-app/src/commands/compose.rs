//! Composition: a command made of other commands' calls (P6b.A1) — the
//! one mechanism `oxplow.command.sequence` and an extension's own commands
//! (P6b.B2) are built on. A composite is a [`Compose`] handler: given its
//! input it says which calls to run; the bus decides where they run.
//!
//! - Every call runs in the transaction (a `Tx` command, a `Dispatch`
//!   command routed inside it, a composite whose calls all do): one run in
//!   one transaction (`CommandBus::run_nested`) — all or nothing, one audit
//!   row and `command.executed` with the children's events caused by it,
//!   undone by the children's inverses reversed.
//! - Any call leaves it (an `External` command, `work_item.*` on another
//!   provider's item): its **steps** run in order (`steps.rs`), each
//!   landing as it runs, with no undo (P7 review, tsk713).
//!
//! Either way each child's own invokers, policy and confirmation apply,
//! checked before anything runs.

use std::sync::{Arc, Weak};

use oxplow_domain::{
    Atomicity, CommandCall, CommandEffect, CommandError, CommandSpec, Confirm, Invokers, Lifecycle,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{Command, CommandBus, Handler, HandlerOutput, TxCtx, TxHandler};

/// What a composite runs for one input: its calls, in order, the run's
/// own `result` (beside the children's), and events of its own, appended
/// after the children's and caused by the run's `command.executed` (an
/// extension command's declared types, P8.D4).
#[derive(Debug, Clone, Default)]
pub struct Composition {
    pub calls: Vec<CommandCall>,
    pub result: Option<Value>,
    pub events: Vec<oxplow_domain::Envelope>,
}

/// Say what a composite runs for `input`, reading on `conn` (the run's
/// transaction, or a read snapshot when the bus routes it). Pure: the bus
/// may compose more than once.
pub type Composer =
    dyn Fn(&rusqlite::Connection, &Value) -> Result<Composition, CommandError> + Send + Sync;

/// A composite's handler: its composer, and the `Tx` handler that runs
/// what it composes in the bus's transaction (`run_nested`) when every
/// call can.
pub struct Compose {
    pub compose: Arc<Composer>,
    pub tx: Arc<TxHandler>,
}

impl Compose {
    /// The composite `parent` (its spec, for the children's checks) over
    /// `compose`, on `bus`.
    pub fn handler(bus: &Arc<CommandBus>, parent: CommandSpec, compose: Arc<Composer>) -> Handler {
        let bus: Weak<CommandBus> = Arc::downgrade(bus);
        let composer = compose.clone();
        let tx: Arc<TxHandler> = Arc::new(move |ctx: &TxCtx<'_>, input: Value| {
            let bus = bus.upgrade().ok_or_else(|| CommandError::Failed {
                message: "the command bus is gone".into(),
            })?;
            let Composition {
                calls,
                result,
                events,
            } = composer(ctx.conn, &input)?;
            let nested = bus.run_nested(ctx, &parent, &calls)?;
            Ok(HandlerOutput {
                result: json!({ "result": result, "children": nested.children }),
                inverse: nested.inverse,
                events: nested.events.into_iter().chain(events).collect(),
                after_commit: nested.after_commit,
                unchanged: false,
            })
        });
        Handler::Compose(Arc::new(Compose { compose, tx }))
    }
}

pub const SEQUENCE: &str = "oxplow.command.sequence";

/// `oxplow.command.sequence`: the commands to run, in order.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SequenceInput {
    pub calls: Vec<SequenceCall>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SequenceCall {
    /// A registered command that writes.
    pub name: String,
    /// Its input, matching its schema.
    pub input: Value,
}

/// `oxplow.command.sequence`'s spec: also what an extension effect's run is
/// audited as (`effect_triggers`).
pub fn sequence_spec() -> CommandSpec {
    CommandSpec {
        name: SEQUENCE.into(),
        summary: "Run several commands as one, each one's own policy and confirmation \
                  checked before any runs; the run has one audit row. In oxplow's own \
                  records they run in one transaction and undo together; when one goes to \
                  an external system they run in order, each landing as it runs, with no undo."
            .into(),
        input_schema: serde_json::to_value(schemars::schema_for!(SequenceInput))
            .expect("schema serializes"),
        invokers: Invokers::ALL,
        // The children decide: one that asks makes the sequence ask.
        confirm: Confirm::Never,
        undoable: true,
        lifecycle: Lifecycle::Stable,
        // In the transaction, or as steps outside it: its calls decide.
        atomicity: Atomicity::Dispatch,
        effect: CommandEffect::Write,
        needs: Vec::new(),
    }
}

pub fn sequence_command(bus: &Arc<CommandBus>) -> Command {
    let spec = sequence_spec();
    let compose: Arc<Composer> = Arc::new(|_conn, input: &Value| {
        let input: SequenceInput =
            serde_json::from_value(input.clone()).map_err(|e| CommandError::Invalid {
                field: None,
                message: e.to_string(),
            })?;
        Ok(Composition {
            calls: input
                .calls
                .into_iter()
                .map(|c| CommandCall {
                    name: c.name,
                    input: c.input,
                })
                .collect(),
            result: None,
            events: Vec::new(),
        })
    });
    let handler = Compose::handler(bus, spec.clone(), compose);
    Command::new(spec, handler).expect("command.sequence registers")
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::refs::build::work_item_ref;
    use oxplow_domain::stores::TaskStore as _;
    use oxplow_domain::Actor;

    /// Two `work_item.*` commands as one run by an agent: one audit row
    /// naming the sequence with the children in its result, the children's
    /// events caused by that one `command.executed`, and one undo that
    /// reverses both.
    #[tokio::test]
    async fn a_sequence_of_work_item_commands_is_one_audited_undoable_run() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let agent = Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let task = work_item_ref(fx.task);
        let before = fx.svc.task_store.get(fx.task).await.unwrap().unwrap();
        let out = fx
            .svc
            .commands
            .run(
                &agent,
                SEQUENCE,
                json!({ "calls": [
                    { "name": "oxplow.work_item.update", "input": { "ref": task, "title": "Renamed" } },
                    { "name": "oxplow.work_item.transition", "input": { "ref": task, "to": "done" } },
                ] }),
                false,
            )
            .await
            .unwrap();
        let after = fx.svc.task_store.get(fx.task).await.unwrap().unwrap();
        assert_eq!(after.title, "Renamed");
        assert_eq!(after.status, oxplow_domain::TaskStatus::Done);
        assert_eq!(out.result["children"].as_array().unwrap().len(), 2);
        assert_eq!(
            out.result["children"][1]["name"],
            "oxplow.work_item.transition"
        );

        let audits = oxplow_db::SqliteCommandAuditStore::new(fx.svc.db.clone())
            .list_recent(20)
            .await
            .unwrap();
        let ok: Vec<_> = audits
            .iter()
            .filter(|a| a.outcome == oxplow_domain::events::schema::CommandOutcome::Ok)
            .collect();
        assert_eq!(ok.len(), 1, "{audits:?}");
        assert_eq!(ok[0].command, SEQUENCE);

        let events = fx.svc.event_log_store.read_after(0, 1000).await.unwrap();
        let executed = events
            .iter()
            .filter(|e| e.envelope.event_type == "command.executed")
            .collect::<Vec<_>>();
        assert_eq!(executed.len(), 1);
        for t in ["work_item.edited", "work_item.transitioned"] {
            let caused = events
                .iter()
                .find(|e| e.envelope.event_type == t)
                .unwrap_or_else(|| panic!("{t} logged"));
            assert_eq!(
                caused.envelope.cause.as_ref(),
                Some(&executed[0].envelope.id),
                "{t}"
            );
        }

        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        let restored = fx.svc.task_store.get(fx.task).await.unwrap().unwrap();
        assert_eq!(restored.title, before.title);
        assert_eq!(restored.status, before.status);
    }
}
