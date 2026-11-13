//! `records.read/drift { uri, path }` (`oxplow.refs.drift`): how far a file a
//! model row cites has drifted from the snapshot its ref is pinned to — the
//! unified diff, so the agent reads only what changed. Any model row's
//! `file_pin` edge qualifies, not a knowledge page's alone.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::super::ops::Op;
use super::super::util::{failed, ServicesSlot};
use super::ids::{read_op, result_of};

/// The command its operation is declared as.
pub const REFS_DRIFT: &str = "oxplow.refs.drift";

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DriftParams {
    /// The model row that cites the file, by its URI
    /// (`oxplow:oxplow/knowledge?id=architecture&store=oxwiki`).
    pub uri: String,
    /// Repo-relative path of the file the row cites.
    pub path: String,
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    vec![read_op(
        "records.read",
        "drift",
        services,
        |svc, _, p: DriftParams| async move {
            let drift = crate::pin_drift::compute_pin_drift(
                &svc.page_ref_store,
                &svc.snapshot_store,
                &svc.snapshot_content,
                &svc.layout.project_dir,
                &p.uri,
                &p.path,
            )
            .await
            .map_err(failed)?;
            result_of(drift)
        },
    )]
}

#[cfg(test)]
mod tests {
    use oxplow_domain::Actor;
    use serde_json::json;

    /// A row of a model that is not knowledge, with a file edge and no pin,
    /// reads `no_pin`; a path it doesn't cite reads `not_a_ref`.
    #[tokio::test]
    async fn drift_reads_any_rows_file_edge_through_the_bus() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let svc = crate::Services::in_memory(dir.path()).await.unwrap();
        let row = "oxplow:oxplow/book?id=dune&store=shelves";
        svc.page_ref_store
            .upsert_edge(oxplow_db::PageRefEdge::new(
                row,
                oxplow_domain::uri::build::file_uri("crates/foo.rs"),
                "file_pin",
            ))
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
            let input = json!({ "uri": row, "path": path });
            async move {
                svc.commands
                    .run(&actor, super::REFS_DRIFT, input, false)
                    .await
                    .unwrap()
                    .result["status"]
                    .clone()
            }
        };
        assert_eq!(status("crates/foo.rs").await, "no_pin");
        assert_eq!(status("crates/zzz.rs").await, "not_a_ref");
    }
}
