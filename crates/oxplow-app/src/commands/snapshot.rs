//! `oxplow.snapshot.restore_file` (P8.A9): write a captured file's bytes back to
//! its path in its stream's worktree (`snapshot_files`). `External` (the
//! worktree) and destructive — it overwrites what's there now — so a
//! person confirms it and an agent's run becomes a proposal.

use crate::commands::ops::Op;
use oxplow_domain::Confirm;
use std::sync::Arc;

use oxplow_domain::CommandError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::util::{parse, schema};
use super::{Handler, HandlerOutput, Invocation};
use crate::snapshot_files::{SnapshotFileError, SnapshotFiles};

pub const RESTORE_FILE: &str = "oxplow.snapshot.restore_file";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RestoreInput {
    /// The captured file row (a `file_snapshot` id).
    pub file_snapshot: i64,
}

/// `snapshot.restore_file { file_snapshot }`.
pub fn restore_file_op(files: SnapshotFiles) -> Op {
    Op::new(
        "files.write",
        "restore_file",
        schema::<RestoreInput>(),
        false,
        Handler::External(Arc::new(move |_: Invocation, input: Value| {
            let files = files.clone();
            Box::pin(async move {
                let input: RestoreInput = parse(input)?;
                let restored = files
                    .restore_file_snapshot(input.file_snapshot)
                    .await
                    .map_err(|e| match e {
                        SnapshotFileError::NotFound
                        | SnapshotFileError::NoContent
                        | SnapshotFileError::Expired => CommandError::Invalid {
                            field: Some("/file_snapshot".into()),
                            message: e.to_string(),
                        },
                        SnapshotFileError::Other(message) => CommandError::Failed { message },
                    })?;
                Ok(HandlerOutput {
                    result: json!({ "restored": restored }),
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .confirm_at_least(Confirm::Destructive)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::services_with_effort;
    use oxplow_domain::Actor;

    /// A restore overwrites the worktree: unconfirmed, a person is asked
    /// and an agent's run is a proposal.
    #[tokio::test]
    async fn a_restore_asks_first() {
        let fx = services_with_effort().await;
        let input = json!({ "file_snapshot": 1 });
        let err = fx
            .svc
            .commands
            .run(&Actor::Human, RESTORE_FILE, input.clone(), false)
            .await
            .unwrap_err();
        assert!(
            matches!(err, CommandError::NeedsConfirmation { .. }),
            "{err:?}"
        );
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Agent {
                    thread_id: Some(fx.thread),
                    stream_id: None,
                },
                RESTORE_FILE,
                input,
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Proposed { .. }), "{err:?}");
    }
}
