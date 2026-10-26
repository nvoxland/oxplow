//! Bookmark operations (`bookmarks.write`; declared as commands in
//! `extensions/oxplow-foundation`): a person stars a page at a scope — the
//! thread they're in, its stream, or the project — or takes the star off.
//! `Tx` over the bookmark store's `_tx` cores; read back through
//! `v_bookmark`. The viewer is the thread (`oxplow:oxplow/thread?id=thr1`) the person is in, its
//! stream implied; a bookmark is visible once per viewer, so setting it at
//! another scope moves it. Each undoes to what the viewer saw before.

use std::sync::Arc;

use oxplow_db::bookmark_store::{remove_tx, set_tx, Bookmark, BookmarkScope, Viewer};
use oxplow_domain::{CommandCall, CommandError, StreamId, ThreadId};
use rusqlite::{Connection, OptionalExtension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::ops::Op;
use super::util::{bare_id, invalid, parse, schema};
use super::{Handler, HandlerOutput, TxCtx};

/// The capability its operations are of.
pub const CAPABILITY: &str = "bookmarks.write";
/// The commands that undo them.
pub const SET: &str = "oxplow.bookmark.set";
pub const REMOVE: &str = "oxplow.bookmark.remove";

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
    /// The page's canonical ref (its tab id, `oxplow:oxplow/page?page=git-dashboard`).
    pub uri: String,
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
    pub uri: String,
    /// The thread the person is in (`thr1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// The stream (`str1`), when there's no thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,
}

/// The viewer named by `thread` / `stream`: a thread's stream is its own.
fn viewer(
    conn: &Connection,
    thread: Option<&str>,
    stream: Option<&str>,
) -> Result<Viewer, CommandError> {
    let thread = thread
        .map(|t| bare_id::<ThreadId>(t, oxplow_domain::uri::resources::THREAD, "/thread"))
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
        (None, Some(s)) => Some(bare_id(
            s,
            oxplow_domain::uri::resources::STREAM,
            "/stream",
        )?),
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
                "uri": b.page_ref, "page_kind": b.page_kind, "label": b.label,
                "scope": b.scope, "thread": at["thread"], "stream": at["stream"],
            }),
        },
        None => CommandCall {
            name: REMOVE.into(),
            input: json!({ "uri": page_ref, "thread": at["thread"], "stream": at["stream"] }),
        },
    }
}

/// `set { ref, page_kind, label?, scope, thread?, stream? }`.
pub fn set_op() -> Op {
    Op::new(
        CAPABILITY,
        "set",
        schema::<SetInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let at = json!({ "thread": input.get("thread"), "stream": input.get("stream") });
            let input: SetInput = parse(input)?;
            let viewer = viewer(ctx.conn, input.thread.as_deref(), input.stream.as_deref())?;
            let bookmark = Bookmark {
                page_ref: input.uri.clone(),
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
                inverse: Some(restore(before, &input.uri, &at)),
                ..HandlerOutput::default()
            })
        })),
    )
    // The person's navigation, through a page too — never the agent's.
}

/// `remove { ref, thread?, stream? }`.
pub fn remove_op() -> Op {
    Op::new(
        CAPABILITY,
        "remove",
        schema::<RemoveInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let at = json!({ "thread": input.get("thread"), "stream": input.get("stream") });
            let input: RemoveInput = parse(input)?;
            let viewer = viewer(ctx.conn, input.thread.as_deref(), input.stream.as_deref())?;
            let removed = remove_tx(ctx.conn, viewer, &input.uri)?;
            Ok(HandlerOutput {
                result: json!({ "removed": removed.is_some() }),
                inverse: removed.is_some().then(|| restore(removed, &input.uri, &at)),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// The bookmark operations, for the bus.
pub fn ops() -> Vec<Op> {
    vec![set_op(), remove_op()]
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
                    "SELECT scope FROM bookmark WHERE uri = ?1",
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
            json!({ "uri": "oxplow:oxplow/page?page=git-dashboard", "page_kind": "git-dashboard",
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
            scope_of(&fx, "oxplow:oxplow/page?page=git-dashboard")
                .await
                .as_deref(),
            Some("project")
        );
        fx.svc
            .commands
            .undo(&Actor::Human, second.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(
            scope_of(&fx, "oxplow:oxplow/page?page=git-dashboard")
                .await
                .as_deref(),
            Some("thread")
        );
        fx.svc
            .commands
            .undo(&Actor::Human, first.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(
            scope_of(&fx, "oxplow:oxplow/page?page=git-dashboard").await,
            None
        );
    }

    /// An agent's bookmark runs.
    #[tokio::test]
    async fn an_agent_may_bookmark() {
        let fx = services_with_effort().await;
        let thread = fx.thread.to_string();
        let input = json!({ "uri": "oxplow:oxplow/file?path=src/b.rs", "page_kind": "file", "scope": "stream", "thread": thread });
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        fx.svc
            .commands
            .run(&agent, SET, input, false)
            .await
            .unwrap();
    }

    /// Removing undoes to where it was.
    #[tokio::test]
    async fn remove_undoes() {
        let fx = services_with_effort().await;
        let thread = fx.thread.to_string();
        let input = json!({ "uri": "oxplow:oxplow/file?path=src/a.rs", "page_kind": "file", "scope": "stream", "thread": thread });
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
                json!({ "uri": "oxplow:oxplow/file?path=src/a.rs", "thread": thread }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["removed"], json!(true));
        assert_eq!(
            scope_of(&fx, "oxplow:oxplow/file?path=src/a.rs").await,
            None
        );
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(
            scope_of(&fx, "oxplow:oxplow/file?path=src/a.rs")
                .await
                .as_deref(),
            Some("stream")
        );
    }
}
