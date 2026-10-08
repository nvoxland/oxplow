//! `oxplow.file.save` (`files.write` `save`): what the editor's Save
//! writes — a file of a stream's worktree, with the content the person
//! saved. `External` (the worktree), audited like any write, its content
//! left out of the record (`with_unrecorded`: its size stands in), and it
//! logs `file.saved` for whatever should happen on save (an effect
//! `on: [file.saved]`). A person saves in any stream; an agent — whose
//! Save the window does for it, as it (`client_host`) — only in its own
//! stream, and as a write, only from the stream's writer thread.

use std::sync::Arc;

use oxplow_domain::events::schema::{FileSaved, FileSavedV1};
use oxplow_domain::{CommandError, Envelope};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::ops::Op;
use super::{Handler, HandlerOutput, Invocation};
use crate::workspace_files::{WorkspaceError, WorkspaceFiles};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveInput {
    /// The stream whose worktree it's in (`str2`).
    pub stream: String,
    /// Its path in the worktree.
    pub path: String,
    /// What it holds now.
    pub content: String,
}

/// `save { stream, path, content }`.
pub fn save_op(files: Arc<WorkspaceFiles>) -> Op {
    Op::new(
        "files.write",
        "save",
        serde_json::to_value(schemars::schema_for!(SaveInput)).expect("schema serializes"),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input: Value| {
            let files = files.clone();
            Box::pin(async move {
                let input: SaveInput =
                    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                        field: None,
                        message: e.to_string(),
                    })?;
                // A person saves in any stream; anyone else only in its
                // own — an agent's tools' rule (an effect has none).
                if !invocation.actor.may_confirm()
                    && invocation.actor.stream_id().map(|s| s.to_string())
                        != Some(input.stream.clone())
                {
                    return Err(CommandError::Denied {
                        reason: format!(
                            "{} saves only in its own stream's worktree, not `{}`",
                            invocation.actor.source(),
                            input.stream
                        ),
                    });
                }
                let bytes = input.content.len() as u64;
                let saved = files
                    .write(Some(&input.stream), input.path.clone(), input.content)
                    .await
                    .map_err(|e| match e {
                        WorkspaceError::Io(_) => CommandError::Failed {
                            message: e.to_string(),
                        },
                        other => CommandError::Invalid {
                            field: Some("/path".into()),
                            message: other.to_string(),
                        },
                    })?;
                let mut saved_event = Envelope::typed::<FileSaved>(
                    invocation.actor.source(),
                    &FileSavedV1 {
                        stream: input.stream.clone(),
                        path: saved.path.clone(),
                        bytes,
                    },
                )
                .with_subject([format!("file:{}", saved.path)]);
                saved_event.anchors.stream_id = input.stream.parse().ok();
                Ok(HandlerOutput {
                    result: json!({ "path": saved.path, "bytes": bytes }),
                    events: vec![saved_event],
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .with_unrecorded(&["content"])
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
        let stream = fx.svc.streams.list_streams().await.unwrap()[0]
            .id
            .to_string();
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

    /// An agent's Save — `oxplow.editor.save`, which the window does,
    /// writing the file with `oxplow.file.save` for the call — runs as the
    /// agent: audited to it, not to the person at the window.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_save_the_window_does_for_an_agent_runs_as_the_agent() {
        use crate::events::OxplowEvent;
        let fx = crate::test_fixtures::services_with_effort().await;
        let stream = fx.svc.streams.list_streams().await.unwrap()[0].id;
        let agent = Actor::Agent {
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
                            json!({ "stream": stream.to_string(), "path": "notes.txt", "content": "hi" }),
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
        let input = json!({ "stream": stream.to_string(), "path": "n.txt", "content": "x" });
        let elsewhere = Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: Some(oxplow_domain::StreamId::new(99)),
        };
        let effect = Actor::Effect {
            effect: "acme/save".into(),
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
