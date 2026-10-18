//! The files of a stream's worktree as commands (`files.write`): `save`
//! (what the editor's Save writes), `create`, `create_directory`, `rename`
//! and `delete`. Each is `External` (the worktree), audited like any
//! write, and logs what it did for whatever should react (`file.saved`,
//! `file.created`, `file.directory_created`, `file.renamed`,
//! `file.deleted`: an effect `on: [file.saved]`; the window, which fixes
//! its open buffers on a rename or delete). Save leaves its content out of
//! the record (`with_unrecorded`: its size stands in). A person works in
//! any stream; an agent — whose Save the window does for it, as it
//! (`client_host`) — only in its own stream, and as a write, only from an
//! open thread. Delete asks first (`Destructive`).

use std::sync::Arc;

use oxplow_domain::events::schema::{
    FileCreated, FileCreatedV1, FileDeleted, FileDeletedV1, FileDirectoryCreated,
    FileDirectoryCreatedV1, FileRenamed, FileRenamedV1, FileSaved, FileSavedV1,
};
use oxplow_domain::{CommandError, Confirm, Envelope, StreamId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::ops::Op;
use super::util::{parse, ref_id, schema};
use super::{Handler, HandlerOutput, Invocation};
use crate::workspace_files::{WorkspaceError, WorkspaceFiles};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveInput {
    /// The stream whose worktree it's in (`stream:str2`).
    pub stream: String,
    /// Its path in the worktree.
    pub path: String,
    /// What it holds now.
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInput {
    /// The stream whose worktree it's in (`stream:str2`).
    pub stream: String,
    /// Its path in the worktree; the folders above it are made.
    pub path: String,
    /// What it holds; empty when left out.
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathInput {
    /// The stream whose worktree it's in (`stream:str2`).
    pub stream: String,
    /// Its path in the worktree.
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameInput {
    /// The stream whose worktree it's in (`stream:str2`).
    pub stream: String,
    /// The file or directory as it is.
    pub from: String,
    /// Where it goes.
    pub to: String,
}

/// The stream an input names, if `invocation` may change its files: a
/// person any; anyone else only their own — an agent's tools' rule (an
/// effect has none).
fn stream_of(invocation: &Invocation, named: &str) -> Result<StreamId, CommandError> {
    let stream: StreamId = ref_id(named, "stream", "/stream")?;
    if !invocation.actor().may_confirm() && invocation.actor().stream_id() != Some(stream) {
        return Err(CommandError::Denied {
            reason: format!(
                "{} changes files only in its own stream's worktree, not `{named}`",
                invocation.actor().source(),
            ),
        });
    }
    Ok(stream)
}

fn failed(e: WorkspaceError) -> CommandError {
    match e {
        WorkspaceError::Io(_) => CommandError::Failed {
            message: e.to_string(),
        },
        other => CommandError::Invalid {
            field: Some("/path".into()),
            message: other.to_string(),
        },
    }
}

/// `event`, anchored on `stream` and about `subject`.
fn logged(mut event: Envelope, stream: StreamId, subject: &str) -> Envelope {
    event.anchors.stream_id = Some(stream);
    event.with_subject([oxplow_domain::refs::build::file_ref(subject)])
}

/// `save { stream, path, content }`.
pub fn save_op(files: Arc<WorkspaceFiles>) -> Op {
    Op::new(
        "files.write",
        "save",
        schema::<SaveInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input: Value| {
            let files = files.clone();
            Box::pin(async move {
                let input: SaveInput = parse(input)?;
                let stream = stream_of(&invocation, &input.stream)?;
                let bytes = input.content.len() as u64;
                let saved = files
                    .write(Some(&stream.to_string()), input.path.clone(), input.content)
                    .await
                    .map_err(failed)?;
                let event = logged(
                    Envelope::typed::<FileSaved>(
                        invocation.actor().source(),
                        &FileSavedV1 {
                            stream: stream.to_string(),
                            path: saved.path.clone(),
                            bytes,
                        },
                    ),
                    stream,
                    &saved.path,
                );
                Ok(HandlerOutput {
                    result: json!({ "path": saved.path, "bytes": bytes }),
                    events: vec![event],
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .with_unrecorded(&["content"])
}

/// `create { stream, path, content? }`: a new file.
pub fn create_op(files: Arc<WorkspaceFiles>) -> Op {
    Op::new(
        "files.write",
        "create",
        schema::<CreateInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input: Value| {
            let files = files.clone();
            Box::pin(async move {
                let input: CreateInput = parse(input)?;
                let stream = stream_of(&invocation, &input.stream)?;
                let made = files
                    .create_file(Some(&stream.to_string()), input.path, input.content)
                    .await
                    .map_err(failed)?;
                let event = logged(
                    Envelope::typed::<FileCreated>(
                        invocation.actor().source(),
                        &FileCreatedV1 {
                            stream: stream.to_string(),
                            path: made.path.clone(),
                        },
                    ),
                    stream,
                    &made.path,
                );
                Ok(HandlerOutput {
                    result: json!({ "path": made.path }),
                    events: vec![event],
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .with_unrecorded(&["content"])
}

/// `create_directory { stream, path }`.
pub fn create_directory_op(files: Arc<WorkspaceFiles>) -> Op {
    Op::new(
        "files.write",
        "create_directory",
        schema::<PathInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input: Value| {
            let files = files.clone();
            Box::pin(async move {
                let input: PathInput = parse(input)?;
                let stream = stream_of(&invocation, &input.stream)?;
                let path = files
                    .create_directory(Some(&stream.to_string()), input.path)
                    .await
                    .map_err(failed)?;
                let event = logged(
                    Envelope::typed::<FileDirectoryCreated>(
                        invocation.actor().source(),
                        &FileDirectoryCreatedV1 {
                            stream: stream.to_string(),
                            path: path.clone(),
                        },
                    ),
                    stream,
                    &path,
                );
                Ok(HandlerOutput {
                    result: json!({ "path": path }),
                    events: vec![event],
                    ..HandlerOutput::default()
                })
            })
        })),
    )
}

/// `rename { stream, from, to }`: a file or a directory moves.
pub fn rename_op(files: Arc<WorkspaceFiles>) -> Op {
    Op::new(
        "files.write",
        "rename",
        schema::<RenameInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input: Value| {
            let files = files.clone();
            Box::pin(async move {
                let input: RenameInput = parse(input)?;
                let stream = stream_of(&invocation, &input.stream)?;
                let (from, to) = files
                    .rename(Some(&stream.to_string()), input.from, input.to)
                    .await
                    .map_err(failed)?;
                let event = logged(
                    Envelope::typed::<FileRenamed>(
                        invocation.actor().source(),
                        &FileRenamedV1 {
                            stream: stream.to_string(),
                            from: from.clone(),
                            to: to.clone(),
                        },
                    ),
                    stream,
                    &to,
                );
                Ok(HandlerOutput {
                    result: json!({ "from": from, "to": to }),
                    events: vec![event],
                    ..HandlerOutput::default()
                })
            })
        })),
    )
}

/// `delete { stream, path }`: a file or a directory with what's in it.
pub fn delete_op(files: Arc<WorkspaceFiles>) -> Op {
    Op::new(
        "files.write",
        "delete",
        schema::<PathInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input: Value| {
            let files = files.clone();
            Box::pin(async move {
                let input: PathInput = parse(input)?;
                let stream = stream_of(&invocation, &input.stream)?;
                let path = files
                    .delete(Some(&stream.to_string()), input.path)
                    .await
                    .map_err(failed)?;
                let event = logged(
                    Envelope::typed::<FileDeleted>(
                        invocation.actor().source(),
                        &FileDeletedV1 {
                            stream: stream.to_string(),
                            path: path.clone(),
                        },
                    ),
                    stream,
                    &path,
                );
                Ok(HandlerOutput {
                    result: json!({ "path": path }),
                    events: vec![event],
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .confirm_at_least(Confirm::Destructive)
}

#[cfg(test)]
mod tests {
    use oxplow_domain::Actor;
    use serde_json::json;

    /// Save writes the file and is audited — its content left out of the
    /// record, its size kept — and logs `file.saved` caused by the run.
    #[tokio::test(flavor = "multi_thread")]
    async fn save_writes_is_audited_without_its_content_and_logs_file_saved() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let stream = oxplow_domain::refs::build::stream_ref(
            fx.svc.streams.list_streams().await.unwrap()[0].id,
        );
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.file.save",
                json!({ "stream": stream, "path": "notes.txt", "content": "hello" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result, json!({ "path": "notes.txt", "bytes": 5 }));
        assert_eq!(
            std::fs::read_to_string(fx._dir.path().join("notes.txt")).unwrap(),
            "hello"
        );
        let audit = fx
            .svc
            .commands
            .audit_store()
            .get(out.audit_id.unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            audit.input,
            json!({ "stream": stream, "path": "notes.txt", "content": { "omitted_bytes": 5 } })
        );
        let events = fx.svc.event_log_store.read_after(0, 500).await.unwrap();
        let saved = events
            .iter()
            .find(|e| e.envelope.event_type == "file.saved")
            .expect("file.saved logged");
        assert_eq!(saved.envelope.cause, out.event_id);
        assert_eq!(saved.envelope.payload["path"], json!("notes.txt"));
        assert_eq!(saved.envelope.subject, vec!["file:notes.txt".to_string()]);
    }

    async fn run(
        fx: &crate::test_fixtures::EffortFixture,
        name: &str,
        input: serde_json::Value,
        confirmed: bool,
    ) -> Result<oxplow_domain::CommandOutcome, oxplow_domain::CommandError> {
        fx.svc
            .commands
            .run(&Actor::Human, name, input, confirmed)
            .await
    }

    async fn stream_ref_of(fx: &crate::test_fixtures::EffortFixture) -> String {
        oxplow_domain::refs::build::stream_ref(fx.svc.streams.list_streams().await.unwrap()[0].id)
    }

    async fn logged(
        fx: &crate::test_fixtures::EffortFixture,
        event_type: &str,
    ) -> Vec<serde_json::Value> {
        fx.svc
            .event_log_store
            .read_after(0, 1000)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.envelope.event_type == event_type)
            .map(|e| e.envelope.payload)
            .collect()
    }

    /// Create, create_directory, rename and delete change the worktree,
    /// refuse what the filesystem would clobber, and each logs what it did
    /// (`file.created`, `file.directory_created`, `file.renamed`,
    /// `file.deleted`) — what the window reacts to to fix its open buffers.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_file_commands_change_the_worktree_and_log_what_they_did() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx._dir.path().to_path_buf();
        let stream = stream_ref_of(&fx).await;

        let out = run(
            &fx,
            "oxplow.file.create",
            json!({ "stream": stream, "path": "a/new.txt", "content": "hi" }),
            false,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!({ "path": "a/new.txt" }));
        assert_eq!(
            std::fs::read_to_string(root.join("a/new.txt")).unwrap(),
            "hi"
        );
        let again = run(
            &fx,
            "oxplow.file.create",
            json!({ "stream": stream, "path": "a/new.txt" }),
            false,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(again, oxplow_domain::CommandError::Invalid { .. }),
            "{again:?}"
        );

        let out = run(
            &fx,
            "oxplow.file.create_directory",
            json!({ "stream": stream, "path": "docs/guide" }),
            false,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!({ "path": "docs/guide" }));
        assert!(root.join("docs/guide").is_dir());

        let out = run(
            &fx,
            "oxplow.file.rename",
            json!({ "stream": stream, "from": "a/new.txt", "to": "a/old.txt" }),
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            out.result,
            json!({ "from": "a/new.txt", "to": "a/old.txt" })
        );
        assert!(!root.join("a/new.txt").exists() && root.join("a/old.txt").exists());

        // Delete asks first; confirmed, it goes.
        let ask = run(
            &fx,
            "oxplow.file.delete",
            json!({ "stream": stream, "path": "a/old.txt" }),
            false,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(ask, oxplow_domain::CommandError::NeedsConfirmation { .. }),
            "{ask:?}"
        );
        assert!(root.join("a/old.txt").exists());
        let out = run(
            &fx,
            "oxplow.file.delete",
            json!({ "stream": stream, "path": "a/old.txt" }),
            true,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!({ "path": "a/old.txt" }));
        assert!(!root.join("a/old.txt").exists());

        assert_eq!(
            logged(&fx, "file.created").await,
            vec![json!({ "stream": stream_str(&stream), "path": "a/new.txt" })]
        );
        assert_eq!(logged(&fx, "file.directory_created").await.len(), 1);
        assert_eq!(
            logged(&fx, "file.renamed").await,
            vec![json!({ "stream": stream_str(&stream), "from": "a/new.txt", "to": "a/old.txt" })]
        );
        assert_eq!(
            logged(&fx, "file.deleted").await,
            vec![json!({ "stream": stream_str(&stream), "path": "a/old.txt" })]
        );
    }

    fn stream_str(stream_ref: &str) -> String {
        stream_ref.trim_start_matches("stream:").to_string()
    }

    /// A person works in any stream; an agent only in its own — the rule
    /// of Save.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_changes_files_only_in_its_own_stream() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let stream = stream_ref_of(&fx).await;
        let elsewhere = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: Some(oxplow_domain::StreamId::new(99)),
        };
        for (name, input) in [
            (
                "oxplow.file.create",
                json!({ "stream": stream, "path": "x.txt" }),
            ),
            (
                "oxplow.file.create_directory",
                json!({ "stream": stream, "path": "d" }),
            ),
            (
                "oxplow.file.rename",
                json!({ "stream": stream, "from": "x.txt", "to": "y.txt" }),
            ),
        ] {
            let err = fx
                .svc
                .commands
                .run(&elsewhere, name, input, true)
                .await
                .unwrap_err();
            assert!(
                matches!(&err, oxplow_domain::CommandError::Denied { reason } if reason.contains("only in its own stream")),
                "{name}: {err:?}"
            );
        }
    }

    /// An agent's Save — `oxplow.editor.save`, which the window does,
    /// writing the file with `oxplow.file.save` for the call — runs as the
    /// agent: audited to it, not to the person at the window.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_save_the_window_does_for_an_agent_runs_as_the_agent() {
        use crate::events::OxplowEvent;
        let fx = crate::test_fixtures::services_with_effort().await;
        let stream = fx.svc.streams.list_streams().await.unwrap()[0].id;
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: Some(stream),
        };
        let mut seen = fx.svc.events.subscribe_ui();
        fx.svc
            .client_host
            .register("w1", vec!["editor.write".into()]);
        let window = {
            let svc = fx.svc.clone();
            tokio::spawn(async move {
                loop {
                    let OxplowEvent::ClientCall { id, client, .. } = seen.recv().await.unwrap()
                    else {
                        continue;
                    };
                    // The window writes the shown file for the call — as
                    // whoever made it.
                    let actor = svc.client_host.caller(&client, &id).unwrap();
                    let saved = svc
                        .commands
                        .run(
                            &actor,
                            "oxplow.file.save",
                            json!({ "stream": oxplow_domain::refs::build::stream_ref(stream), "path": "notes.txt", "content": "hi" }),
                            false,
                        )
                        .await
                        .unwrap();
                    svc.client_host
                        .answer(&client, &id, Ok(json!({ "saved": "notes.txt" })));
                    return saved;
                }
            })
        };
        fx.svc
            .commands
            .run(&agent, "oxplow.editor.save", json!({}), false)
            .await
            .unwrap();
        let saved = window.await.unwrap();
        let audit = fx
            .svc
            .commands
            .audit_store()
            .get(saved.audit_id.unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            audit.actor_kind,
            oxplow_domain::events::schema::ActorKind::Agent
        );
        assert_eq!(audit.actor_id, Some(fx.thread.to_string()));
    }

    /// A person saves in any stream; an agent only in its own (its tools'
    /// rule), an effect — with no stream — in none.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_saves_only_in_its_own_stream() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let stream = fx.svc.streams.list_streams().await.unwrap()[0].id;
        let input = json!({ "stream": oxplow_domain::refs::build::stream_ref(stream), "path": "n.txt", "content": "x" });
        let elsewhere = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: Some(oxplow_domain::StreamId::new(99)),
        };
        let effect = Actor::Effect {
            effect: "acme/save".into(),
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        for actor in [elsewhere, effect] {
            let err = fx
                .svc
                .commands
                .run(&actor, "oxplow.file.save", input.clone(), false)
                .await
                .unwrap_err();
            assert!(
                matches!(&err, oxplow_domain::CommandError::Denied { reason }
                    if reason.contains("only in its own stream")),
                "{actor:?}: {err:?}"
            );
        }
        assert!(!fx._dir.path().join("n.txt").exists());
    }
}
