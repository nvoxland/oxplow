//! Composition: `command.sequence` runs several `Tx` commands as one run
//! (P6b.A1) — the one mechanism a composite is built on, and what an
//! extension's own command (P6b.B2) becomes once its script has said
//! which commands to run. Each child's own invokers, policy and
//! confirmation apply (`CommandBus::run_nested`); the parent has the one
//! audit row and `command.executed`, its children's events caused by it;
//! undo is the children's inverses, reversed.

use std::sync::{Arc, Weak};

use oxplow_domain::{
    Atomicity, CommandCall, CommandEffect, CommandError, CommandSpec, Confirm, Invokers, Lifecycle,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{Command, CommandBus, Handler, HandlerOutput, TxCtx};

pub const SEQUENCE: &str = "command.sequence";

/// `command.sequence`: the commands to run, in order.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SequenceInput {
    pub calls: Vec<SequenceCall>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SequenceCall {
    /// A registered `Tx` command.
    pub name: String,
    /// Its input, matching its schema.
    pub input: Value,
}

pub fn sequence_command(bus: &Arc<CommandBus>) -> Command {
    let bus: Weak<CommandBus> = Arc::downgrade(bus);
    let spec = CommandSpec {
        name: SEQUENCE.into(),
        summary: "Run several commands as one. Each one's own policy and confirmation \
                  apply; the run has one audit row, and undoing it reverses them all."
            .into(),
        input_schema: serde_json::to_value(schemars::schema_for!(SequenceInput))
            .expect("schema serializes"),
        invokers: Invokers::ALL,
        // The children decide: one that asks makes the sequence ask.
        confirm: Confirm::Never,
        undoable: true,
        lifecycle: Lifecycle::Stable,
        atomicity: Atomicity::Tx,
        effect: CommandEffect::Write,
    };
    let parent = spec.clone();
    let handler = Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
        let input: SequenceInput =
            serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                field: None,
                message: e.to_string(),
            })?;
        let bus = bus.upgrade().ok_or_else(|| CommandError::Failed {
            message: "the command bus is gone".into(),
        })?;
        let calls: Vec<CommandCall> = input
            .calls
            .into_iter()
            .map(|c| CommandCall {
                name: c.name,
                input: c.input,
            })
            .collect();
        let nested = bus.run_nested(ctx, &parent, &calls)?;
        Ok(HandlerOutput {
            result: json!({ "result": Value::Null, "children": nested.children }),
            inverse: nested.inverse,
            events: nested.events,
            after_commit: nested.after_commit,
        })
    }));
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
        let fx = crate::test_fixtures::services_with_effort().await;
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
                    { "name": "work_item.update", "input": { "ref": task, "title": "Renamed" } },
                    { "name": "work_item.transition", "input": { "ref": task, "to": "done" } },
                ] }),
                false,
            )
            .await
            .unwrap();
        let after = fx.svc.task_store.get(fx.task).await.unwrap().unwrap();
        assert_eq!(after.title, "Renamed");
        assert_eq!(after.status, oxplow_domain::TaskStatus::Done);
        assert_eq!(out.result["children"].as_array().unwrap().len(), 2);
        assert_eq!(out.result["children"][1]["name"], "work_item.transition");

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
