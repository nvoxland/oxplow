//! Comment commands (P8.A6, `.context/ipc-and-stores.md` "Comments"):
//! threaded annotations anchored to a span of any page, written through
//! the bus by a person, an agent or a lens. Each is `Tx` over
//! `oxplow_db::comment_store::*_tx`, logging `knowledge.comment.*` caused
//! by the run. The author is the actor — a person `user`, an agent
//! `agent` — never something a caller names. An agent comments on its own
//! stream, its comments in its own thread. The renderer re-locating a
//! comment's anchor is `oxplow.knowledge.relocate_comment`: recorded when the
//! anchor moved, nothing when it is where it was.

use crate::commands::ops::Op;
use std::sync::Arc;

use oxplow_db::comment_store::{
    add_message_tx, create_tx, delete_tx, get_tx, relink_tx, set_anchor_tx, set_intent_tx,
    set_status_tx, NewComment,
};
use oxplow_domain::refs::build::{stream_ref, thread_ref};
use oxplow_domain::{
    Actor, CommandCall, CommandError, CommentId, CommentIntent, CommentStatus, CommentTarget,
    CommentThread, StreamId, ThreadId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::thread::agent_scope;
use super::{Handler, HandlerOutput, TxCtx};

pub const ADD: &str = "oxplow.knowledge.add_comment";
pub const REPLY: &str = "oxplow.knowledge.reply_comment";
pub const UPDATE: &str = "oxplow.knowledge.update_comment";
pub const DELETE: &str = "oxplow.knowledge.delete_comment";
pub const RELOCATE: &str = "oxplow.knowledge.relocate_comment";

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
pub struct RelocateInput {
    /// The comment (`cmt12`).
    pub comment: String,
    /// Where its quote sits in the content as rendered now (the W3C
    /// selectors array, as JSON) — its last known place when `orphaned`.
    pub selectors_json: String,
    /// The quote is no longer in the content.
    pub orphaned: bool,
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

/// `oxplow.knowledge.add_comment`: a comment with its first message.
pub fn add_op() -> Op {
    Op::new(
        "knowledge.write",
        "add_comment",
        schema::<AddInput>(),
        false,
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
}

/// `knowledge.reply_comment { comment, body }`.
pub fn reply_op() -> Op {
    Op::new(
        "knowledge.write",
        "reply_comment",
        schema::<ReplyInput>(),
        false,
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
}

/// `knowledge.update_comment { comment, intent?, status?, quote? +
/// selectors_json? }`: its intent, open/resolved, or a relink to a new
/// span. Undone by what each was.
pub fn update_op() -> Op {
    Op::new(
        "knowledge.write",
        "update_comment",
        schema::<UpdateInput>(),
        true,
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
}

/// `knowledge.delete_comment { comment }`: it and its messages. A
/// person's, confirmed; not undoable.
pub fn delete_op() -> Op {
    Op::new(
        "knowledge.write",
        "delete_comment",
        schema::<CommentInput>(),
        false,
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
}

/// `knowledge.relocate_comment { comment, selectors_json, orphaned }`:
/// the renderer, having found a comment's quote in the content as it is
/// now (or not), stores where. Not a relink — that is a person choosing a
/// new span (`update_comment`). An anchor already where it was changes
/// nothing and leaves no record (`HandlerOutput::unchanged`).
pub fn relocate_op() -> Op {
    Op::new(
        "knowledge.write",
        "relocate_comment",
        schema::<RelocateInput>(),
        false,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: RelocateInput = parse(input)?;
            let id = comment_id(&input.comment)?;
            let before = load(ctx, id)?.comment;
            if before.selectors_json == input.selectors_json && before.orphaned == input.orphaned {
                return Ok(HandlerOutput {
                    result: json!({ "comment": input.comment, "moved": false }),
                    unchanged: true,
                    ..HandlerOutput::default()
                });
            }
            let events = set_anchor_tx(ctx.conn, id, &input.selectors_json, input.orphaned)?;
            Ok(HandlerOutput {
                result: json!({ "comment": input.comment, "moved": true }),
                events,
                ..HandlerOutput::default()
            })
        })),
    )
}

/// The comment commands, for the bus.
pub fn ops() -> Vec<Op> {
    vec![
        add_op(),
        reply_op(),
        update_op(),
        delete_op(),
        relocate_op(),
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

    /// tsk861: a moved anchor is stored and recorded; one already where
    /// it was writes nothing — no audit row, no event — and an agent can't
    /// relocate (it is the renderer's).
    #[tokio::test]
    async fn an_unchanged_anchor_writes_nothing() {
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
        let relocate = |selectors: &str, orphaned: bool| json!({ "comment": id, "selectors_json": selectors, "orphaned": orphaned });
        let run = |actor: Actor, input: Value| {
            let bus = fx.svc.commands.clone();
            async move { bus.run(&actor, RELOCATE, input, false).await }
        };

        let moved = run(Actor::Human, relocate("{\"from\":4}", false))
            .await
            .unwrap();
        assert_eq!(moved.result["moved"], true);
        assert!(moved.audit_id.is_some());
        let stored = fx.svc.comment_store.get(cid).await.unwrap().unwrap();
        assert_eq!(stored.comment.selectors_json, "{\"from\":4}");

        let audits = fx
            .svc
            .commands
            .audit_store()
            .list_recent(500)
            .await
            .unwrap()
            .len();
        let events = fx
            .svc
            .event_log_store
            .read_after(0, 5000)
            .await
            .unwrap()
            .len();
        let again = run(Actor::Human, relocate("{\"from\":4}", false))
            .await
            .unwrap();
        assert_eq!(again.result["moved"], false);
        assert_eq!((again.audit_id, again.event_id), (None, None));
        assert_eq!(
            fx.svc
                .commands
                .audit_store()
                .list_recent(500)
                .await
                .unwrap()
                .len(),
            audits
        );
        assert_eq!(
            fx.svc
                .event_log_store
                .read_after(0, 5000)
                .await
                .unwrap()
                .len(),
            events
        );
        let after = fx.svc.comment_store.get(cid).await.unwrap().unwrap();
        assert_eq!(after.comment.updated_at, stored.comment.updated_at);

        let orphaned = run(Actor::Human, relocate("{\"from\":4}", true))
            .await
            .unwrap();
        assert!(orphaned.audit_id.is_some());
        assert!(
            fx.svc
                .comment_store
                .get(cid)
                .await
                .unwrap()
                .unwrap()
                .comment
                .orphaned
        );

        let err = run(agent(&fx), relocate("{\"from\":5}", false))
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
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
