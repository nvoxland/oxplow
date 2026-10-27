//! `collectors.preview/preview`: dry-run a deployed collector against a
//! stream's files and data and return what it would store, storing nothing
//! (`oxplow.collector.preview`).

use oxplow_domain::StreamId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::super::ops::Op;
use super::super::util::{failed, invalid, ServicesSlot};
use super::ids::{check_optional_stream, read_op, result_of};
use super::stream_or_actors;
use crate::collector_runner::{self, RunCollectorError};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PreviewCollectorParams {
    /// The extension that declares it (its folder under `.oxplow/extensions/`).
    pub owner: String,
    /// The collector's `id` in its `collectors:`.
    pub id: String,
    /// Stream whose files and data it reads; omit for your own. What runs
    /// is the deployed collector: a draft is tested with
    /// `$OXPLOW_BIN extension test <name>`.
    pub stream_id: Option<String>,
    /// The event an `on:` collector reacts to, as a fixture writes it:
    /// `{ type, v?, payload?, subject?, anchors? }`. The script gets it as
    /// the event envelope, carried to the type's newest version.
    pub event: Option<serde_json::Value>,
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    vec![read_op(
        "collectors.preview",
        "preview",
        services,
        |svc, actor, p: PreviewCollectorParams| async move {
            check_optional_stream(
                "oxplow_preview_collector",
                "/stream_id",
                p.stream_id.as_deref(),
            )?;
            // Omitted: the caller's own stream.
            let stream = stream_or_actors(&svc, &actor, p.stream_id.clone()).await?;
            let root = svc
                .worktrees
                .resolve(stream.as_deref())
                .await
                .into_local_path();
            let collectors = collector_runner::Collectors::of(&svc, &root);
            // A fact collector (`facts:`) previews through the fact
            // engine: the facts it would record, over that stream's latest
            // snapshot.
            if let Ok((_, spec)) = collector_runner::find_collector(&collectors, &p.owner, &p.id) {
                if !spec.facts.is_empty() {
                    let stream_id = stream
                        .as_deref()
                        .map(|s| {
                            StreamId::try_from_str(s).ok_or_else(|| {
                                invalid("/stream_id", format!("invalid stream id `{s}`"))
                            })
                        })
                        .transpose()?
                        .map(|s| s.value());
                    let facts = svc
                        .metrics
                        .preview_fact_collector_in(
                            &p.owner,
                            &spec,
                            &root,
                            stream_id,
                            p.event.as_ref(),
                        )
                        .await
                        .map_err(|m| invalid("/id", m))?;
                    return result_of(json!({ "owner": p.owner, "id": p.id, "facts": facts }));
                }
            }
            let preview = collector_runner::preview_collector(
                &collectors,
                &p.owner,
                &p.id,
                p.event.as_ref(),
                None,
            )
            .await
            .map_err(|e| match e {
                RunCollectorError::NotFound => invalid(
                    "/id",
                    format!(
                        "no deployed collector `{}/{}` (see `v_extension`; a draft is tested with `$OXPLOW_BIN extension test <name>`)",
                        p.owner, p.id
                    ),
                ),
                RunCollectorError::Denied(m)
                | RunCollectorError::Failed(m)
                | RunCollectorError::Disabled(m) => invalid("/id", m),
                RunCollectorError::Storage(e) => failed(e),
            })?;
            result_of(preview)
        },
    )]
}

#[cfg(test)]
mod tests {
    use oxplow_domain::{Actor, CommandError};
    use serde_json::json;

    /// An agent previews a source from its stream's worktree: rows back,
    /// nothing stored; an unknown collector says where to look.
    #[tokio::test]
    async fn a_preview_returns_rows_without_storing() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let ext = dir.path().join(".oxplow/extensions/work");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: work\nintent:\n  purpose: test\ncollectors:\n  - id: one\n    runtime: jaq\n    entry: one.jq\n    input: \"SELECT 1 AS n\"\n    entities:\n      - { name: nums, key: n, columns: { n: int } }\n",
        )
        .unwrap();
        std::fs::write(ext.join("one.jq"), "{entities: {nums: .rows}}").unwrap();
        let svc = crate::Services::in_memory(dir.path()).await.unwrap();
        crate::test_fixtures::activate_with_problems(&svc).await;
        let actor = Actor::Agent {
            session_id: None,
            thread_id: None,
            stream_id: None,
        };
        let out = svc
            .commands
            .run(
                &actor,
                "oxplow.collector.preview",
                json!({ "owner": "work", "id": "one" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["entities"][0]["rows"], json!([[1]]));
        assert!(svc.collector_store.list_runs().await.unwrap().is_empty());
        let err = svc
            .commands
            .run(
                &actor,
                "oxplow.collector.preview",
                json!({ "owner": "work", "id": "nope" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { message, .. } if message.contains("v_extension")),
            "{err:?}"
        );
    }
}
