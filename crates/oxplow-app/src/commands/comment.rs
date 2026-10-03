//! Comment commands (P8.A6, `.context/ipc-and-stores.md` "Comments"):
//! threaded annotations anchored to a span of any page, written through
//! the bus by a person, an agent or a lens. Each is `Tx` over
//! `oxplow_db::comment_store::*_tx`, logging `knowledge.comment.*` caused
//! by the run. The author is the actor — a person `user`, an agent
//! `agent` — never something a caller names. An agent comments on its own
//! stream, its comments in its own thread. Re-locating a comment's anchor
//! after a render is a passive sync, not an intent, so it stays off the
//! bus (`set_comment_anchor`).

use std::sync::Arc;

use oxplow_db::comment_store::{
    add_message_tx, create_tx, delete_tx, get_tx, relink_tx, set_intent_tx, set_status_tx,
    NewComment,
};
use oxplow_domain::refs::build::{stream_ref, thread_ref};
use oxplow_domain::{
    Actor, Atomicity, CommandCall, CommandEffect, CommandError, CommandSpec, CommentId,
    CommentIntent, CommentStatus, CommentTarget, CommentThread, Confirm, Invokers, Lifecycle,
    StreamId, ThreadId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::thread::agent_scope;
use super::{Command, Handler, HandlerOutput, TxCtx};

pub const ADD: &str = "knowledge.add_comment";
pub const REPLY: &str = "knowledge.reply_comment";
pub const UPDATE: &str = "knowledge.update_comment";
pub const DELETE: &str = "knowledge.delete_comment";

/// A person, or a lens acting for one.
const PEOPLE: Invokers = Invokers {
    human: true,
    agent: false,
    lens: true,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddInput {
    /// The stream (`stream:str1`).
    pub stream: String,
    /// The thread it was made in (`thread:thr3`); an agent's is its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// What it's anchored to: a page's kind and id (`wiki` / `some-page`,
    /// `file` / `src/a.rs`, `work_item` / `oxplow:tsk4`).
    pub target: CommentTarget,
    /// The text it's about.
    #[serde(default)]
    pub quote: String,
    /// Where on the page (the W3C selectors array, as JSON).
    #[serde(default)]
    pub selectors_json: String,
    /// The regions the selection sat inside, innermost first.
    #[serde(default)]
    pub context_chain: Vec<CommentTarget>,
    /// The refs inside the selection.
    #[serde(default)]
    pub referenced_refs: Vec<CommentTarget>,
    /// `note` (the agent leaves it alone, the default) or `followup` (the
    /// agent should act on it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<CommentIntent>,
    /// The first message.
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplyInput {
    /// The comment (`cmt12`).
    pub comment: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateInput {
    /// The comment (`cmt12`).
    pub comment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<CommentIntent>,
    /// `open` or `resolved`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<CommentStatus>,
    /// Re-attach it to a newly selected span: its quote …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    /// … and its anchor (both, or neither).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selectors_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommentInput {
    /// The comment (`cmt12`).
    pub comment: String,
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

fn comment_id(value: &str) -> Result<CommentId, CommandError> {
    CommentId::try_from_str(value)
        .ok_or_else(|| invalid("/comment", format!("`{value}` isn't a comment id (cmt…)")))
}

fn stream_of(value: &str) -> Result<StreamId, CommandError> {
    value
        .strip_prefix("stream:")
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| {
            invalid(
                "/stream",
                format!("`{value}` isn't a stream ref (stream:<id>)"),
            )
        })
}

fn thread_of(value: &str) -> Result<ThreadId, CommandError> {
    value
        .strip_prefix("thread:")
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| {
            invalid(
                "/thread",
                format!("`{value}` isn't a thread ref (thread:<id>)"),
            )
        })
}

/// Who wrote it: a person `user`, an agent `agent` (a lens writes as the
/// one it acts for), the system `system`.
pub(super) fn author_of(actor: &Actor) -> &'static str {
    match actor {
        Actor::Human => "user",
        Actor::Agent { .. } => "agent",
        Actor::Lens { on_behalf_of, .. } => author_of(on_behalf_of),
        Actor::System => "system",
        Actor::Effect { .. } => "effect",
    }
}

fn load(ctx: &TxCtx<'_>, id: CommentId) -> Result<CommentThread, CommandError> {
    get_tx(ctx.conn, id)?.ok_or_else(|| invalid("/comment", format!("no comment `{id}`")))
}

/// An agent changes comments only on its own stream.
fn on_own_stream(ctx: &TxCtx<'_>, stream: StreamId) -> Result<(), CommandError> {
    match agent_scope(ctx)? {
        Some((_, own)) if own != stream => Err(CommandError::Denied {
            reason: format!(
                "an agent comments only on its own stream (`{}`)",
                stream_ref(own)
            ),
        }),
        _ => Ok(()),
    }
}

fn schema<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes")
}

fn spec(
    name: &str,
    summary: &str,
    schema: Value,
    invokers: Invokers,
    confirm: Confirm,
    undoable: bool,
) -> CommandSpec {
    CommandSpec {
        name: name.into(),
        summary: summary.into(),
        input_schema: schema,
        invokers,
        confirm,
        undoable,
        lifecycle: Lifecycle::Stable,
        atomicity: Atomicity::Tx,
        effect: CommandEffect::Record,
    }
}

/// `knowledge.add_comment`: a comment with its first message.
pub fn add_command() -> Command {
    Command::new(
        spec(
            ADD,
            "Comment on a span of a page (its `target`), with a first message. `intent`: \
             `note` (the agent leaves it alone) or `followup` (the agent should act on it). \
             An agent comments on its own stream, in its own thread.",
            schema::<AddInput>(),
            Invokers::ALL,
            Confirm::Never,
            false,
        ),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: AddInput = parse(input)?;
            let stream = stream_of(&input.stream)?;
            on_own_stream(ctx, stream)?;
            let named = input.thread.as_deref().map(thread_of).transpose()?;
            let thread = match agent_scope(ctx)? {
                Some((own, _)) => {
                    if named.is_some_and(|t| t != own) {
                        return Err(CommandError::Denied {
                            reason: format!(
                                "an agent comments in its own thread (`{}`)",
                                thread_ref(own)
                            ),
                        });
                    }
                    Some(own)
                }
                None => named,
            };
            let (created, events) = create_tx(
                ctx.conn,
                &ctx.events.vocabulary.kinds,
                &NewComment {
                    stream,
                    thread,
                    target: input.target,
                    quote: input.quote,
                    selectors_json: input.selectors_json,
                    context_chain: input.context_chain,
                    referenced_refs: input.referenced_refs,
                    intent: input.intent.unwrap_or(CommentIntent::Note),
                    author: author_of(ctx.actor).into(),
                    body: input.body,
                },
            )?;
            Ok(HandlerOutput {
                result: serde_json::to_value(created).expect("a comment serializes"),
                events,
                ..HandlerOutput::default()
            })
        })),
    )
    .expect("knowledge.add_comment is a valid command")
}

/// `knowledge.reply_comment { comment, body }`.
pub fn reply_command() -> Command {
    Command::new(
        spec(
            REPLY,
            "Reply on a comment (`cmt12`).",
            schema::<ReplyInput>(),
            Invokers::ALL,
            Confirm::Never,
            false,
        ),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: ReplyInput = parse(input)?;
            let id = comment_id(&input.comment)?;
            on_own_stream(ctx, load(ctx, id)?.comment.stream_id)?;
            let (message, events) =
                add_message_tx(ctx.conn, id, author_of(ctx.actor), &input.body)?;
            Ok(HandlerOutput {
                result: serde_json::to_value(message).expect("a message serializes"),
                events,
                ..HandlerOutput::default()
            })
        })),
    )
    .expect("knowledge.reply_comment is a valid command")
}

/// `knowledge.update_comment { comment, intent?, status?, quote? +
/// selectors_json? }`: its intent, open/resolved, or a relink to a new
/// span. Undone by what each was.
pub fn update_command() -> Command {
    Command::new(
        spec(
            UPDATE,
            "Change a comment (`cmt12`): its `intent`, its `status` (`open` / `resolved`), or \
             re-attach it to a new span (`quote` and `selectors_json` together).",
            schema::<UpdateInput>(),
            Invokers::ALL,
            Confirm::Never,
            true,
        ),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: UpdateInput = parse(input)?;
            let id = comment_id(&input.comment)?;
            let before = load(ctx, id)?.comment;
            on_own_stream(ctx, before.stream_id)?;
            let mut events = Vec::new();
            let mut undo = serde_json::Map::new();
            undo.insert("comment".into(), json!(input.comment));
            if let Some(intent) = input.intent {
                events.extend(set_intent_tx(ctx.conn, id, intent)?);
                undo.insert("intent".into(), json!(before.intent));
            }
            if let Some(status) = input.status {
                events.extend(set_status_tx(ctx.conn, id, status)?);
                undo.insert("status".into(), json!(before.status));
            }
            match (&input.quote, &input.selectors_json) {
                (Some(quote), Some(selectors)) => {
                    events.extend(relink_tx(ctx.conn, id, quote, selectors)?);
                    undo.insert("quote".into(), json!(before.quote));
                    undo.insert("selectors_json".into(), json!(before.selectors_json));
                }
                (None, None) => {}
                _ => {
                    return Err(invalid(
                        "/quote",
                        "a relink names both `quote` and `selectors_json`".into(),
                    ))
                }
            }
            if undo.len() == 1 {
                return Err(invalid(
                    "",
                    "name what changes: `intent`, `status`, or `quote` + `selectors_json`".into(),
                ));
            }
            Ok(HandlerOutput {
                result: serde_json::to_value(load(ctx, id)?).expect("a comment serializes"),
                inverse: Some(CommandCall {
                    name: UPDATE.into(),
                    input: Value::Object(undo),
                }),
                events,
                ..HandlerOutput::default()
            })
        })),
    )
    .expect("knowledge.update_comment is a valid command")
}

/// `knowledge.delete_comment { comment }`: it and its messages. A
/// person's, confirmed; not undoable.
pub fn delete_command() -> Command {
    Command::new(
        spec(
            DELETE,
            "Delete a comment and its messages (`cmt12`). A person's, confirmed.",
            schema::<CommentInput>(),
            PEOPLE,
            Confirm::Destructive,
            false,
        ),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: CommentInput = parse(input)?;
            let id = comment_id(&input.comment)?;
            load(ctx, id)?;
            let events = delete_tx(ctx.conn, id)?;
            Ok(HandlerOutput {
                result: json!({ "comment": input.comment, "deleted": true }),
                events,
                ..HandlerOutput::default()
            })
        })),
    )
    .expect("knowledge.delete_comment is a valid command")
}

/// The comment commands, for the bus.
pub fn commands() -> Vec<Command> {
    vec![
        add_command(),
        reply_command(),
        update_command(),
        delete_command(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{services_with_effort, EffortFixture};
    use oxplow_domain::stores::CommentStore as _;

    fn agent(fx: &EffortFixture) -> Actor {
        Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: None,
        }
    }

    async fn add(fx: &EffortFixture, actor: &Actor, input: Value) -> Result<Value, CommandError> {
        fx.svc
            .commands
            .run(actor, ADD, input, false)
            .await
            .map(|o| o.result)
    }

    fn comment_on(stream: &str, thread: Option<&str>) -> Value {
        let mut v = json!({ "stream": stream, "target": { "kind": "wiki", "id": "a-page" },
                            "quote": "the words", "body": "what about this?" });
        if let Some(t) = thread {
            v["thread"] = json!(t);
        }
        v
    }

    /// The author is the actor, and an agent's comment is in its own
    /// thread — naming another is refused; its knowledge event is caused by
    /// the run.
    #[tokio::test]
    async fn the_author_is_the_actor_and_an_agent_comments_in_its_own_thread() {
        let fx = services_with_effort().await;
        let stream = stream_ref(StreamId::new(1));
        let mine = add(&fx, &agent(&fx), comment_on(&stream, None))
            .await
            .unwrap();
        assert_eq!(mine["comment"]["author"], "agent");
        assert_eq!(mine["comment"]["thread_id"], json!(fx.thread));
        let err = add(&fx, &agent(&fx), comment_on(&stream, Some("thread:thr99")))
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        let theirs = add(&fx, &Actor::Human, comment_on(&stream, None))
            .await
            .unwrap();
        assert_eq!(theirs["comment"]["author"], "user");

        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                REPLY,
                json!({ "comment": mine["comment"]["id"], "body": "ok" }),
                false,
            )
            .await
            .unwrap();
        let events = fx.svc.event_log_store.read_after(0, 500).await.unwrap();
        assert!(events.iter().any(|e| {
            e.envelope.event_type == "knowledge.comment.written"
                && e.envelope.cause.as_ref() == out.event_id.as_ref()
        }));
    }

    /// Resolving undoes to open; deleting is a person's and asks first.
    #[tokio::test]
    async fn resolve_undoes_and_delete_is_a_persons() {
        let fx = services_with_effort().await;
        let c = add(
            &fx,
            &Actor::Human,
            comment_on(&stream_ref(StreamId::new(1)), None),
        )
        .await
        .unwrap();
        let id = c["comment"]["id"].as_str().unwrap().to_string();
        let cid = CommentId::try_from_str(&id).unwrap();
        let out = fx
            .svc
            .commands
            .run(
                &agent(&fx),
                UPDATE,
                json!({ "comment": id, "status": "resolved" }),
                false,
            )
            .await
            .unwrap();
        let status = |fx: &EffortFixture| {
            let store = fx.svc.comment_store.clone();
            async move { store.get(cid).await.unwrap().unwrap().comment.status }
        };
        assert_eq!(status(&fx).await, CommentStatus::Resolved);
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(status(&fx).await, CommentStatus::Open);

        let err = fx
            .svc
            .commands
            .run(&agent(&fx), DELETE, json!({ "comment": id }), false)
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        let err = fx
            .svc
            .commands
            .run(&Actor::Human, DELETE, json!({ "comment": id }), false)
            .await
            .unwrap_err();
        assert!(
            matches!(err, CommandError::NeedsConfirmation { .. }),
            "{err:?}"
        );
    }
}
