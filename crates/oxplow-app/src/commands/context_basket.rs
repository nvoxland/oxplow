//! Context basket operations (`context.write`; declared as commands in
//! `extensions/oxplow-foundation`): the person collects pages and pieces of
//! text for agents into a basket — a thread's own, or a named basket of the
//! project — moves them between baskets, notes them, and attaches a named
//! basket to a thread's next prompt. `Tx` over `oxplow_db::context_store`'s
//! `_tx` cores; read back through `v_context_item` and `v_context_basket`.
//! The next prompt to any agent in a thread carries its own basket and the
//! baskets attached to it (`agent_context`). Each undoes; an agent's call is
//! refused, since the baskets are what the person collected.

use std::sync::Arc;

use oxplow_db::context_store::{self as store, Basket, ContextItem, NamedBasket, NewItem};
use oxplow_domain::{CommandCall, CommandError, DomainError, StreamId, ThreadId};
use rusqlite::{params, Connection, OptionalExtension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::ops::Op;
use super::util::{bare_id, invalid, parse, schema};
use super::{Handler, HandlerOutput, TxCtx};

/// The capability its operations are of.
pub const CAPABILITY: &str = "context.write";
/// The commands, by name.
pub const ADD: &str = "oxplow.context.add";
pub const REMOVE: &str = "oxplow.context.remove";
pub const NOTE: &str = "oxplow.context.note";
pub const MOVE: &str = "oxplow.context.move";
pub const CLEAR: &str = "oxplow.context.clear";
pub const ATTACH: &str = "oxplow.context.attach";
pub const DETACH: &str = "oxplow.context.detach";
pub const CREATE_BASKET: &str = "oxplow.context.create_basket";
pub const RENAME_BASKET: &str = "oxplow.context.rename_basket";
pub const DELETE_BASKET: &str = "oxplow.context.delete_basket";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddInput {
    /// The thread whose own basket it goes in (`thr1`); or
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// the named basket it goes in, by name, made when there is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basket: Option<String>,
    /// The stream it was collected in (`str1`): a file `uri` names is in
    /// that stream's worktree. Default: the thread's stream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,
    /// The page (its canonical URI), or where `text` came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    /// What the person saw it as (the page's title).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// A piece of text: with it the item is text, from `uri` when given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The lines around `text`, when `uri` has no lines to read them back
    /// from (a terminal's scrollback).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub around: Option<String>,
    /// The person's note: what they want done about it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoteInput {
    /// The item (`v_context_item.id`).
    pub item: i64,
    /// Its note; empty or absent clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ItemInput {
    /// The item (`v_context_item.id`).
    pub item: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveInput {
    /// The item (`v_context_item.id`).
    pub item: i64,
    /// The thread whose own basket it moves to (`thr1`); or
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// the named basket it moves to, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basket: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WhichInput {
    /// The thread whose own basket it is (`thr1`); or
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// the named basket, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basket: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttachInput {
    /// The named basket, by name.
    pub basket: String,
    /// The thread whose next prompt carries it (`thr1`).
    pub thread: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BasketInput {
    /// The named basket, by name.
    pub basket: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NameInput {
    /// The new basket's name: up to 60 characters, not "This Thread",
    /// unique in any case.
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameInput {
    /// The named basket, by its name now.
    pub basket: String,
    /// Its new name.
    pub name: String,
}

fn person_only(ctx: &TxCtx<'_>) -> Result<(), CommandError> {
    if ctx.actor().is_agent_driven() {
        return Err(invalid(
            "/",
            "the context baskets are what the person collects for agents: an agent \
             doesn't add to or change them",
        ));
    }
    Ok(())
}

/// A store refusal as the field it is about.
fn at(field: &'static str) -> impl Fn(DomainError) -> CommandError {
    move |e| match e {
        DomainError::Invalid(m) => invalid(field, m),
        other => CommandError::from(other),
    }
}

fn thread_of(raw: &str, field: &str) -> Result<ThreadId, CommandError> {
    bare_id(raw, oxplow_domain::uri::resources::THREAD, field)
}

/// The named basket called `name`, refused when there is none.
fn named(conn: &Connection, name: &str, field: &'static str) -> Result<NamedBasket, CommandError> {
    store::basket_by_name_tx(conn, name)?.ok_or_else(|| {
        invalid(
            field,
            format!("there is no basket named \"{}\"", name.trim()),
        )
    })
}

/// The basket `thread` or `basket` names: exactly one of them.
fn which(
    conn: &Connection,
    thread: Option<&str>,
    basket: Option<&str>,
) -> Result<Basket, CommandError> {
    match (thread, basket) {
        (Some(t), None) => Ok(Basket::Thread(thread_of(t, "/thread")?)),
        (None, Some(b)) => Ok(Basket::Named(named(conn, b, "/basket")?.id)),
        _ => Err(invalid(
            "/",
            "name exactly one basket: a `thread` (its own) or a named `basket`",
        )),
    }
}

/// How the commands name `basket`: `{ "thread": … }` or `{ "basket": … }`.
fn naming(conn: &Connection, basket: Basket) -> Result<Value, CommandError> {
    Ok(match basket {
        Basket::Thread(t) => json!({ "thread": t.to_string() }),
        Basket::Named(id) => {
            let b = store::get_basket_tx(conn, id)?
                .ok_or_else(|| invalid("/basket", format!("no basket {id}")))?;
            json!({ "basket": b.name })
        }
    })
}

/// The stream `thread` is in.
fn stream_of_thread(conn: &Connection, thread: ThreadId) -> Result<Option<i64>, CommandError> {
    conn.query_row(
        "SELECT stream_id FROM threads WHERE id = ?1",
        params![thread.value()],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| CommandError::from(oxplow_db::map_sql_err(e)))
}

/// The call that puts `item` back (as a new item) in `basket`.
fn re_add(conn: &Connection, item: &ContextItem) -> Result<CommandCall, CommandError> {
    let mut input = naming(conn, item.basket())?;
    let fields = json!({
        "stream": item.stream_id.map(|s| StreamId::new(s).to_string()),
        "uri": item.source_uri, "label": item.label, "text": item.text,
        "around": item.around, "note": item.note,
    });
    for (k, v) in fields.as_object().into_iter().flatten() {
        if !v.is_null() {
            input[k] = v.clone();
        }
    }
    Ok(CommandCall {
        name: ADD.into(),
        input,
    })
}

fn sequence(calls: Vec<CommandCall>) -> Option<CommandCall> {
    (!calls.is_empty()).then(|| CommandCall {
        name: super::compose::SEQUENCE.into(),
        input: json!({
            "calls": calls.into_iter().map(|c| json!({ "name": c.name, "input": c.input })).collect::<Vec<Value>>(),
        }),
    })
}

/// A note as stored: trimmed, and none when nothing is left.
fn note_of(note: Option<String>) -> Option<String> {
    note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty())
}

fn op(name: &'static str, input: Value, handler: Handler) -> Op {
    Op::new(CAPABILITY, name, input, true, handler)
}

/// `add { thread | basket, stream?, uri?, label?, text?, around?, note? }`.
pub fn add_op() -> Op {
    op(
        "add",
        schema::<AddInput>(),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            person_only(ctx)?;
            let input: AddInput = parse(input)?;
            let (basket, made) = match (&input.thread, &input.basket) {
                (Some(t), None) => (Basket::Thread(thread_of(t, "/thread")?), None),
                (None, Some(b)) => {
                    let (b, made) = store::ensure_basket_tx(ctx.conn, b).map_err(at("/basket"))?;
                    (Basket::Named(b.id), made.then_some(b.name))
                }
                _ => {
                    return Err(invalid(
                        "/",
                        "name exactly one basket: a `thread` (its own) or a named `basket`",
                    ))
                }
            };
            let stream_id = match (&input.stream, basket) {
                (Some(s), _) => Some(
                    bare_id::<StreamId>(s, oxplow_domain::uri::resources::STREAM, "/stream")?
                        .value(),
                ),
                (None, Basket::Thread(t)) => stream_of_thread(ctx.conn, t)?,
                (None, Basket::Named(_)) => None,
            };
            let id = store::add_tx(
                ctx.conn,
                basket,
                &NewItem {
                    stream_id,
                    source_uri: input.uri.filter(|u| !u.trim().is_empty()),
                    label: input.label.filter(|l| !l.trim().is_empty()),
                    text: input.text.filter(|t| !t.trim().is_empty()),
                    around: input.around.filter(|a| !a.trim().is_empty()),
                    note: note_of(input.note),
                },
            )
            .map_err(at("/"))?;
            // Undoing an add that made its basket takes the basket too.
            let inverse = match made {
                Some(name) => CommandCall {
                    name: DELETE_BASKET.into(),
                    input: json!({ "basket": name }),
                },
                None => CommandCall {
                    name: REMOVE.into(),
                    input: json!({ "item": id }),
                },
            };
            Ok(HandlerOutput {
                result: json!({ "item": id }),
                inverse: Some(inverse),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// `remove { item }`.
pub fn remove_op() -> Op {
    op(
        "remove",
        schema::<ItemInput>(),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            person_only(ctx)?;
            let input: ItemInput = parse(input)?;
            let inverse = match store::get_tx(ctx.conn, input.item)? {
                Some(item) => Some(re_add(ctx.conn, &item)?),
                None => None,
            };
            let removed = store::remove_tx(ctx.conn, input.item)?;
            Ok(HandlerOutput {
                result: json!({ "removed": removed.is_some() }),
                inverse,
                ..HandlerOutput::default()
            })
        })),
    )
}

/// `note { item, note? }`: set or clear an item's note; undoes to the note
/// it had.
pub fn note_op() -> Op {
    op(
        "note",
        schema::<NoteInput>(),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            person_only(ctx)?;
            let input: NoteInput = parse(input)?;
            let note = note_of(input.note);
            let before = store::set_note_tx(ctx.conn, input.item, note.as_deref())
                .map_err(at("/item"))?
                .ok_or_else(|| invalid("/item", format!("no context item {}", input.item)))?;
            Ok(HandlerOutput {
                result: json!({ "item": input.item, "note": note }),
                inverse: (before.note != note).then(|| CommandCall {
                    name: NOTE.into(),
                    input: json!({ "item": input.item, "note": before.note }),
                }),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// `move { item, thread | basket }`: into another basket; undoes to the one
/// it was in.
pub fn move_op() -> Op {
    op(
        "move",
        schema::<MoveInput>(),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            person_only(ctx)?;
            let input: MoveInput = parse(input)?;
            let to = which(ctx.conn, input.thread.as_deref(), input.basket.as_deref())?;
            let before = store::move_tx(ctx.conn, input.item, to)?
                .ok_or_else(|| invalid("/item", format!("no context item {}", input.item)))?;
            let mut back = naming(ctx.conn, before.basket())?;
            back["item"] = json!(input.item);
            Ok(HandlerOutput {
                result: json!({ "item": input.item }),
                inverse: (before.basket() != to).then(|| CommandCall {
                    name: MOVE.into(),
                    input: back,
                }),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// `clear { thread | basket }`: empties it; undoing puts everything back.
pub fn clear_op() -> Op {
    op(
        "clear",
        schema::<WhichInput>(),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            person_only(ctx)?;
            let input: WhichInput = parse(input)?;
            let basket = which(ctx.conn, input.thread.as_deref(), input.basket.as_deref())?;
            let items = store::items_tx(ctx.conn, basket)?;
            let inverse = sequence(
                items
                    .iter()
                    .map(|i| re_add(ctx.conn, i))
                    .collect::<Result<_, _>>()?,
            );
            let cleared = store::clear_tx(ctx.conn, basket)?;
            Ok(HandlerOutput {
                result: json!({ "cleared": cleared.len() }),
                inverse,
                ..HandlerOutput::default()
            })
        })),
    )
}

/// The call that puts `basket`'s attachment back to `thread` (none:
/// detached).
fn re_attach(basket: &str, thread: Option<i64>) -> CommandCall {
    match thread {
        Some(t) => CommandCall {
            name: ATTACH.into(),
            input: json!({ "basket": basket, "thread": ThreadId::new(t).to_string() }),
        },
        None => CommandCall {
            name: DETACH.into(),
            input: json!({ "basket": basket }),
        },
    }
}

/// `attach { basket, thread }`: the thread's next prompt carries the
/// basket, then empties and detaches it. A basket is attached to one
/// thread at a time.
pub fn attach_op() -> Op {
    op(
        "attach",
        schema::<AttachInput>(),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            person_only(ctx)?;
            let input: AttachInput = parse(input)?;
            let basket = named(ctx.conn, &input.basket, "/basket")?;
            let thread = thread_of(&input.thread, "/thread")?;
            let before = store::attach_tx(ctx.conn, basket.id, Some(thread))?;
            Ok(HandlerOutput {
                result: json!({ "basket": basket.name, "thread": thread.to_string() }),
                inverse: (before != Some(thread.value())).then(|| re_attach(&basket.name, before)),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// `detach { basket }`: no thread's next prompt carries it.
pub fn detach_op() -> Op {
    op(
        "detach",
        schema::<BasketInput>(),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            person_only(ctx)?;
            let input: BasketInput = parse(input)?;
            let basket = named(ctx.conn, &input.basket, "/basket")?;
            let before = store::attach_tx(ctx.conn, basket.id, None)?;
            Ok(HandlerOutput {
                result: json!({ "basket": basket.name }),
                inverse: before.map(|t| re_attach(&basket.name, Some(t))),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// `create_basket { name }`.
pub fn create_basket_op() -> Op {
    op(
        "create_basket",
        schema::<NameInput>(),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            person_only(ctx)?;
            let input: NameInput = parse(input)?;
            let basket = store::create_basket_tx(ctx.conn, &input.name).map_err(at("/name"))?;
            Ok(HandlerOutput {
                result: json!({ "basket": basket.name }),
                inverse: Some(CommandCall {
                    name: DELETE_BASKET.into(),
                    input: json!({ "basket": basket.name }),
                }),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// `rename_basket { basket, name }`.
pub fn rename_basket_op() -> Op {
    op(
        "rename_basket",
        schema::<RenameInput>(),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            person_only(ctx)?;
            let input: RenameInput = parse(input)?;
            let basket = named(ctx.conn, &input.basket, "/basket")?;
            store::rename_basket_tx(ctx.conn, basket.id, &input.name).map_err(at("/name"))?;
            let now = store::get_basket_tx(ctx.conn, basket.id)?
                .expect("the basket just renamed")
                .name;
            Ok(HandlerOutput {
                result: json!({ "basket": now }),
                inverse: (now != basket.name).then(|| CommandCall {
                    name: RENAME_BASKET.into(),
                    input: json!({ "basket": now, "name": basket.name }),
                }),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// `delete_basket { basket }`: the basket and what is in it; undoing makes
/// it again with its items and its attachment.
pub fn delete_basket_op() -> Op {
    op(
        "delete_basket",
        schema::<BasketInput>(),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            person_only(ctx)?;
            let input: BasketInput = parse(input)?;
            let basket = named(ctx.conn, &input.basket, "/basket")?;
            let mut calls = vec![CommandCall {
                name: CREATE_BASKET.into(),
                input: json!({ "name": basket.name }),
            }];
            for item in store::items_tx(ctx.conn, Basket::Named(basket.id))? {
                calls.push(re_add(ctx.conn, &item)?);
            }
            if let Some(t) = basket.attached_thread_id {
                calls.push(re_attach(&basket.name, Some(t)));
            }
            let (_, items) =
                store::delete_basket_tx(ctx.conn, basket.id)?.expect("the basket just read");
            Ok(HandlerOutput {
                result: json!({ "deleted": basket.name, "items": items.len() }),
                inverse: sequence(calls),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// The context basket operations, for the bus.
pub fn ops() -> Vec<Op> {
    vec![
        add_op(),
        remove_op(),
        note_op(),
        move_op(),
        clear_op(),
        attach_op(),
        detach_op(),
        create_basket_op(),
        rename_basket_op(),
        delete_basket_op(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::services_with_thread;
    use oxplow_domain::Actor;

    async fn run(
        fx: &crate::test_fixtures::ThreadFixture,
        name: &str,
        input: Value,
    ) -> oxplow_domain::CommandOutcome {
        fx.svc
            .commands
            .run(&Actor::Human, name, input, false)
            .await
            .unwrap()
    }

    async fn undo(fx: &crate::test_fixtures::ThreadFixture, run: &oxplow_domain::CommandOutcome) {
        fx.svc
            .commands
            .undo(&Actor::Human, run.audit_id.unwrap(), false)
            .await
            .unwrap();
    }

    async fn items(
        fx: &crate::test_fixtures::ThreadFixture,
        basket: Option<&str>,
    ) -> Vec<ContextItem> {
        let (thread, name) = (fx.thread, basket.map(str::to_string));
        fx.svc
            .db
            .read(move |c| {
                let basket = match name {
                    None => Basket::Thread(thread),
                    Some(n) => match store::basket_by_name_tx(c, &n)? {
                        Some(b) => Basket::Named(b.id),
                        None => return Ok(Vec::new()),
                    },
                };
                store::items_tx(c, basket)
            })
            .await
            .unwrap()
    }

    async fn basket(fx: &crate::test_fixtures::ThreadFixture, name: &str) -> Option<NamedBasket> {
        let name = name.to_string();
        fx.svc
            .db
            .read(move |c| store::basket_by_name_tx(c, &name))
            .await
            .unwrap()
    }

    fn uris(items: &[ContextItem]) -> Vec<String> {
        items.iter().filter_map(|i| i.source_uri.clone()).collect()
    }

    /// A person adds a page and a piece of text to the thread's own basket,
    /// each with the thread's stream; undoing the second takes it back out.
    #[tokio::test]
    async fn add_and_undo() {
        let fx = services_with_thread().await;
        let thread = fx.thread.to_string();
        run(
            &fx,
            ADD,
            json!({ "thread": thread, "uri": "oxplow:oxplow/commit?sha=abc", "label": "abc" }),
        )
        .await;
        let text = run(
            &fx,
            ADD,
            json!({ "thread": thread, "uri": "oxplow:oxplow/file?path=a.rs#lines=1-2", "text": "fn a() {}" }),
        )
        .await;
        let own = items(&fx, None).await;
        assert_eq!(
            uris(&own),
            [
                "oxplow:oxplow/commit?sha=abc",
                "oxplow:oxplow/file?path=a.rs#lines=1-2"
            ]
        );
        assert!(own.iter().all(|i| i.stream_id.is_some()), "{own:?}");
        undo(&fx, &text).await;
        assert_eq!(items(&fx, None).await.len(), 1);
    }

    /// An add names exactly one basket.
    #[tokio::test]
    async fn an_add_names_one_basket() {
        let fx = services_with_thread().await;
        for input in [
            json!({ "uri": "oxplow:oxplow/commit?sha=abc" }),
            json!({ "thread": fx.thread.to_string(), "basket": "Auth", "uri": "oxplow:oxplow/commit?sha=abc" }),
        ] {
            let err = fx
                .svc
                .commands
                .run(&Actor::Human, ADD, input, false)
                .await
                .unwrap_err();
            assert!(err.to_string().contains("exactly one basket"), "{err}");
        }
    }

    /// An add to a basket that isn't there makes it, with the stream it
    /// names; undoing that add takes the basket too, and undoing an add to
    /// a basket that was there takes only the item.
    #[tokio::test]
    async fn an_add_makes_its_named_basket() {
        let fx = services_with_thread().await;
        let first = run(
            &fx,
            ADD,
            json!({ "basket": "Auth", "stream": "str1", "uri": "oxplow:oxplow/file?path=a.rs" }),
        )
        .await;
        let in_auth = items(&fx, Some("Auth")).await;
        assert_eq!(in_auth.len(), 1);
        assert_eq!(in_auth[0].stream_id, Some(1));
        let second = run(
            &fx,
            ADD,
            json!({ "basket": "auth", "uri": "oxplow:oxplow/commit?sha=b" }),
        )
        .await;
        assert_eq!(items(&fx, Some("Auth")).await.len(), 2);
        undo(&fx, &second).await;
        assert_eq!(items(&fx, Some("Auth")).await.len(), 1);
        undo(&fx, &first).await;
        assert!(basket(&fx, "Auth").await.is_none());
    }

    /// Clearing empties a basket; undoing it puts everything back where it was.
    #[tokio::test]
    async fn clear_and_undo() {
        let fx = services_with_thread().await;
        for n in 0..2 {
            run(&fx, ADD, json!({ "basket": "Auth", "stream": "str1", "uri": format!("oxplow:oxplow/knowledge?id=p{n}&store=oxwiki") })).await;
        }
        let cleared = run(&fx, CLEAR, json!({ "basket": "Auth" })).await;
        assert_eq!(cleared.result["cleared"], json!(2));
        assert!(items(&fx, Some("Auth")).await.is_empty());
        undo(&fx, &cleared).await;
        let back = items(&fx, Some("Auth")).await;
        assert_eq!(back.len(), 2);
        assert!(back.iter().all(|i| i.stream_id == Some(1)));
    }

    /// A note rides an add; `note` changes it and undoes to the note
    /// before; removing an item and undoing that keeps its note and the
    /// lines around its text.
    #[tokio::test]
    async fn an_items_note_is_set_on_add_changed_and_undone() {
        let fx = services_with_thread().await;
        let thread = fx.thread.to_string();
        let added = run(
            &fx,
            ADD,
            json!({ "thread": thread, "uri": "oxplow:oxplow/file?path=a.rs#lines=1-2",
                    "text": "fn a() {}", "around": "// a\nfn a() {}\n",
                    "note": "  handle the empty case  " }),
        )
        .await;
        let item = added.result["item"].as_i64().unwrap();
        let note = || async { items(&fx, None).await.pop().and_then(|i| i.note) };
        assert_eq!(note().await.as_deref(), Some("handle the empty case"));
        let changed = run(&fx, NOTE, json!({ "item": item, "note": "rename it" })).await;
        assert_eq!(note().await.as_deref(), Some("rename it"));
        undo(&fx, &changed).await;
        assert_eq!(note().await.as_deref(), Some("handle the empty case"));
        run(&fx, NOTE, json!({ "item": item, "note": " " })).await;
        assert_eq!(note().await, None, "an empty note clears it");

        run(&fx, NOTE, json!({ "item": item, "note": "keep me" })).await;
        let removed = run(&fx, REMOVE, json!({ "item": item })).await;
        undo(&fx, &removed).await;
        let back = items(&fx, None).await;
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].note.as_deref(), Some("keep me"));
        assert_eq!(back[0].around.as_deref(), Some("// a\nfn a() {}\n"));
    }

    /// An item moves from the thread's basket to a named one and back on undo.
    #[tokio::test]
    async fn move_and_undo() {
        let fx = services_with_thread().await;
        run(&fx, CREATE_BASKET, json!({ "name": "Auth" })).await;
        let added = run(
            &fx,
            ADD,
            json!({ "thread": fx.thread.to_string(), "uri": "oxplow:oxplow/commit?sha=a" }),
        )
        .await;
        let item = added.result["item"].as_i64().unwrap();
        let moved = run(&fx, MOVE, json!({ "item": item, "basket": "Auth" })).await;
        assert!(items(&fx, None).await.is_empty());
        assert_eq!(items(&fx, Some("Auth")).await.len(), 1);
        undo(&fx, &moved).await;
        assert_eq!(items(&fx, None).await.len(), 1);
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                MOVE,
                json!({ "item": item, "basket": "Nope" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no basket named"), "{err}");
    }

    /// Attach and detach undo to the attachment before.
    #[tokio::test]
    async fn attach_detach_and_undo() {
        let fx = services_with_thread().await;
        let thread = fx.thread.to_string();
        run(&fx, CREATE_BASKET, json!({ "name": "Auth" })).await;
        let attached = run(&fx, ATTACH, json!({ "basket": "Auth", "thread": thread })).await;
        assert_eq!(
            basket(&fx, "Auth").await.unwrap().attached_thread_id,
            Some(fx.thread.value())
        );
        undo(&fx, &attached).await;
        assert_eq!(basket(&fx, "Auth").await.unwrap().attached_thread_id, None);
        run(&fx, ATTACH, json!({ "basket": "Auth", "thread": thread })).await;
        let detached = run(&fx, DETACH, json!({ "basket": "Auth" })).await;
        assert_eq!(basket(&fx, "Auth").await.unwrap().attached_thread_id, None);
        undo(&fx, &detached).await;
        assert_eq!(
            basket(&fx, "Auth").await.unwrap().attached_thread_id,
            Some(fx.thread.value())
        );
    }

    /// A basket is renamed and back; deleting an attached basket with items
    /// takes them, and undoing it makes it again with its items, their
    /// streams and its attachment.
    #[tokio::test]
    async fn rename_and_delete_with_undo() {
        let fx = services_with_thread().await;
        run(&fx, ADD, json!({ "basket": "Auth", "stream": "str1", "uri": "oxplow:oxplow/commit?sha=a", "note": "look" })).await;
        let renamed = run(
            &fx,
            RENAME_BASKET,
            json!({ "basket": "Auth", "name": "Login" }),
        )
        .await;
        assert!(basket(&fx, "Auth").await.is_none());
        undo(&fx, &renamed).await;
        assert!(basket(&fx, "Auth").await.is_some());
        run(
            &fx,
            ATTACH,
            json!({ "basket": "Auth", "thread": fx.thread.to_string() }),
        )
        .await;
        let deleted = run(&fx, DELETE_BASKET, json!({ "basket": "Auth" })).await;
        assert!(basket(&fx, "Auth").await.is_none());
        undo(&fx, &deleted).await;
        let back = basket(&fx, "Auth").await.unwrap();
        assert_eq!(back.attached_thread_id, Some(fx.thread.value()));
        let in_it = items(&fx, Some("Auth")).await;
        assert_eq!(uris(&in_it), ["oxplow:oxplow/commit?sha=a"]);
        assert_eq!(
            (in_it[0].stream_id, in_it[0].note.as_deref()),
            (Some(1), Some("look"))
        );
    }

    /// The baskets are the person's: an agent's add is refused.
    #[tokio::test]
    async fn an_agent_may_not_add() {
        let fx = services_with_thread().await;
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
                json!({ "thread": fx.thread.to_string(), "uri": "oxplow:oxplow/knowledge?id=x&store=oxwiki" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("the person collects"), "{err}");
    }
}
