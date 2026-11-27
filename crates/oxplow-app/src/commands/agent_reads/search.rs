//! `search.read/search { query, stream_id?, resources?, limit? }`: site-wide
//! search over every searchable resource (`v_resource.searchable`) and a
//! stream's files, by name and by text, ranked on one scale
//! (`oxplow_db::search_rank`), best first; each hit names the URI it found
//! and its tier.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::super::ops::Op;
use super::super::util::{failed, parse, schema, ServicesSlot};
use super::super::{Handler, HandlerOutput, Invocation};
use super::check_stream_id;

/// The command its operation is declared as.
pub const SEARCH_FIND: &str = "oxplow.search.find";

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SiteSearchInput {
    /// Free text. A hit whose name is or starts with it ranks first, then
    /// one whose name or title holds each word, then one whose body does
    /// (BM25); each hit's `tier` says which (0 exact … 5 body only).
    pub query: String,
    /// Keep one worktree's files. Only files belong to a stream: tasks,
    /// knowledge, threads and every other hit are found whatever stream
    /// they are in. Omit to search every stream's files.
    #[serde(default)]
    pub stream_id: Option<String>,
    /// Restrict to these resources: any `v_resource` lists as `searchable`
    /// (`oxplow/task`, `oxplow/knowledge`, `oxplow/thread_note`, an
    /// extension's), or `oxplow/file`.
    #[serde(default)]
    pub resources: Option<Vec<String>>,
    /// Max hits to return (default 50).
    #[serde(default)]
    pub limit: Option<u32>,
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    vec![Op::new(
        "search.read",
        "search",
        schema::<SiteSearchInput>(),
        false,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let input: SiteSearchInput = parse(input)?;
                if let Some(stream) = &input.stream_id {
                    check_stream_id("/stream_id", stream)?;
                }
                let svc = services.get()?;
                let hits = svc
                    .search_store
                    .search(
                        &input.query,
                        input.stream_id.as_deref(),
                        &input.resources.unwrap_or_default(),
                        input.limit.unwrap_or(50) as usize,
                    )
                    .await
                    .map_err(failed)?;
                Ok(HandlerOutput {
                    result: serde_json::to_value(hits).map_err(failed)?,
                    ..HandlerOutput::default()
                })
            })
        })),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::services_with_thread;
    use oxplow_domain::{Actor, CommandError};
    use serde_json::json;

    /// A indexed task is found by a prefix of its title, through the bus
    /// and leaving no record; a stream id that isn't one is refused at
    /// its field.
    #[tokio::test]
    async fn search_finds_a_task_through_the_bus() {
        let fx = services_with_thread().await;
        fx.svc
            .search_store
            .upsert("task", "tsk1", None, "Reconcile quokka ledger", "")
            .await
            .unwrap();
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
                SEARCH_FIND,
                json!({ "query": "quokk", "resources": ["task"] }),
                false,
            )
            .await
            .unwrap();
        assert!(out.result.to_string().contains("quokka"), "{}", out.result);
        assert!(out.command_run_id.is_none(), "a read leaves no record");

        let err = fx
            .svc
            .commands
            .run(
                &actor,
                SEARCH_FIND,
                json!({ "query": "x", "stream_id": "thr1" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), message }
                if f == "/stream_id" && message.contains("looks like a thread")),
            "{err:?}"
        );
    }
}
