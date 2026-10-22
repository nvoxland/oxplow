//! `records.read/read { ref, stream? }`: what a ref names — its model's
//! row, what sits beside it, or a body no model holds (a file at a
//! revision, an event's output) — read for the actor's thread and stream
//! (`ref_read::read`).

use std::sync::Arc;

use oxplow_domain::Actor;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::super::ops::Op;
use super::super::util::{failed, invalid, parse, schema, ServicesSlot};
use super::super::{Handler, HandlerOutput, Invocation};
use crate::ref_read::{read, ReadError, Reader};

/// The command its operation is declared as.
pub const RECORDS_READ: &str = "oxplow.records.read";

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadInput {
    /// What to read, by its URI: `oxplow:oxplow/file?path=src/a.rs&rev=git:HEAD#lines=10-40`,
    /// `oxplow:oxplow/work_item?id=tsk12&provider=oxplow`, `oxplow:oxplow/commit?sha=<sha>`, `oxplow:oxplow/event?id=<id>#part=output`,
    /// `oxplow:oxplow/lens?lens=<ext>/<slug>`.
    pub uri: String,
    /// The stream whose worktree a `file:` or `lens:` ref is read in
    /// (`str2`); absent, your own.
    #[serde(default)]
    pub stream: Option<String>,
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    vec![Op::new(
        "records.read",
        "read",
        schema::<ReadInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let ReadInput { uri, stream } = parse(input)?;
                let svc = services.get()?;
                let (thread, own_stream) = match invocation.actor() {
                    Actor::Agent {
                        thread_id,
                        stream_id,
                        ..
                    } => (*thread_id, *stream_id),
                    _ => (None, None),
                };
                let at = match stream.as_deref() {
                    Some(stream) => Some(
                        oxplow_domain::StreamId::try_from_str(stream).ok_or_else(|| {
                            invalid(
                                "/stream",
                                format!("`stream` is a stream's id (`str2`), not `{stream}`"),
                            )
                        })?,
                    ),
                    None => None,
                };
                let reader = Reader {
                    thread,
                    stream: own_stream,
                };
                let out = read(&svc, &uri, &reader, at).await.map_err(|e| match e {
                    ReadError::Storage(e) => failed(e),
                    other => invalid("/uri", other.to_string()),
                })?;
                Ok(HandlerOutput {
                    result: serde_json::to_value(out).map_err(failed)?,
                    ..HandlerOutput::default()
                })
            })
        })),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::services_with_effort;
    use oxplow_domain::CommandError;
    use serde_json::json;

    /// A file reads from the calling thread's worktree through the bus,
    /// leaving no record; a bad `stream` is refused at its field.
    #[tokio::test]
    async fn a_ref_reads_through_the_bus_for_the_calling_thread() {
        let fx = services_with_effort().await;
        std::fs::write(fx._dir.path().join("who.txt"), "mine").unwrap();
        let actor = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let out = fx
            .svc
            .commands
            .run(
                &actor,
                RECORDS_READ,
                json!({ "uri": "oxplow:oxplow/file?path=who.txt" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["content"], "mine");
        assert!(out.audit_id.is_none(), "a read leaves no record");

        let err = fx
            .svc
            .commands
            .run(
                &actor,
                RECORDS_READ,
                json!({ "uri": "oxplow:oxplow/file?path=who.txt", "stream": "oxplow:oxplow/stream?id=str2" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/stream"),
            "{err:?}"
        );
    }
}
