//! `oxplow.file.save` (`files.write` `save`): what the editor's Save
//! writes — a file of a stream's worktree, with the content the person
//! saved. `External` (the worktree), audited like any write, its content
//! left out of the record (`with_unrecorded`: its size stands in), and it
//! logs `file.saved` for whatever should happen on save (an effect
//! `on: [file.saved]`).

use std::sync::Arc;

use oxplow_domain::events::schema::{FileSaved, FileSavedV1};
use oxplow_domain::{CommandError, Envelope, Invokers};
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
    .open_to(Invokers::HUMAN_ONLY)
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
}
