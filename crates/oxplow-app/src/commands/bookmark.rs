//! Bookmark commands: a person stars a page at a scope — the
//! thread they're in, its stream, or the project — or takes the star off.
//! `Tx` over the bookmark store's `_tx` cores; read back through
//! `v_bookmark`. The viewer is the thread (`thr1`) the person is in, its
//! stream implied; a bookmark is visible once per viewer, so setting it at
//! another scope moves it. Each undoes to what the viewer saw before.

use std::sync::Arc;

use oxplow_db::bookmark_store::{remove_tx, set_tx, Bookmark, BookmarkScope, Viewer};
use oxplow_domain::{
    Atomicity, CommandCall, CommandEffect, CommandError, CommandSpec, Confirm, Invokers, Lifecycle,
    StreamId, ThreadId,
};
use rusqlite::{Connection, OptionalExtension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{Command, Handler, HandlerOutput, TxCtx};

pub const SET: &str = "oxplow.bookmark.set";
pub const REMOVE: &str = "oxplow.bookmark.remove";

/// A person, or a lens acting for one: bookmarks are the person's
/// navigation, not the agent's.
const PEOPLE: Invokers = Invokers {
    human: true,
    agent: false,
    lens: true,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScopeInput {
    Thread,
    Stream,
    Project,
}

impl From<ScopeInput> for BookmarkScope {
    fn from(s: ScopeInput) -> Self {
        match s {
            ScopeInput::Thread => BookmarkScope::Thread,
            ScopeInput::Stream => BookmarkScope::Stream,
            ScopeInput::Project => BookmarkScope::Project,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetInput {
    /// The page's canonical ref (its tab id, `page:git-dashboard`).
    #[serde(rename = "ref")]
    pub page_ref: String,
    /// Its page kind, for the icon.
    pub page_kind: String,
    /// Its title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub scope: ScopeInput,
    /// The thread the person is in (`thr1`); its stream is the stream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// The stream (`str1`), when there's no thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveInput {
    /// The page's canonical ref.
    #[serde(rename = "ref")]
    pub page_ref: String,
    /// The thread the person is in (`thr1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// The stream (`str1`), when there's no thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,
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

fn schema<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes")
}

/// The viewer named by `thread` / `stream`: a thread's stream is its own.
fn viewer(
    conn: &Connection,
    thread: Option<&str>,
    stream: Option<&str>,
) -> Result<Viewer, CommandError> {
    let thread = thread
        .map(|t| {
            ThreadId::try_from_str(t)
                .ok_or_else(|| invalid("/thread", format!("`{t}` isn't a thread id (thr…)")))
        })
        .transpose()?;
    let stream = match (thread, stream) {
        (Some(t), _) => {
            let s: Option<i64> = conn
                .query_row(
                    "SELECT stream_id FROM threads WHERE id = ?1",
                    [t.value()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| CommandError::from(oxplow_db::map_sql_err(e)))?;
            Some(StreamId::new(s.ok_or_else(|| {
                invalid("/thread", format!("no thread `{t}`"))
            })?))
        }
        (None, Some(s)) => Some(
            StreamId::try_from_str(s)
                .ok_or_else(|| invalid("/stream", format!("`{s}` isn't a stream id (str…)")))?,
        ),
        (None, None) => None,
    };
    Ok(Viewer { thread, stream })
}

/// What puts `before` back for the viewer: setting it again, or removing
/// what wasn't there.
fn restore(before: Option<Bookmark>, page_ref: &str, at: &Value) -> CommandCall {
    match before {
        Some(b) => CommandCall {
            name: SET.into(),
            input: json!({
                "ref": b.page_ref, "page_kind": b.page_kind, "label": b.label,
                "scope": b.scope, "thread": at["thread"], "stream": at["stream"],
            }),
        },
        None => CommandCall {
            name: REMOVE.into(),
            input: json!({ "ref": page_ref, "thread": at["thread"], "stream": at["stream"] }),
        },
    }
}

fn spec(name: &str, summary: &str, schema: Value) -> CommandSpec {
    CommandSpec {
        id: name.into(),
        summary: summary.into(),
        input_schema: schema,
        invokers: PEOPLE,
        confirm: Confirm::Never,
        undoable: true,
        lifecycle: Lifecycle::Stable,
        atomicity: Atomicity::Tx,
        effect: CommandEffect::Record,
        needs: Vec::new(),
    }
}

/// `bookmark.set { ref, page_kind, label?, scope, thread?, stream? }`.
pub fn set_command() -> Command {
    Command::new(
        spec(
            SET,
            "Bookmark a page at a scope (`thread`, `stream` or `project`), moving it there if it's bookmarked at another.",
            schema::<SetInput>(),
        ),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let at = json!({ "thread": input.get("thread"), "stream": input.get("stream") });
            let input: SetInput = parse(input)?;
            let viewer = viewer(ctx.conn, input.thread.as_deref(), input.stream.as_deref())?;
            let bookmark = Bookmark {
                page_ref: input.page_ref.clone(),
                page_kind: input.page_kind,
                label: input.label,
                scope: input.scope.into(),
            };
            let before = set_tx(ctx.conn, viewer, &bookmark).map_err(|e| match e {
                oxplow_domain::DomainError::Invalid(m) => invalid("/scope", m),
                other => CommandError::from(other),
            })?;
            Ok(HandlerOutput {
                result: serde_json::to_value(&bookmark).expect("a bookmark serializes"),
                inverse: Some(restore(before, &input.page_ref, &at)),
                ..HandlerOutput::default()
            })
        })),
    )
    .expect("oxplow.bookmark.set is a valid command")
}

/// `bookmark.remove { ref, thread?, stream? }`.
pub fn remove_command() -> Command {
    Command::new(
        spec(
            REMOVE,
            "Take a page's bookmark off, whichever scope it's at.",
            schema::<RemoveInput>(),
        ),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let at = json!({ "thread": input.get("thread"), "stream": input.get("stream") });
            let input: RemoveInput = parse(input)?;
            let viewer = viewer(ctx.conn, input.thread.as_deref(), input.stream.as_deref())?;
            let removed = remove_tx(ctx.conn, viewer, &input.page_ref)?;
            Ok(HandlerOutput {
                result: json!({ "removed": removed.is_some() }),
                inverse: removed
                    .is_some()
                    .then(|| restore(removed, &input.page_ref, &at)),
                ..HandlerOutput::default()
            })
        })),
    )
    .expect("oxplow.bookmark.remove is a valid command")
}

/// The bookmark commands, for the bus.
pub fn commands() -> Vec<Command> {
    vec![set_command(), remove_command()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::services_with_effort;
    use oxplow_domain::Actor;

    async fn scope_of(fx: &crate::test_fixtures::EffortFixture, page_ref: &str) -> Option<String> {
        let page_ref = page_ref.to_string();
        fx.svc
            .db
            .read(move |conn| {
                conn.query_row(
                    "SELECT scope FROM bookmark WHERE ref = ?1",
                    [page_ref],
                    |r| r.get(0),
                )
                .optional()
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
    }

    /// A person bookmarks a page, moves it to the project, and undoing
    /// walks it back: to the thread, then off.
    #[tokio::test]
    async fn set_moves_and_undo_walks_it_back() {
        let fx = services_with_effort().await;
        let thread = fx.thread.to_string();
        let set = |scope: &str| {
            json!({ "ref": "page:git-dashboard", "page_kind": "git-dashboard",
                    "label": "Git", "scope": scope, "thread": thread })
        };
        let first = fx
            .svc
            .commands
            .run(&Actor::Human, SET, set("thread"), false)
            .await
            .unwrap();
        let second = fx
            .svc
            .commands
            .run(&Actor::Human, SET, set("project"), false)
            .await
            .unwrap();
        assert_eq!(
            scope_of(&fx, "page:git-dashboard").await.as_deref(),
            Some("project")
        );
        fx.svc
            .commands
            .undo(&Actor::Human, second.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(
            scope_of(&fx, "page:git-dashboard").await.as_deref(),
            Some("thread")
        );
        fx.svc
            .commands
            .undo(&Actor::Human, first.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(scope_of(&fx, "page:git-dashboard").await, None);
    }

    /// Removing undoes to where it was; an agent can't bookmark.
    #[tokio::test]
    async fn remove_undoes_and_agents_cannot_bookmark() {
        let fx = services_with_effort().await;
        let thread = fx.thread.to_string();
        let input = json!({ "ref": "file:src/a.rs", "page_kind": "file", "scope": "stream", "thread": thread });
        let agent = Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        assert!(fx
            .svc
            .commands
            .run(&agent, SET, input.clone(), false)
            .await
            .is_err());
        fx.svc
            .commands
            .run(&Actor::Human, SET, input, false)
            .await
            .unwrap();
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                REMOVE,
                json!({ "ref": "file:src/a.rs", "thread": thread }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["removed"], json!(true));
        assert_eq!(scope_of(&fx, "file:src/a.rs").await, None);
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(
            scope_of(&fx, "file:src/a.rs").await.as_deref(),
            Some("stream")
        );
    }
}
