//! Context basket operations (`context.write`; declared as commands in
//! `extensions/oxplow-foundation`): the person puts a page or a piece of
//! text in a thread's basket for its agents, takes one out, or empties
//! it. `Tx` over the store's `_tx` cores; read back through
//! `v_thread_context`. The next prompt to any agent in the thread carries
//! what waits there (`agent_context`). Each undoes; an agent's call is
//! refused, since the basket is what the person collected.

use std::sync::Arc;

use oxplow_db::thread_context_store::{add_tx, clear_tx, remove_tx, ContextItem, NewItem};
use oxplow_domain::{CommandCall, CommandError, ThreadId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::ops::Op;
use super::util::{bare_id, invalid, parse, schema};
use super::{Handler, HandlerOutput, TxCtx};

/// The capability its operations are of.
pub const CAPABILITY: &str = "context.write";
/// The commands that undo them.
pub const ADD: &str = "oxplow.context.add";
pub const REMOVE: &str = "oxplow.context.remove";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddInput {
    /// The thread whose basket it goes in (`thr1`).
    pub thread: String,
    /// The page (its canonical URI), or where `text` came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    /// What the person saw it as (the page's title).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// A piece of text: with it the item is text, from `uri` when given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveInput {
    /// The item (`v_thread_context.id`).
    pub item: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClearInput {
    /// The thread whose basket is emptied (`thr1`).
    pub thread: String,
}

fn person_only(ctx: &TxCtx<'_>) -> Result<(), CommandError> {
    if ctx.actor().is_agent_driven() {
        return Err(invalid(
            "/",
            "the context basket is what the person collects for the thread's agents: an agent \
             doesn't add to or change it",
        ));
    }
    Ok(())
}

fn thread_of(raw: &str) -> Result<ThreadId, CommandError> {
    bare_id(raw, oxplow_domain::uri::resources::THREAD, "/thread")
}

/// The call that puts `item` back (as a new item).
fn re_add(item: &ContextItem) -> CommandCall {
    CommandCall {
        name: ADD.into(),
        input: json!({
            "thread": ThreadId::new(item.thread_id).to_string(),
            "uri": item.source_uri, "label": item.label, "text": item.text,
        }),
    }
}

/// `add { thread, uri?, label?, text? }`.
pub fn add_op() -> Op {
    Op::new(
        CAPABILITY,
        "add",
        schema::<AddInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            person_only(ctx)?;
            let input: AddInput = parse(input)?;
            let thread = thread_of(&input.thread)?;
            let id = add_tx(
                ctx.conn,
                thread,
                &NewItem {
                    source_uri: input.uri.filter(|u| !u.trim().is_empty()),
                    label: input.label.filter(|l| !l.trim().is_empty()),
                    text: input.text.filter(|t| !t.trim().is_empty()),
                },
            )
            .map_err(|e| match e {
                oxplow_domain::DomainError::Invalid(m) => invalid("/", m),
                other => CommandError::from(other),
            })?;
            Ok(HandlerOutput {
                result: json!({ "item": id }),
                inverse: Some(CommandCall {
                    name: REMOVE.into(),
                    input: json!({ "item": id }),
                }),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// `remove { item }`.
pub fn remove_op() -> Op {
    Op::new(
        CAPABILITY,
        "remove",
        schema::<RemoveInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            person_only(ctx)?;
            let input: RemoveInput = parse(input)?;
            let removed = remove_tx(ctx.conn, input.item)?;
            Ok(HandlerOutput {
                result: json!({ "removed": removed.is_some() }),
                inverse: removed.as_ref().map(re_add),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// `clear { thread }`: what still waits; delivered items stay.
pub fn clear_op() -> Op {
    Op::new(
        CAPABILITY,
        "clear",
        schema::<ClearInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            person_only(ctx)?;
            let input: ClearInput = parse(input)?;
            let cleared = clear_tx(ctx.conn, thread_of(&input.thread)?)?;
            let inverse = (!cleared.is_empty()).then(|| CommandCall {
                name: super::compose::SEQUENCE.into(),
                input: json!({
                    "calls": cleared.iter().map(re_add).map(|c| json!({ "name": c.name, "input": c.input })).collect::<Vec<Value>>(),
                }),
            });
            Ok(HandlerOutput {
                result: json!({ "cleared": cleared.len() }),
                inverse,
                ..HandlerOutput::default()
            })
        })),
    )
}

/// The context basket operations, for the bus.
pub fn ops() -> Vec<Op> {
    vec![add_op(), remove_op(), clear_op()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::services_with_effort;
    use oxplow_domain::Actor;

    async fn waiting(fx: &crate::test_fixtures::EffortFixture) -> Vec<(String, Option<String>)> {
        let thread = fx.thread;
        fx.svc
            .db
            .read(move |c| oxplow_db::thread_context_store::undelivered_tx(c, thread))
            .await
            .unwrap()
            .into_iter()
            .map(|i| (i.kind.as_str().to_string(), i.source_uri))
            .collect()
    }

    /// A person adds a page and a piece of text; undoing the second takes
    /// it back out.
    #[tokio::test]
    async fn add_and_undo() {
        let fx = services_with_effort().await;
        let thread = fx.thread.to_string();
        fx.svc
            .commands
            .run(
                &Actor::Human,
                ADD,
                json!({ "thread": thread, "uri": "oxplow:oxplow/commit?sha=abc", "label": "abc" }),
                false,
            )
            .await
            .unwrap();
        let text = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                ADD,
                json!({ "thread": thread, "uri": "oxplow:oxplow/file?path=a.rs#lines=1-2", "text": "fn a() {}" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            waiting(&fx).await,
            [
                ("page".into(), Some("oxplow:oxplow/commit?sha=abc".into())),
                (
                    "text".into(),
                    Some("oxplow:oxplow/file?path=a.rs#lines=1-2".into())
                ),
            ]
        );
        fx.svc
            .commands
            .undo(&Actor::Human, text.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(waiting(&fx).await.len(), 1);
    }

    /// Clearing empties what waits; undoing it puts everything back.
    #[tokio::test]
    async fn clear_and_undo() {
        let fx = services_with_effort().await;
        let thread = fx.thread.to_string();
        for n in 0..2 {
            fx.svc
                .commands
                .run(
                    &Actor::Human,
                    ADD,
                    json!({ "thread": thread, "uri": format!("oxplow:oxplow/wiki?slug=p{n}") }),
                    false,
                )
                .await
                .unwrap();
        }
        let cleared = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.context.clear",
                json!({ "thread": thread }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(cleared.result["cleared"], json!(2));
        assert!(waiting(&fx).await.is_empty());
        fx.svc
            .commands
            .undo(&Actor::Human, cleared.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(waiting(&fx).await.len(), 2);
    }

    /// The basket is the person's: an agent's add is refused.
    #[tokio::test]
    async fn an_agent_may_not_add() {
        let fx = services_with_effort().await;
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let err = fx
            .svc
            .commands
            .run(
                &agent,
                ADD,
                json!({ "thread": fx.thread.to_string(), "uri": "oxplow:oxplow/wiki?slug=x" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("the person collects"), "{err}");
    }
}
