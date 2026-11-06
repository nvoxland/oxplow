//! Stream commands (P8.A4, `.context/commands.md`): a stream's lifecycle
//! — a new worktree (one of a branch, a new branch, or one already on
//! disk), archiving — and its
//! title and custom prompt. Creating, adopting and archiving reach the
//! VCS and the stream's capture service, so they are `External` and a
//! person's; archiving is destructive (it can remove the working copy).
//! Renaming and the prompt are `Tx` over the stream row. A stream is
//! named by ref (`oxplow:oxplow/stream?id=str1`).

use crate::commands::ops::Op;
use std::sync::Arc;

use oxplow_db::stream_store::{get_tx, upsert_tx};
use oxplow_domain::uri::build::stream_uri;
use oxplow_domain::{CommandCall, CommandError, Confirm, Stream, StreamId, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::thread::agent_scope;
use super::util::{bare_id, invalid, parse, schema, sql};
use super::{Handler, HandlerOutput, Invocation, TxCtx};

pub const CREATE: &str = "oxplow.stream.create";
pub const ARCHIVE: &str = "oxplow.stream.archive";
pub const RENAME: &str = "oxplow.stream.rename";
pub const SET_PROMPT: &str = "oxplow.stream.set_prompt";

// No docs on the variants: a documented variant is a `oneOf` of
// constants in the schema, and a plain `enum` is what a form draws as a
// select.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StreamSource {
    Existing,
    New,
    Worktree,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInput {
    /// The stream's name; its worktree's folder is made from it.
    pub title: String,
    /// Where its worktree comes from: `existing` (a branch that exists),
    /// `new` (a new branch forked from a start point) or `worktree` (one
    /// already on disk, adopted as it is: a person's).
    pub source: StreamSource,
    /// `source: existing`: the branch to put the stream on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_branch: Option<String>,
    /// `source: new`: the branch to create for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_branch: Option<String>,
    /// `source: new`: the branch or revision it forks from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_point: Option<String>,
    /// `source: worktree`: the path of an existing worktree of this
    /// repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArchiveInput {
    /// The stream (`str2`).
    pub stream: String,
    /// Also remove its working copy from disk.
    #[serde(default)]
    pub delete_worktree: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameInput {
    /// The stream (`str1`).
    pub stream: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetPromptInput {
    /// The stream (`str1`).
    pub stream: String,
    /// Appended to every agent prompt in this stream; empty (or only
    /// whitespace) or absent clears it.
    #[schemars(extend("contentMediaType" = "text/markdown"))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

/// What the `External` stream commands reach: the stream service (the VCS
/// and the rows), the capture registry, and what an archive cleans up.
#[derive(Clone)]
pub struct StreamDeps {
    pub streams: oxplow_session::StreamService,
    pub snapshot_captures: crate::snapshot_capture_registry::SnapshotCaptureRegistry,
    /// Each stream's one watch, which starts and stops with it.
    pub stream_watches: crate::stream_watch::StreamWatchRegistry,
    /// The snapshot implementations: an archive's end takes are the active
    /// one's marks.
    pub snapshots: Arc<oxplow_domain::snapshot::SnapshotRegistry>,
    pub ref_moves: crate::ref_moves::RefMoves,
    pub threads: Arc<oxplow_db::SqliteThreadStore>,
    pub sessions: Arc<oxplow_db::SqliteAgentSessionStore>,
    pub processes: crate::agent_sessions::SessionProcesses,
    pub log: Arc<oxplow_db::SqliteEventLogStore>,
    pub search: Arc<oxplow_db::SqliteSearchStore>,
    pub worktrees: Arc<crate::worktrees::WorktreeRouter>,
    pub efforts: Arc<oxplow_db::SqliteEffortStore>,
    pub db: oxplow_db::Database,
    /// A stream's terminals close with it: their shells end, their rows go.
    pub terminals: Arc<oxplow_db::SqliteTerminalStore>,
    pub terminal_sessions: crate::terminal_sessions::TerminalSessionRegistry,
}

fn session(e: oxplow_session::SessionError) -> CommandError {
    use oxplow_session::SessionError as E;
    match e {
        E::DuplicateWorktreeSlug(_) => invalid("/title", e.to_string()),
        E::Storage(oxplow_domain::DomainError::NotFound) => invalid("/stream", "no such stream"),
        other => CommandError::Failed {
            message: other.to_string(),
        },
    }
}

fn load(ctx: &TxCtx<'_>, id: StreamId) -> Result<Stream, CommandError> {
    get_tx(ctx.conn, id)
        .map_err(sql)?
        .filter(|s| s.archived_at.is_none())
        .ok_or_else(|| invalid("/stream", format!("no stream `{}`", stream_uri(id))))
}

fn result(stream: &Stream) -> HandlerOutput {
    HandlerOutput {
        result: serde_json::to_value(stream).expect("a stream serializes"),
        ..HandlerOutput::default()
    }
}

/// A new stream's watch starts, and its capture with it, so its edits land in
/// snapshots and reach the UI.
fn start_capture(deps: &StreamDeps, stream: &Stream) {
    deps.stream_watches.register(stream);
}

/// A folder name for a stream titled `title`: lowercase words joined by
/// hyphens, accents dropped; a title with no words gets a stamp. The
/// slug is fixed when the worktree is made, so it stays conservative.
fn slug_of(title: &str) -> String {
    let mut slug = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        // Latin letters with a mark (é, ü) fold to their base letter.
        let c = match c {
            'à'..='å' => 'a',
            'ç' => 'c',
            'è'..='ë' => 'e',
            'ì'..='ï' => 'i',
            'ñ' => 'n',
            'ò'..='ö' => 'o',
            'ù'..='ü' => 'u',
            'ý' | 'ÿ' => 'y',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() {
        format!("stream-{}", Timestamp::now().unix_ms())
    } else {
        slug
    }
}

/// `stream.create { title, source, … }` — a stream on a worktree: of an
/// existing branch, of a new one forked from a start point, or adopted
/// from disk. Each source names the fields it needs. The worktree's folder
/// comes from the title. Anyone's: agents set up parallel work too.
/// It reaches the VCS and starts the stream's capture service.
pub fn create_op(deps: StreamDeps) -> Op {
    Op::new(
        "worktrees.write",
        "create",
        schema::<CreateInput>(),
        false,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let deps = deps.clone();
            Box::pin(async move {
                let input: CreateInput = parse(input)?;
                let need = |value: &Option<String>, field: &str| -> Result<String, CommandError> {
                    value
                        .as_deref()
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                        .map(str::to_string)
                        .ok_or_else(|| {
                            invalid(
                                &format!("/{field}"),
                                format!("`source: {}` needs `{field}`", source_name(input.source)),
                            )
                        })
                };
                let title = input.title.trim().to_string();
                if title.is_empty() {
                    return Err(invalid("/title", "a stream needs a name"));
                }
                let stream = match input.source {
                    StreamSource::Existing => {
                        let branch = need(&input.existing_branch, "existing_branch")?;
                        deps.streams
                            .create_worktree(&slug_of(&title), title, branch.clone(), branch)
                            .await
                    }
                    StreamSource::New => {
                        let branch = need(&input.new_branch, "new_branch")?;
                        let from = need(&input.start_point, "start_point")?;
                        deps.streams
                            .create_worktree(&slug_of(&title), title, branch, from)
                            .await
                    }
                    StreamSource::Worktree => {
                        let path = need(&input.worktree, "worktree")?;
                        deps.streams
                            .adopt_worktree(std::path::PathBuf::from(path), title)
                            .await
                    }
                }
                .map_err(session)?;
                start_capture(&deps, &stream);
                Ok(result(&stream))
            })
        })),
    )
}

fn source_name(source: StreamSource) -> &'static str {
    match source {
        StreamSource::Existing => "existing",
        StreamSource::New => "new",
        StreamSource::Worktree => "worktree",
    }
}

/// `stream.archive { stream, delete_worktree? }`: refused while an agent
/// runs in one of its threads; its threads go with it — their open efforts
/// close at a snapshot taken first, their agent sessions close
/// (`stream_archived`) and their processes stop — and its working copy
/// when asked.
/// Destructive: a person confirms it.
pub fn archive_op(deps: StreamDeps) -> Op {
    Op::new(
        "worktrees.write",
        "archive",
        schema::<ArchiveInput>(),
        false,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let deps = deps.clone();
            Box::pin(async move {
                use crate::agent_status_derive::{derive_session_status, recent_activity};
                use oxplow_domain::stores::{AgentSessionStore as _, ThreadStore as _};
                let input: ArchiveInput = parse(input)?;
                let id = bare_id(&input.stream, oxplow_domain::uri::resources::STREAM, "/stream")?;
                // Refused before anything changes: nothing below runs for a
                // stream that can't be archived.
                deps.streams.archivable(&id).await.map_err(session)?;
                // Running as the rail shows it: derived from the logged
                // activity, so an agent that died mid-turn doesn't pin it.
                let threads = deps.threads.list_for_stream(&id).await?;
                let now = Timestamp::now();
                for t in &threads {
                    // Each open session, and what no session claims.
                    let sessions = deps.sessions.list_open_for_thread(&t.id).await?;
                    let slots = sessions.iter().map(|s| Some(s.id)).chain([None]);
                    for slot in slots {
                        let activity = recent_activity(&deps.log, t.id, slot).await?;
                        if derive_session_status(&activity, now)
                            == oxplow_domain::AgentStatusState::Running
                        {
                            return Err(invalid(
                                "/stream",
                                "an agent is still running in one of this stream's threads",
                            ));
                        }
                    }
                }
                // Its threads' work ends: each open effort closes at a
                // snapshot taken now, while the working copy is still there.
                for t in &threads {
                    use oxplow_db::EffortStore as _;
                    let Some(effort) = deps.efforts.find_open_for_thread(&t.id).await? else {
                        continue;
                    };
                    let end = match deps.snapshots.active() {
                        Some(snapshots) => snapshots
                            .mark(&oxplow_domain::snapshot::MarkRequest {
                                thread: Some(t.id),
                                effort: Some(effort.id),
                                ..oxplow_domain::snapshot::MarkRequest::new(
                                    id,
                                    oxplow_domain::snapshot::SnapshotTrigger::EffortEnd,
                                )
                            })
                            .await
                            .map_err(|e| CommandError::Failed {
                                message: format!("the end snapshot of {}: {e}", effort.id),
                            })?
                            .map(|m| m.snapshot)
                            .or(effort.start_snapshot_id),
                        None => None,
                    };
                    deps.efforts
                        .close(effort.id, end, oxplow_db::effort_store::ClosedBy::System)
                        .await?;
                }
                // Its threads' agent sessions close, and their processes stop
                // before the working copy goes.
                for t in &threads {
                    let closed = deps
                        .sessions
                        .close_for_thread(
                            t.id,
                            oxplow_domain::agent_session::SessionCloseReason::StreamArchived,
                        )
                        .await?;
                    for session in closed {
                        deps.processes.kill(session);
                    }
                }
                // Its terminals' shells end with their rows, before the
                // working copy they run in goes.
                for terminal in deps.terminals.for_stream(id).await? {
                    deps.terminal_sessions
                        .close_key(&crate::terminal_sessions::terminal_key(terminal.id))
                        .await;
                }
                deps.db
                    .transaction(move |tx| {
                        for terminal in oxplow_db::terminal_store::for_stream_tx(tx, id)? {
                            oxplow_db::terminal_store::delete_tx(tx, terminal.id)?;
                        }
                        Ok(())
                    })
                    .await?;
                // Before the worktree goes: its deletion is no one's surprise.
                deps.stream_watches.unregister(&id);
                deps.streams
                    .archive_stream(&id, input.delete_worktree)
                    .await
                    .map_err(session)?;
                // Its files leave the search index; nothing routes to it.
                if let Err(e) = deps.search.purge_stream_files(&id.to_string()).await {
                    tracing::warn!(error = %e, stream = %id, "purging an archived stream's search rows failed");
                }
                deps.worktrees.forget(&id).await;
                deps.snapshot_captures.unregister(&id);
                Ok(HandlerOutput {
                    result: json!({ "stream": input.stream, "archived": true }),
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .confirm_at_least(Confirm::Destructive)
}

/// `stream.rename { stream, title }`; an agent renames only its own
/// stream. Undone by renaming it back.
pub fn rename_op() -> Op {
    Op::new(
        "streams.write",
        "rename",
        schema::<RenameInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: RenameInput = parse(input)?;
            let id = bare_id(
                &input.stream,
                oxplow_domain::uri::resources::STREAM,
                "/stream",
            )?;
            if let Some((_, own)) = agent_scope(ctx)? {
                if own != id {
                    return Err(CommandError::Denied {
                        reason: format!(
                            "an agent renames only its own stream (`{}`)",
                            stream_uri(own)
                        ),
                    });
                }
            }
            let mut stream = load(ctx, id)?;
            let before = std::mem::replace(&mut stream.title, input.title);
            stream.updated_at = Timestamp::now();
            upsert_tx(ctx.conn, &stream).map_err(sql)?;
            Ok(HandlerOutput {
                inverse: Some(CommandCall {
                    name: RENAME.into(),
                    input: json!({ "stream": input.stream, "title": before }),
                }),
                ..result(&stream)
            })
        })),
    )
}

/// `stream.set_prompt { stream, prompt? }` — a person's: it steers every
/// agent in the stream. Undone by setting it back.
pub fn set_prompt_op() -> Op {
    Op::new(
        "streams.write",
        "set_prompt",
        schema::<SetPromptInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: SetPromptInput = parse(input)?;
            let mut stream = load(
                ctx,
                bare_id(
                    &input.stream,
                    oxplow_domain::uri::resources::STREAM,
                    "/stream",
                )?,
            )?;
            let before = std::mem::replace(
                &mut stream.custom_prompt,
                input.prompt.filter(|p| !p.trim().is_empty()),
            );
            stream.updated_at = Timestamp::now();
            upsert_tx(ctx.conn, &stream).map_err(sql)?;
            Ok(HandlerOutput {
                inverse: Some(CommandCall {
                    name: SET_PROMPT.into(),
                    input: json!({ "stream": input.stream, "prompt": before }),
                }),
                ..result(&stream)
            })
        })),
    )
}

/// The stream commands, for the bus.
pub fn ops(deps: StreamDeps) -> Vec<Op> {
    vec![
        create_op(deps.clone()),
        archive_op(deps),
        rename_op(),
        set_prompt_op(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{services_with_effort, EffortFixture};
    use oxplow_domain::stores::StreamStore as _;
    use oxplow_domain::Actor;

    async fn primary(fx: &EffortFixture) -> Stream {
        fx.svc.stream_store.primary().await.unwrap().unwrap()
    }

    fn agent(fx: &EffortFixture) -> Actor {
        Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        }
    }

    /// Renaming undoes to the old title; an agent renames and sets the
    /// prompt too.
    #[tokio::test]
    async fn rename_undoes_and_the_prompt_is_set() {
        let fx = services_with_effort().await;
        let s = primary(&fx).await;
        let r = s.id.to_string();
        let out = fx
            .svc
            .commands
            .run(
                &agent(&fx),
                RENAME,
                json!({ "stream": r, "title": "Renamed" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(primary(&fx).await.title, "Renamed");
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(primary(&fx).await.title, s.title);

        fx.svc
            .commands
            .run(
                &agent(&fx),
                SET_PROMPT,
                json!({ "stream": r, "prompt": "x" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(primary(&fx).await.custom_prompt.as_deref(), Some("x"));
        fx.svc
            .commands
            .run(
                &Actor::Human,
                SET_PROMPT,
                json!({ "stream": r, "prompt": "be brief" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            primary(&fx).await.custom_prompt.as_deref(),
            Some("be brief")
        );
        // A prompt that is only whitespace clears it, from any caller.
        fx.svc
            .commands
            .run(
                &Actor::Human,
                SET_PROMPT,
                json!({ "stream": r, "prompt": "  \n\t " }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(primary(&fx).await.custom_prompt, None);
    }

    /// Archiving is destructive: a person is asked first; an agent is not
    /// asked. The primary stream can't be archived by either.
    #[tokio::test]
    async fn archiving_asks_a_person_first() {
        let fx = services_with_effort().await;
        let r = primary(&fx).await.id.to_string();
        let err = fx
            .svc
            .commands
            .run(&Actor::Human, ARCHIVE, json!({ "stream": r }), false)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::NeedsConfirmation { preview } if preview.destructive),
            "{err:?}"
        );
        let err = fx
            .svc
            .commands
            .run(&agent(&fx), ARCHIVE, json!({ "stream": r }), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("primary"), "{err}");
        let err = fx
            .svc
            .commands
            .run(&Actor::Human, ARCHIVE, json!({ "stream": r }), true)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("primary"), "{err}");
        assert!(primary(&fx).await.archived_at.is_none());
        // Refused before anything ran: its thread's effort is still open.
        use oxplow_db::EffortStore as _;
        let open = fx
            .svc
            .effort_store
            .find_open_for_thread(&fx.thread)
            .await
            .unwrap();
        assert_eq!(open.map(|e| e.id), Some(fx.effort), "the effort stays open");
    }

    async fn main_branch(fx: &EffortFixture) -> String {
        let root = fx.svc.layout.project_dir.clone();
        fx.svc.vcs.head(&root).await.unwrap().branch.unwrap()
    }

    /// A stream is created from a branch: a new one forked from a start
    /// point, or one that exists. Its worktree folder is the title's slug,
    /// worked out here and not by the caller.
    #[tokio::test]
    async fn a_stream_is_created_from_a_new_or_an_existing_branch() {
        let fx = services_with_effort().await;
        let main = main_branch(&fx).await;
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                CREATE,
                json!({ "title": "Side Work!", "source": "new", "new_branch": "side-work", "start_point": main }),
                false,
            )
            .await
            .unwrap();
        let made = serde_json::from_value::<Stream>(out.result).unwrap();
        assert_eq!(
            (made.title.as_str(), made.branch.as_str()),
            ("Side Work!", "side-work")
        );
        assert!(
            made.worktree_path.ends_with("-side-work"),
            "{}",
            made.worktree_path
        );

        let root = fx.svc.layout.project_dir.clone();
        let git = std::process::Command::new("git")
            .args(["branch", "feature"])
            .current_dir(&root)
            .status()
            .unwrap();
        assert!(git.success());
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                CREATE,
                json!({ "title": "Feature", "source": "existing", "existing_branch": "feature" }),
                false,
            )
            .await
            .unwrap();
        let made = serde_json::from_value::<Stream>(out.result).unwrap();
        assert_eq!(made.branch, "feature");
        assert_eq!(made.branch_source, "feature");
    }

    /// Each source needs its own fields, named when they are missing.
    #[tokio::test]
    async fn a_source_without_its_fields_is_refused() {
        let fx = services_with_effort().await;
        for (input, field) in [
            (
                json!({ "title": "T", "source": "existing" }),
                "/existing_branch",
            ),
            (
                json!({ "title": "T", "source": "new", "new_branch": "b" }),
                "/start_point",
            ),
            (
                json!({ "title": "T", "source": "new", "start_point": "main" }),
                "/new_branch",
            ),
            (json!({ "title": "T", "source": "worktree" }), "/worktree"),
        ] {
            let err = fx
                .svc
                .commands
                .run(&Actor::Human, CREATE, input, false)
                .await
                .unwrap_err();
            assert!(
                matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == field),
                "{field}: {err:?}"
            );
        }
    }

    /// An agent sets up parallel work on a branch, and may adopt a worktree
    /// on disk as a person may (one that isn't a worktree fails as theirs
    /// would, not as a refusal).
    #[tokio::test]
    async fn an_agent_creates_from_a_branch_and_may_adopt() {
        let fx = services_with_effort().await;
        let main = main_branch(&fx).await;
        fx.svc
            .commands
            .run(
                &agent(&fx),
                CREATE,
                json!({ "title": "Agent work", "source": "new", "new_branch": "agent-work", "start_point": main }),
                false,
            )
            .await
            .unwrap();
        let err = fx
            .svc
            .commands
            .run(
                &agent(&fx),
                CREATE,
                json!({ "title": "Adopted", "source": "worktree", "worktree": "/tmp/somewhere" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(!matches!(err, CommandError::Denied { .. }), "{err:?}");
    }

    /// An agent running in any of a thread's sessions — not only its first
    /// — keeps the stream from being archived.
    #[tokio::test]
    async fn a_running_second_session_keeps_the_stream() {
        let fx = services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        let main = fx.svc.vcs.head(&root).await.unwrap().branch.unwrap();
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                CREATE,
                json!({ "title": "Busy", "source": "new", "new_branch": "busy", "start_point": main }),
                false,
            )
            .await
            .unwrap();
        let side = serde_json::from_value::<oxplow_domain::Stream>(out.result)
            .unwrap()
            .id;
        let thread = crate::test_fixtures::new_thread(&fx.svc, side, "t").await;
        use oxplow_domain::stores::AgentSessionStore as _;
        let second = fx
            .svc
            .agent_session_store
            .open(&oxplow_domain::agent_session::NewAgentSession::terminal(
                thread.id, "claude",
            ))
            .await
            .unwrap()
            .id;
        fx.svc
            .hook_ingest
            .ingest(crate::hook_ingest::HookEnvelope {
                session_title: None,
                turn_run: None,
                kind: oxplow_domain::HookKind::UserPromptSubmit,
                thread_id: Some(thread.id),
                stream_id: Some(side),
                agent_session_id: Some(second),
                session_id: Some("h2".into()),
                payload_json: "{}".into(),
                prompt: Some("go".into()),
                decision: None,
                tool: None,
                subagent: None,
                peer: None,
            })
            .await
            .unwrap();
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                ARCHIVE,
                json!({ "stream": side.to_string() }),
                true,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("still running"), "{err}");
    }

    /// Archiving a stream closes its terminals: their shells end and their
    /// rows go with it.
    #[tokio::test]
    async fn archiving_closes_its_terminals() {
        let fx = services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        std::fs::write(root.join("base.txt"), "b\n").unwrap();
        crate::test_fixtures::commit_all(&root, "base");
        let main = fx.svc.vcs.head(&root).await.unwrap().branch.unwrap();
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                CREATE,
                json!({ "title": "Term", "source": "new", "new_branch": "term", "start_point": main }),
                false,
            )
            .await
            .unwrap();
        let side = serde_json::from_value::<oxplow_domain::Stream>(out.result)
            .unwrap()
            .id;
        let opened = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                crate::commands::terminal::OPEN,
                json!({ "stream": side.to_string() }),
                false,
            )
            .await
            .unwrap();
        let id: oxplow_domain::TerminalId = opened.result["id"].as_str().unwrap().parse().unwrap();
        let key = crate::terminal_sessions::terminal_key(id);
        fx.svc
            .terminal_sessions
            .attach_or_create(key.clone(), 80, 24, |c, r| {
                crate::terminal_sessions::SpawnRequest {
                    command: "sh".into(),
                    args: vec!["-c".into(), "sleep 30".into()],
                    cwd: std::env::temp_dir(),
                    env: Vec::new(),
                    env_remove: Vec::new(),
                    cols: c,
                    rows: r,
                }
            })
            .await
            .unwrap();
        assert!(fx
            .svc
            .terminal_sessions
            .session_id_for_key(&key)
            .await
            .is_some());
        fx.svc
            .commands
            .run(
                &Actor::Human,
                ARCHIVE,
                json!({ "stream": side.to_string(), "delete_worktree": true }),
                true,
            )
            .await
            .unwrap();
        assert!(fx
            .svc
            .terminal_sessions
            .session_id_for_key(&key)
            .await
            .is_none());
        assert!(fx.svc.terminal_store.get(id).await.unwrap().is_none());
    }

    /// Archiving a stream closes its threads' open efforts with an end
    /// snapshot taken before its working copy goes.
    #[tokio::test]
    async fn archiving_closes_its_efforts_before_the_worktree_goes() {
        use oxplow_db::EffortStore as _;
        let fx = services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        std::fs::write(root.join("base.txt"), "b\n").unwrap();
        crate::test_fixtures::commit_all(&root, "base");
        let main = fx.svc.vcs.head(&root).await.unwrap().branch.unwrap();
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                CREATE,
                json!({ "title": "Arch", "source": "new", "new_branch": "arch", "start_point": main }),
                false,
            )
            .await
            .unwrap();
        let side = serde_json::from_value::<oxplow_domain::Stream>(out.result)
            .unwrap()
            .id;
        let thread = crate::test_fixtures::new_thread(&fx.svc, side, "t").await;
        let side_dir = fx
            .svc
            .worktrees
            .resolve(Some(&side.to_string()))
            .await
            .into_local_path();
        let capture = fx.svc.snapshot_captures.get(&side).unwrap();
        capture.enqueue_startup_diff().await.unwrap();
        let start = capture
            .request_snapshot(oxplow_domain::snapshot::SnapshotTrigger::Startup)
            .await
            .unwrap();
        let effort = fx
            .svc
            .effort_store
            .start(
                "oxplow:oxplow/work_item?id=A-1&provider=issues",
                &thread.id,
                start,
            )
            .await
            .unwrap();
        std::fs::write(side_dir.join("work.txt"), "w\n").unwrap();
        capture.enqueue_startup_diff().await.unwrap();
        fx.svc
            .commands
            .run(
                &Actor::Human,
                ARCHIVE,
                json!({ "stream": side.to_string(), "delete_worktree": true }),
                true,
            )
            .await
            .unwrap();
        let closed = fx
            .svc
            .effort_store
            .get_effort(&effort.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(closed.closed_by.as_deref(), Some("system"));
        let end = closed.end_snapshot_id.expect("an end snapshot");
        let changed: Vec<String> = fx
            .svc
            .snapshot_store
            .diff_snapshots(closed.start_snapshot_id, end)
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.path)
            .collect();
        assert_eq!(changed, vec!["work.txt".to_string()]);
    }
}
