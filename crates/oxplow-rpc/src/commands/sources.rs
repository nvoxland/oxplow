//! Cores for the `sources` command module — extension-declared data
//! sources: list with state, and run (with the human's consent). Sources
//! are read from the primary stream's worktree: their data is
//! project-global. See `.context/semantic-layer.md`.

use oxplow_app::events::OxplowEvent;
use oxplow_app::source_runner::{self, SourceListing, SourceRunReport};
use oxplow_app::Services;

use crate::error::IpcError;

/// Every declared source with its last run state and whether this
/// machine has approved its current entry script.
pub async fn list_sources(svc: &Services) -> Result<Vec<SourceListing>, IpcError> {
    let root = svc.git.resolve_repo_dir(None).await;
    Ok(source_runner::list_sources(&root, &svc.layout.state_dir, &svc.ext_source_store).await?)
}

/// Run a source now. `approve: true` records the human's consent for the
/// current entry script first (UI only; agents can't approve).
pub async fn run_source(
    svc: &Services,
    extension: String,
    source_id: String,
    approve: Option<bool>,
) -> Result<SourceRunReport, IpcError> {
    let root = svc.git.resolve_repo_dir(None).await;
    let result = source_runner::run_source(
        &root,
        &svc.layout.state_dir,
        &svc.ext_source_store,
        &extension,
        &source_id,
        approve.unwrap_or(false),
    )
    .await;
    // Ran (ok or failed) → data or state changed. A refused run (no
    // consent) or unknown source changed nothing.
    if result.as_ref().map_or_else(|e| e.ran(), |_| true) {
        svc.events.emit(OxplowEvent::SourceSynced {
            extension,
            source_id,
        });
    }
    result.map_err(|e| IpcError::from(oxplow_domain::DomainError::from(e)))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    fn seed(root: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        let ext = root.join("oxplow/extensions/my-gh");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "name: my-gh\nsources:\n  - id: gh\n    runtime: exec\n    entry: sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int, title: text } }\n",
        )
        .unwrap();
        let script = ext.join("sync.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\necho '{\"entities\":{\"pr\":[{\"number\":5,\"title\":\"Five\"}]}}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[tokio::test]
    async fn list_run_and_query_a_source() {
        let (svc, _dir) = crate::test_support::services();
        seed(&svc.layout.project_dir);

        let list = crate::dispatch("list_sources", json!({}), &svc)
            .await
            .unwrap();
        assert_eq!(list[0]["extension"], "my-gh");
        assert_eq!(list[0]["spec"]["id"], "gh");
        assert_eq!(list[0]["approved"], false);
        assert_eq!(list[0]["state"], serde_json::Value::Null);

        let err = crate::dispatch(
            "run_source",
            json!({ "extension": "my-gh", "sourceId": "gh" }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");

        let report = crate::dispatch(
            "run_source",
            json!({ "extension": "my-gh", "sourceId": "gh", "approve": true }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(report["rowCounts"]["pr"], 1);

        let list = crate::dispatch("list_sources", json!({}), &svc)
            .await
            .unwrap();
        assert_eq!(list[0]["approved"], true);
        assert_eq!(list[0]["state"]["status"], "ok");

        let q = crate::dispatch(
            "query_sql",
            json!({ "sql": "SELECT title FROM v_my_gh_pr" }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(q["rows"], json!([["Five"]]));
        let schema = crate::dispatch("describe_schema", json!({}), &svc)
            .await
            .unwrap();
        let pr = schema
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"] == "v_my_gh_pr")
            .unwrap()
            .clone();
        assert_eq!(pr["owner"], "my-gh");
        assert_eq!(pr["available"], true);
    }
}
