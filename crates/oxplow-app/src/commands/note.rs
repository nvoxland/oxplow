//! Thread note commands (P8.A6): the per-thread capture pad — what an
//! agent records as it works (a finding, why it paused) and what an
//! Explore subagent fills in. `Tx` over `oxplow_db::thread_note_store::
//! {add_thread_note_tx, update_note_tx}`, logging `knowledge.note.*`
//! caused by the run, with the note's `page_ref` edges. An agent writes
//! only on its own thread, whatever thread it names; the author is the
//! actor. Both are `Record` and never ask, so an agent that may not write
//! the worktree can still take notes. The result carries `link_warnings`:
//! each `[[…]]` in the body that doesn't resolve.

use std::sync::Arc;

use oxplow_db::thread_note_store::{add_thread_note_tx, note_tx, update_note_tx};
use oxplow_domain::refs::build::thread_ref;
use oxplow_domain::{
    Atomicity, CommandCall, CommandEffect, CommandError, CommandSpec, Confirm, Invokers, Lifecycle,
    NoteId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::comment::author_of;
use super::thread::{acting_thread, agent_scope};
use super::{Command, Handler, HandlerOutput, TxCtx};
use crate::link_check::LinkDeps;

pub const ADD: &str = "oxplow.knowledge.add_note";
pub const UPDATE: &str = "oxplow.knowledge.update_note";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddInput {
    /// The thread (`thread:thr3`); an agent's is always its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// The note (markdown; `[[…]]` links are checked). May be empty: a
    /// note allocated for a subagent to fill in with
    /// `oxplow.knowledge.update_note`.
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateInput {
    /// The note (`not12`).
    pub note: String,
    pub body: String,
}

fn invalid(field: &str, message: String) -> CommandError {
    CommandError::Invalid {
        field: Some(field.into()),
        message,
    }
}

fn parse<T: serde::de::DeserializeOwned>(input: Value) -> Result<T, CommandError> {
    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
        field: None,
        message: e.to_string(),
    })
}

/// The note and the links in its body that don't resolve.
fn with_warnings(
    deps: &LinkDeps,
    ctx: &TxCtx<'_>,
    note: Value,
    body: &str,
    thread: Option<oxplow_domain::ThreadId>,
) -> Value {
    json!({ "note": note, "link_warnings": deps.warnings(ctx, body, thread) })
}

fn schema<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes")
}

fn spec(name: &str, summary: &str, schema: Value, undoable: bool) -> CommandSpec {
    CommandSpec {
        id: name.into(),
        summary: summary.into(),
        input_schema: schema,
        invokers: Invokers::ALL,
        confirm: Confirm::Never,
        undoable,
        lifecycle: Lifecycle::Stable,
        atomicity: Atomicity::Tx,
        effect: CommandEffect::Record,
        needs: Vec::new(),
    }
}

/// `knowledge.add_note { thread?, body }`.
pub fn add_command(deps: LinkDeps) -> Command {
    Command::new(
        spec(
            ADD,
            "Add a note to a thread — an agent's own (a finding, why it paused, context for \
             whoever picks it up). An empty body allocates one for a subagent to fill in with \
             `oxplow.knowledge.update_note`. Returns the note and `link_warnings` for `[[…]]` links \
             that don't resolve.",
            schema::<AddInput>(),
            false,
        ),
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            let input: AddInput = parse(input)?;
            let thread = acting_thread(ctx, input.thread.as_deref())?;
            let (note, event) = add_thread_note_tx(
                ctx.conn,
                &ctx.events.vocabulary.kinds,
                thread,
                &input.body,
                author_of(ctx.actor),
            )?;
            let note = serde_json::to_value(note).expect("a note serializes");
            Ok(HandlerOutput {
                result: with_warnings(&deps, ctx, note, &input.body, Some(thread)),
                events: vec![event],
                ..HandlerOutput::default()
            })
        })),
    )
    .expect("oxplow.knowledge.add_note is a valid command")
}

/// `knowledge.update_note { note, body }`; undone by its previous body.
pub fn update_command(deps: LinkDeps) -> Command {
    Command::new(
        spec(
            UPDATE,
            "Replace a thread note's body (`not12`) — a subagent filling in the note it was \
             given. An agent updates only its own thread's notes.",
            schema::<UpdateInput>(),
            true,
        ),
        Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
            let input: UpdateInput = parse(input)?;
            let id = NoteId::try_from_str(&input.note).ok_or_else(|| {
                invalid("/note", format!("`{}` isn't a note id (not…)", input.note))
            })?;
            let before = note_tx(ctx.conn, id)?
                .ok_or_else(|| invalid("/note", format!("no thread note `{id}`")))?;
            if let Some((own, _)) = agent_scope(ctx)? {
                if before.thread_id != own {
                    return Err(CommandError::Denied {
                        reason: format!(
                            "an agent updates only its own thread's notes (`{}`)",
                            thread_ref(own)
                        ),
                    });
                }
            }
            let event = update_note_tx(ctx.conn, &ctx.events.vocabulary.kinds, id, &input.body)?;
            let note = serde_json::to_value(note_tx(ctx.conn, id)?).expect("a note serializes");
            Ok(HandlerOutput {
                result: with_warnings(&deps, ctx, note, &input.body, Some(before.thread_id)),
                inverse: Some(CommandCall {
                    name: UPDATE.into(),
                    input: json!({ "note": input.note, "body": before.body }),
                }),
                events: event.into_iter().collect(),
                ..HandlerOutput::default()
            })
        })),
    )
    .expect("oxplow.knowledge.update_note is a valid command")
}

/// The note commands, for the bus.
pub fn commands(deps: LinkDeps) -> Vec<Command> {
    vec![add_command(deps.clone()), update_command(deps)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{services_with_effort, EffortFixture};
    use oxplow_domain::stores::ThreadNoteStore as _;
    use oxplow_domain::Actor;

    fn agent(fx: &EffortFixture) -> Actor {
        Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: None,
        }
    }

    /// tsk895: a note's file links are checked in its thread's worktree —
    /// a file only there is a good link, one only in the primary checkout
    /// isn't.
    #[tokio::test]
    async fn a_worktree_threads_links_are_checked_in_its_worktree() {
        let fx = services_with_effort().await;
        let worktree = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(worktree.path().join("src")).unwrap();
        std::fs::write(worktree.path().join("src/only_here.rs"), "").unwrap();
        std::fs::create_dir_all(fx.svc.layout.project_dir.join("src")).unwrap();
        std::fs::write(fx.svc.layout.project_dir.join("src/only_primary.rs"), "").unwrap();
        let path = worktree.path().to_string_lossy().into_owned();
        fx.svc
            .db
            .transaction(move |tx| {
                tx.execute(
                    "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source,
                       worktree_path, created_at, updated_at)
                     VALUES (2, 'worktree', 'w', 'w', 'refs/heads/w', 'main', ?1,
                       '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
                    [&path],
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        let thread =
            crate::test_fixtures::new_thread(&fx.svc, oxplow_domain::StreamId::new(2), "w").await;
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Agent {
                    thread_id: Some(thread.id),
                    stream_id: None,
                },
                ADD,
                json!({ "body": "see [[src/only_here.rs]] not [[src/only_primary.rs]]" }),
                false,
            )
            .await
            .unwrap();
        let targets: Vec<&str> = out.result["link_warnings"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|w| w["target"].as_str())
            .collect();
        assert_eq!(targets, vec!["src/only_primary.rs"]);
    }

    /// A queued agent — one that may not write the worktree — still takes
    /// notes, on its own thread whatever it names; the note's event is
    /// caused by the run, and a broken link comes back as a warning.
    #[tokio::test]
    async fn a_queued_agent_takes_notes_on_its_own_thread() {
        let fx = services_with_effort().await;
        let second =
            crate::test_fixtures::new_thread(&fx.svc, oxplow_domain::StreamId::new(1), "q").await;
        let queued = Actor::Agent {
            thread_id: Some(second.id),
            stream_id: None,
        };
        let out = fx
            .svc
            .commands
            .run(
                &queued,
                ADD,
                json!({ "body": "found it in [[tsk999]]" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["note"]["thread_id"], json!(second.id));
        assert_eq!(out.result["note"]["author"], "agent");
        assert_eq!(out.result["link_warnings"][0]["target"], "tsk999");
        let events = fx.svc.event_log_store.read_after(0, 500).await.unwrap();
        assert!(events
            .iter()
            .any(|e| e.envelope.event_type == "knowledge.note.written"
                && e.envelope.cause.as_ref() == out.event_id.as_ref()));

        let err = fx
            .svc
            .commands
            .run(
                &queued,
                ADD,
                json!({ "thread": thread_ref(fx.thread), "body": "x" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
    }

    /// A subagent fills in the note it was given; another thread's note is
    /// refused; the update undoes to the empty body.
    #[tokio::test]
    async fn a_note_is_filled_in_and_undone() {
        let fx = services_with_effort().await;
        let out = fx
            .svc
            .commands
            .run(&agent(&fx), ADD, json!({}), false)
            .await
            .unwrap();
        let id = out.result["note"]["id"].as_str().unwrap().to_string();
        let filled = fx
            .svc
            .commands
            .run(
                &agent(&fx),
                UPDATE,
                json!({ "note": id, "body": "the answer" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(filled.result["note"]["body"], "the answer");
        fx.svc
            .commands
            .undo(&Actor::Human, filled.audit_id.unwrap(), false)
            .await
            .unwrap();
        let notes = fx
            .svc
            .thread_note_store
            .list_for_thread(&fx.thread)
            .await
            .unwrap();
        assert_eq!(notes[0].body, "");

        let other =
            crate::test_fixtures::new_thread(&fx.svc, oxplow_domain::StreamId::new(1), "o").await;
        let stranger = Actor::Agent {
            thread_id: Some(other.id),
            stream_id: None,
        };
        let err = fx
            .svc
            .commands
            .run(
                &stranger,
                UPDATE,
                json!({ "note": id, "body": "mine now" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
    }
}
