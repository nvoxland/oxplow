//! Stream commands (P8.A4, `.context/commands.md`): a stream's lifecycle
//! — a new worktree, adopting one already on disk, archiving — and its
//! title and custom prompt. Creating, adopting and archiving reach the
//! VCS and the stream's capture service, so they are `External` and a
//! person's; archiving is destructive (it can remove the working copy).
//! Renaming and the prompt are `Tx` over the stream row. A stream is
//! named by ref (`stream:str1`).

use std::sync::Arc;

use oxplow_db::stream_store::{get_tx, upsert_tx};
use oxplow_domain::refs::build::stream_ref;
use oxplow_domain::{
    Atomicity, CommandCall, CommandEffect, CommandError, CommandSpec, Confirm, Invokers, Lifecycle,
    Stream, StreamId, Timestamp,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::thread::agent_scope;
use super::{Command, Handler, HandlerOutput, Invocation, TxCtx};

pub const CREATE_WORKTREE: &str = "stream.create_worktree";
pub const ADOPT_WORKTREE: &str = "stream.adopt_worktree";
pub const ARCHIVE: &str = "stream.archive";
pub const RENAME: &str = "stream.rename";
pub const SET_PROMPT: &str = "stream.set_prompt";

/// A person, or a lens acting for one.
const PEOPLE: Invokers = Invokers {
    human: true,
    agent: false,
    lens: true,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateWorktreeInput {
    /// The worktree's folder name (beside the project, fixed for good).
    pub slug: String,
    pub title: String,
    /// The branch to create for it.
    pub branch: String,
    /// The branch it forks from.
    pub branch_source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdoptWorktreeInput {
    /// An existing worktree of this repository.
    pub path: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArchiveInput {
    /// The stream (`stream:str2`).
    pub stream: String,
    /// Also remove its working copy from disk.
    #[serde(default)]
    pub delete_worktree: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameInput {
    /// The stream (`stream:str1`).
    pub stream: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetPromptInput {
    /// The stream (`stream:str1`).
    pub stream: String,
    /// Appended to every agent prompt in this stream; empty or absent
    /// clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

/// What the `External` stream commands reach: the stream service (the VCS
/// and the rows), the capture registry, and what an archive cleans up.
#[derive(Clone)]
pub struct StreamDeps {
    pub streams: oxplow_session::StreamService,
    pub snapshot_captures: crate::snapshot_capture_registry::SnapshotCaptureRegistry,
    pub ref_moves: crate::ref_moves::RefMoves,
    pub threads: Arc<oxplow_db::SqliteThreadStore>,
    pub log: Arc<oxplow_db::SqliteEventLogStore>,
    pub search: Arc<oxplow_db::SqliteSearchStore>,
    pub worktrees: Arc<crate::worktrees::WorktreeRouter>,
    pub efforts: Arc<oxplow_db::SqliteEffortStore>,
}

fn invalid(field: &str, message: String) -> CommandError {
    CommandError::Invalid {
        field: Some(field.into()),
        message,
    }
}

fn parse<T: serde::de::DeserializeOwned>(input: serde_json::Value) -> Result<T, CommandError> {
    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
        field: None,
        message: e.to_string(),
    })
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

fn session(e: oxplow_session::SessionError) -> CommandError {
    use oxplow_session::SessionError as E;
    match e {
        E::DuplicateWorktreeSlug(_) => invalid("/slug", e.to_string()),
        E::Storage(oxplow_domain::DomainError::NotFound) => {
            invalid("/stream", "no such stream".into())
        }
        other => CommandError::Failed {
            message: other.to_string(),
        },
    }
}

fn sql(e: rusqlite::Error) -> CommandError {
    CommandError::from(oxplow_db::map_sql_err(e))
}

fn load(ctx: &TxCtx<'_>, id: StreamId) -> Result<Stream, CommandError> {
    get_tx(ctx.conn, id)
        .map_err(sql)?
        .filter(|s| s.archived_at.is_none())
        .ok_or_else(|| invalid("/stream", format!("no stream `{}`", stream_ref(id))))
}

fn result(stream: &Stream) -> HandlerOutput {
    HandlerOutput {
        result: serde_json::to_value(stream).expect("a stream serializes"),
        ..HandlerOutput::default()
    }
}

fn schema<T: JsonSchema>() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes")
}

fn person_external(
    name: &str,
    summary: &str,
    schema: serde_json::Value,
    confirm: Confirm,
) -> CommandSpec {
    CommandSpec {
        name: name.into(),
        summary: summary.into(),
        input_schema: schema,
        invokers: Invokers::HUMAN_ONLY,
        confirm,
        undoable: false,
        lifecycle: Lifecycle::Stable,
        atomicity: Atomicity::External,
        effect: CommandEffect::Write,
    }
}

fn row_spec(
    name: &str,
    summary: &str,
    schema: serde_json::Value,
    invokers: Invokers,
) -> CommandSpec {
    CommandSpec {
        name: name.into(),
        summary: summary.into(),
        input_schema: schema,
        invokers,
        confirm: Confirm::Never,
        undoable: true,
        lifecycle: Lifecycle::Stable,
        atomicity: Atomicity::Tx,
        effect: CommandEffect::Record,
    }
}

/// A new stream's capture service starts, so its edits land in snapshots.
fn start_capture(deps: &StreamDeps, stream: &Stream) {
    if let Some(capture) = deps.snapshot_captures.register(stream) {
        capture.spawn_watcher();
        capture.spawn_git_refs_listener(&deps.ref_moves);
    }
}

/// `stream.create_worktree { slug, title, branch, branch_source }`.
pub fn create_worktree_command(deps: StreamDeps) -> Command {
    Command::new(
        person_external(
            CREATE_WORKTREE,
            "Create a stream on a new worktree beside the project (`git worktree add`), on a \
             new branch forked from `branch_source`. A person's.",
            schema::<CreateWorktreeInput>(),
            Confirm::Never,
        ),
        Handler::External(Arc::new(move |_: Invocation, input| {
            let deps = deps.clone();
            Box::pin(async move {
                let input: CreateWorktreeInput = parse(input)?;
                let stream = deps
                    .streams
                    .create_worktree(&input.slug, input.title, input.branch, input.branch_source)
                    .await
                    .map_err(session)?;
                start_capture(&deps, &stream);
                Ok(result(&stream))
            })
        })),
    )
    .expect("stream.create_worktree is a valid command")
}

/// `stream.adopt_worktree { path, title }`.
pub fn adopt_worktree_command(deps: StreamDeps) -> Command {
    Command::new(
        person_external(
            ADOPT_WORKTREE,
            "Make an existing worktree of this repository a stream. A person's.",
            schema::<AdoptWorktreeInput>(),
            Confirm::Never,
        ),
        Handler::External(Arc::new(move |_: Invocation, input| {
            let deps = deps.clone();
            Box::pin(async move {
                let input: AdoptWorktreeInput = parse(input)?;
                let stream = deps
                    .streams
                    .adopt_worktree(std::path::PathBuf::from(&input.path), input.title)
                    .await
                    .map_err(session)?;
                start_capture(&deps, &stream);
                Ok(result(&stream))
            })
        })),
    )
    .expect("stream.adopt_worktree is a valid command")
}

/// `stream.archive { stream, delete_worktree? }`: refused while an agent
/// runs in one of its threads; its threads go with it — their open efforts
/// close at a snapshot taken first — and its working copy when asked.
/// Destructive: a person confirms it.
pub fn archive_command(deps: StreamDeps) -> Command {
    Command::new(
        person_external(
            ARCHIVE,
            "Archive a stream and its threads (`delete_worktree` also removes its working copy \
             from disk). Refused while an agent is running in it. A person's, confirmed.",
            schema::<ArchiveInput>(),
            Confirm::Destructive,
        ),
        Handler::External(Arc::new(move |_: Invocation, input| {
            let deps = deps.clone();
            Box::pin(async move {
                use crate::agent_status_derive::{derive_thread_status, recent_activity};
                use oxplow_domain::stores::ThreadStore as _;
                let input: ArchiveInput = parse(input)?;
                let id = stream_of(&input.stream)?;
                // Running as the rail shows it: derived from the logged
                // activity, so an agent that died mid-turn doesn't pin it.
                let threads = deps.threads.list_for_stream(&id).await?;
                let now = Timestamp::now();
                for t in &threads {
                    let activity = recent_activity(&deps.log, t.id).await?;
                    if derive_thread_status(&activity, now)
                        == oxplow_domain::AgentStatusState::Running
                    {
                        return Err(invalid(
                            "/stream",
                            "an agent is still running in one of this stream's threads".into(),
                        ));
                    }
                }
                // Its threads' work ends: each open effort closes at a
                // snapshot taken now, while the working copy is still there.
                for t in &threads {
                    use oxplow_db::EffortStore as _;
                    let Some(effort) = deps.efforts.find_open_for_thread(&t.id).await? else {
                        continue;
                    };
                    let end = match deps.snapshot_captures.get(&id) {
                        Some(capture) => capture
                            .request_snapshot(crate::snapshot_capture::TakeRequest {
                                trigger: oxplow_domain::snapshot::SnapshotTrigger::EffortEnd,
                                thread_id: Some(t.id),
                                turn_id: None,
                                effort_id: Some(effort.id),
                                budget: None,
                            })
                            .await
                            .map_err(|e| CommandError::Failed {
                                message: format!("the end snapshot of {}: {e}", effort.id),
                            })?
                            .or(effort.start_snapshot_id),
                        None => None,
                    };
                    deps.efforts
                        .close(effort.id, end, oxplow_db::effort_store::ClosedBy::System)
                        .await?;
                }
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
    .expect("stream.archive is a valid command")
}

/// `stream.rename { stream, title }`; an agent renames only its own
/// stream. Undone by renaming it back.
pub fn rename_command() -> Command {
    Command::new(
        row_spec(
            RENAME,
            "Rename a stream (`stream:str1`).",
            schema::<RenameInput>(),
            Invokers::ALL,
        ),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: RenameInput = parse(input)?;
            let id = stream_of(&input.stream)?;
            if let Some((_, own)) = agent_scope(ctx)? {
                if own != id {
                    return Err(CommandError::Denied {
                        reason: format!(
                            "an agent renames only its own stream (`{}`)",
                            stream_ref(own)
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
    .expect("stream.rename is a valid command")
}

/// `stream.set_prompt { stream, prompt? }` — a person's: it steers every
/// agent in the stream. Undone by setting it back.
pub fn set_prompt_command() -> Command {
    Command::new(
        row_spec(
            SET_PROMPT,
            "Set (or clear) the text appended to every agent prompt in a stream. A person's: it \
             steers the agents.",
            schema::<SetPromptInput>(),
            PEOPLE,
        ),
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: SetPromptInput = parse(input)?;
            let mut stream = load(ctx, stream_of(&input.stream)?)?;
            let before = std::mem::replace(
                &mut stream.custom_prompt,
                input.prompt.filter(|p| !p.is_empty()),
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
    .expect("stream.set_prompt is a valid command")
}

/// The stream commands, for the bus.
pub fn commands(deps: StreamDeps) -> Vec<Command> {
    vec![
        create_worktree_command(deps.clone()),
        adopt_worktree_command(deps.clone()),
        archive_command(deps),
        rename_command(),
        set_prompt_command(),
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
            thread_id: Some(fx.thread),
            stream_id: None,
        }
    }

    /// Renaming undoes to the old title; an agent renames only its own
    /// stream; the prompt is a person's.
    #[tokio::test]
    async fn rename_undoes_and_the_prompt_is_a_persons() {
        let fx = services_with_effort().await;
        let s = primary(&fx).await;
        let r = stream_ref(s.id);
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

        let err = fx
            .svc
            .commands
            .run(
                &agent(&fx),
                RENAME,
                json!({ "stream": "stream:str99", "title": "x" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        let err = fx
            .svc
            .commands
            .run(
                &agent(&fx),
                SET_PROMPT,
                json!({ "stream": r, "prompt": "x" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
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
    }

    /// Archiving is destructive: a person is asked first, and an agent
    /// can't archive at all. The primary stream can't be archived.
    #[tokio::test]
    async fn archiving_asks_first_and_is_a_persons() {
        let fx = services_with_effort().await;
        let r = stream_ref(primary(&fx).await.id);
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
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        let err = fx
            .svc
            .commands
            .run(&Actor::Human, ARCHIVE, json!({ "stream": r }), true)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("primary"), "{err}");
        assert!(primary(&fx).await.archived_at.is_none());
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
                CREATE_WORKTREE,
                json!({ "slug": "arch", "title": "Arch", "branch": "arch", "branch_source": main }),
                false,
            )
            .await
            .unwrap();
        let side = serde_json::from_value::<oxplow_domain::Stream>(out.result)
            .unwrap()
            .id;
        let thread = crate::test_fixtures::new_thread(&fx.svc, side, "t").await;
        let side_dir = fx.svc.worktrees.resolve(Some(&side.to_string())).await;
        let capture = fx.svc.snapshot_captures.get(&side).unwrap();
        capture.enqueue_startup_diff().await.unwrap();
        let start = capture
            .request_snapshot(oxplow_domain::snapshot::SnapshotTrigger::Startup)
            .await
            .unwrap();
        let effort = fx
            .svc
            .effort_store
            .start("work_item:issues:A-1", &thread.id, start)
            .await
            .unwrap();
        std::fs::write(side_dir.join("work.txt"), "w\n").unwrap();
        capture.enqueue_startup_diff().await.unwrap();
        fx.svc
            .commands
            .run(
                &Actor::Human,
                ARCHIVE,
                json!({ "stream": stream_ref(side), "delete_worktree": true }),
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
