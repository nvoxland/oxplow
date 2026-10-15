//! `knowledge.read` operations: the comments people anchored in pages
//! (`oxplow.knowledge.comments`) and how far a wiki page's pinned file ref
//! has drifted from the file (`oxplow.knowledge.drift`).

use oxplow_domain::comment::CommentThread;
use oxplow_domain::stores::CommentStore;
use oxplow_domain::{CommandError, CommentStatus, StreamId, ThreadId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::super::ops::Op;
use super::super::util::{failed, invalid, ServicesSlot};
use super::ids::{expect_id_kind, read_op, result_of};
use crate::ref_resolver::{self, RefSummary};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListCommentsParams {
    /// `"thread"` (id = `thr…`) or `"stream"` (id = `str…`). Optional —
    /// when omitted it's inferred from `id`'s prefix.
    pub scope: Option<String>,
    /// The thread (`thr…`) or stream (`str…`) id whose comments to list.
    pub id: Option<String>,
    /// Filter: `"all"` (default), `"open"`, or `"needs_response"`.
    pub status: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct WikiRefDriftParams {
    /// Wiki slug (without `.md`).
    pub slug: String,
    /// Repo-relative file path referenced by the page.
    pub path: String,
}

/// A comment thread with the typed context it was anchored in resolved,
/// so the agent sees *what the highlighted thing is*: `primary` is the
/// target (a task, a file…), `context_chain` the ancestor regions
/// (innermost→outermost), `referenced` the canonical refs found inside
/// the selection itself.
#[derive(Debug, Serialize)]
struct EnrichedCommentThread {
    thread: CommentThread,
    primary: RefSummary,
    context_chain: Vec<RefSummary>,
    referenced: Vec<RefSummary>,
}

/// The resolved target of a `comments` call — a single thread or a whole
/// stream (workspace).
#[derive(Debug, PartialEq, Eq)]
enum CommentScope {
    Thread(ThreadId),
    Stream(StreamId),
}

const THREAD: (&str, &str) = ("thr", "thread id (thr…)");
const STREAM: (&str, &str) = ("str", "stream id (str…)");

fn thread_id(value: &str) -> Result<ThreadId, CommandError> {
    ThreadId::try_from_str(value)
        .ok_or_else(|| invalid("/id", format!("invalid thread id `{value}`")))
}

fn stream_id(value: &str) -> Result<StreamId, CommandError> {
    StreamId::try_from_str(value)
        .ok_or_else(|| invalid("/id", format!("invalid stream id `{value}`")))
}

/// Resolve the `(scope, id)` pair `comments` was called with into a
/// concrete [`CommentScope`].
///
/// `scope` is optional: when omitted it's inferred from `id`'s prefix
/// (`thr…` → thread, `str…` → stream). This keeps weaker models from
/// thrashing on a missing-field error when the id alone already
/// determines the scope. When `scope` *is* given it's honored and the id
/// must match it. Every failure names the fix.
fn resolve_comment_scope(
    scope: Option<&str>,
    id: Option<&str>,
) -> Result<CommentScope, CommandError> {
    let id = id.map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| {
        invalid(
            "/id",
            "list_comments: pass `id` — a thread id (`thr…`) or stream id (`str…`)",
        )
    })?;

    // Normalize an explicit scope up front so an unknown string is caught
    // before we look at the id.
    let scope = scope.map(str::trim).filter(|s| !s.is_empty());
    let want = match scope {
        Some("thread") => Some(THREAD),
        Some("stream") => Some(STREAM),
        Some(other) => {
            return Err(invalid(
                "/scope",
                format!(
                    "list_comments: `scope` must be \"thread\" or \"stream\", got `{other}` \
                     (or omit it and I'll infer it from `id`)"
                ),
            ));
        }
        None => None,
    };

    match want {
        // Explicit scope: validate the id matches, then build it.
        Some(THREAD) => {
            expect_id_kind("list_comments", "/id", id, THREAD.0, THREAD.1)?;
            Ok(CommentScope::Thread(thread_id(id)?))
        }
        Some(_) => {
            expect_id_kind("list_comments", "/id", id, STREAM.0, STREAM.1)?;
            Ok(CommentScope::Stream(stream_id(id)?))
        }
        // No scope: infer it from the id's prefix.
        None => match id.parse::<oxplow_domain::AnyId>() {
            Ok(any) if any.kind.prefix() == THREAD.0 => Ok(CommentScope::Thread(thread_id(id)?)),
            Ok(any) if any.kind.prefix() == STREAM.0 => Ok(CommentScope::Stream(stream_id(id)?)),
            _ => Err(invalid(
                "/scope",
                format!(
                    "list_comments: couldn't infer `scope` from id `{id}` — pass scope \
                     \"thread\" (with a `thr…` id) or \"stream\" (with a `str…` id)"
                ),
            )),
        },
    }
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    vec![
        read_op(
            "knowledge.read",
            "comments",
            services.clone(),
            |svc, _, p: ListCommentsParams| async move {
                let threads = match resolve_comment_scope(p.scope.as_deref(), p.id.as_deref())? {
                    CommentScope::Thread(id) => svc
                        .comment_store
                        .list_for_thread(&id)
                        .await
                        .map_err(failed)?,
                    CommentScope::Stream(id) => svc
                        .comment_store
                        .list_for_stream(&id)
                        .await
                        .map_err(failed)?,
                };
                let status = p.status.as_deref().unwrap_or("all");
                let filtered: Vec<_> = threads
                    .into_iter()
                    .filter(|t| match status {
                        "needs_response" => t.needs_response(),
                        "open" => t.comment.status == CommentStatus::Open,
                        _ => true,
                    })
                    .collect();
                let mut enriched = Vec::with_capacity(filtered.len());
                for t in filtered {
                    let primary = ref_resolver::resolve_ref(
                        &svc,
                        &t.comment.target_kind,
                        &t.comment.target_id,
                    )
                    .await;
                    let context_chain =
                        ref_resolver::resolve_refs(&svc, &t.comment.context_chain).await;
                    let referenced =
                        ref_resolver::resolve_refs(&svc, &t.comment.referenced_refs).await;
                    enriched.push(EnrichedCommentThread {
                        thread: t,
                        primary,
                        context_chain,
                        referenced,
                    });
                }
                result_of(enriched)
            },
        ),
        read_op(
            "knowledge.read",
            "drift",
            services,
            |svc, _, p: WikiRefDriftParams| async move {
                let drift = crate::wiki_drift::compute_wiki_ref_drift(
                    &svc.page_ref_store,
                    &svc.snapshot_store,
                    &svc.snapshot_content,
                    &svc.layout.project_dir,
                    &p.slug,
                    &p.path,
                )
                .await
                .map_err(failed)?;
                result_of(drift)
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::Actor;
    use serde_json::json;

    #[test]
    fn the_scope_is_inferred_from_the_ids_prefix() {
        assert_eq!(
            resolve_comment_scope(None, Some("thr7")).unwrap(),
            CommentScope::Thread(ThreadId::new(7))
        );
        assert_eq!(
            resolve_comment_scope(None, Some("str3")).unwrap(),
            CommentScope::Stream(StreamId::new(3))
        );
    }

    #[test]
    fn an_explicit_scope_is_honored_and_must_match_the_id() {
        assert_eq!(
            resolve_comment_scope(Some("thread"), Some("thr1")).unwrap(),
            CommentScope::Thread(ThreadId::new(1))
        );
        assert_eq!(
            resolve_comment_scope(Some("stream"), Some("str1")).unwrap(),
            CommentScope::Stream(StreamId::new(1))
        );
        // scope says thread but a stream id was passed → friendly error.
        let msg = resolve_comment_scope(Some("thread"), Some("str1"))
            .unwrap_err()
            .to_string();
        assert!(msg.contains("list_comments"), "tool name missing: {msg}");
        assert!(msg.contains("thread id"), "expected kind missing: {msg}");
    }

    #[test]
    fn a_missing_or_uninferable_id_or_unknown_scope_says_the_fix() {
        let msg = resolve_comment_scope(Some("thread"), None)
            .unwrap_err()
            .to_string();
        assert!(
            msg.contains("list_comments") && msg.contains("`id`"),
            "{msg}"
        );
        // Blank/whitespace id is treated the same as missing.
        assert!(resolve_comment_scope(None, Some("   ")).is_err());
        // A valid-but-wrong-kind id (task) can't pick a scope.
        let msg = resolve_comment_scope(None, Some("tsk9"))
            .unwrap_err()
            .to_string();
        assert!(msg.contains("infer") && msg.contains("scope"), "{msg}");
        assert!(resolve_comment_scope(None, Some("nonsense")).is_err());
        let msg = resolve_comment_scope(Some("workspace"), Some("str1"))
            .unwrap_err()
            .to_string();
        assert!(
            msg.contains("workspace") && msg.contains("\"thread\"") && msg.contains("\"stream\""),
            "{msg}"
        );
    }

    /// A page's file ref with no pin reads `no_pin`; a path the page
    /// doesn't reference reads `not_a_ref`.
    #[tokio::test]
    async fn drift_reports_status_per_ref_through_the_bus() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let svc = crate::Services::in_memory(dir.path()).await.unwrap();
        let wiki = dir.path().join(".oxplow").join("wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        std::fs::write(wiki.join("intro.md"), "see [[crates/foo.rs]]").unwrap();
        crate::wiki_pages::sync_page(&svc.db, &svc.vocabulary, dir.path(), "intro")
            .await
            .unwrap();
        let actor = Actor::Agent {
            session_id: None,
            thread_id: None,
            stream_id: None,
        };
        let status = |path: &str| {
            let svc = svc.clone();
            let actor = actor.clone();
            let input = json!({ "slug": "intro", "path": path });
            async move {
                svc.commands
                    .run(&actor, "oxplow.knowledge.drift", input, false)
                    .await
                    .unwrap()
                    .result["status"]
                    .clone()
            }
        };
        assert_eq!(status("crates/foo.rs").await, "no_pin");
        assert_eq!(status("crates/zzz.rs").await, "not_a_ref");
    }

    /// Comments list through the bus with their anchor context resolved;
    /// status filters apply.
    #[tokio::test]
    async fn comments_list_for_a_stream_through_the_bus() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let svc = crate::Services::in_memory(dir.path()).await.unwrap();
        let stream = svc.streams.list_streams().await.unwrap()[0].id;
        svc.commands
            .run(
                &Actor::Human,
                "oxplow.knowledge.add_comment",
                json!({
                    "stream": format!("stream:{stream}"),
                    "target": { "kind": "file", "id": "src/app.rs" },
                    "quote": "the highlighted text",
                    "selectors_json": "[]",
                    "context_chain": [],
                    "referenced_refs": [{ "kind": "file", "id": "src/b.rs" }],
                    "intent": "followup",
                    "body": "what about this?",
                }),
                false,
            )
            .await
            .unwrap();
        let actor = Actor::Agent {
            session_id: None,
            thread_id: None,
            stream_id: None,
        };
        let out = svc
            .commands
            .run(
                &actor,
                "oxplow.knowledge.comments",
                json!({ "id": stream.to_string() }),
                false,
            )
            .await
            .unwrap()
            .result;
        assert_eq!(out[0]["thread"]["comment"]["quote"], "the highlighted text");
        assert_eq!(out[0]["referenced"][0]["id"], "src/b.rs");
        let resolved = svc
            .commands
            .run(
                &actor,
                "oxplow.knowledge.comments",
                json!({ "id": stream.to_string(), "status": "needs_response" }),
                false,
            )
            .await
            .unwrap()
            .result;
        // The human's own follow-up awaits a reply from the agent.
        assert_eq!(resolved.as_array().unwrap().len(), 1, "{resolved}");
    }
}
