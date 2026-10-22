//! Cores for the `collectors` command module — extension-declared
//! collectors: list with their last run, approve (a person's consent) and
//! credentials. Collectors are read from the primary stream's worktree:
//! their data is project-global. Running one is the `oxplow.collector.sync`
//! command. See `.context/semantic-layer.md` → "Collectors".

use oxplow_app::collector_runner::{self, CollectorListing, Collectors};
use oxplow_app::Services;

use crate::error::IpcError;

/// Every declared collector, each extension's, with its last run.
pub async fn list_collectors(svc: &Services) -> Result<Vec<CollectorListing>, IpcError> {
    let root = svc.worktrees.resolve(None).await.into_local_path();
    Ok(collector_runner::list_collectors(&Collectors::of(svc, &root)).await?)
}

/// Set (or clear with `null`) a credential an extension's collector or
/// provider declares.
/// The value goes to the keychain and never comes back. UI only.
pub async fn set_credential(
    svc: &Services,
    extension: String,
    name: String,
    value: Option<String>,
) -> Result<(), IpcError> {
    let root = svc.worktrees.resolve(None).await.into_local_path();
    Ok(collector_runner::set_credential(
        &Collectors::of(svc, &root),
        &extension,
        &name,
        value.as_deref(),
    )?)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    fn seed(root: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        let ext = root.join(".oxplow/extensions/my-gh");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: my-gh\nintent: { purpose: test, origin: null, examples: [] }\npermissions: { process: true }\ncollectors:\n  - id: gh\n    runtime: exec\n    entry: sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int, title: text } }\n",
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
    async fn list_collectors_includes_the_projects_own() {
        let (svc, dir) = crate::test_support::services().await;
        let ext = dir.path().join(".oxplow/extensions/project");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: project\nintent:\n  purpose: test\ncollectors:\n  - { id: tests.cov, records: coverage, entry: \"oxplow:lcov\", report: { path: target/lcov.info } }\n",
        )
        .unwrap();
        crate::test_support::publish(&svc).await;
        let list = crate::dispatch("list_collectors", json!({}), &svc)
            .await
            .unwrap();
        let project: Vec<_> = list
            .as_array()
            .unwrap()
            .iter()
            .filter(|l| l["owner"] == "project")
            .collect();
        assert_eq!(project.len(), 1, "{list}");
        assert_eq!(project[0]["spec"]["id"], "tests.cov");
    }

    #[tokio::test]
    async fn list_run_and_query_a_collector() {
        let (svc, _dir) = crate::test_support::services().await;
        seed(&svc.layout.project_dir);
        crate::test_support::publish(&svc).await;

        let list = crate::dispatch("list_collectors", json!({}), &svc)
            .await
            .unwrap();
        let list = list
            .as_array()
            .unwrap()
            .iter()
            .filter(|l| l["owner"] == "my-gh")
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(list[0]["spec"]["id"], "gh");
        assert_eq!(list[0]["run"], serde_json::Value::Null);

        // Its extension declares `process`, approved when it was deployed:
        // it runs.
        let out = crate::dispatch(
            "run_command",
            json!({
                "id": "oxplow.collector.sync",
                "input": { "collector": "my-gh/gh" },
                "confirmed": false,
            }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(out["result"]["rowCounts"]["pr"], 1);

        let list = crate::dispatch("list_collectors", json!({}), &svc)
            .await
            .unwrap();
        assert_eq!(list[0]["run"]["status"], "ok");
        // The run is logged, in the transaction that wrote its rows.
        let logged = crate::dispatch(
            "query_sql",
            json!({ "sql": "SELECT json_extract(payload, '$.collector'), json_extract(payload, '$.trigger'), json_extract(payload, '$.entities.pr') FROM v_event WHERE type = 'collector.synced'" }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(logged["rows"], json!([["my-gh/gh", "manual", 1]]));

        let q = crate::dispatch(
            "query_sql",
            json!({ "sql": "SELECT title FROM v_my_gh_pr" }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(q["rows"], json!([["Five"]]));
        // The synced entity is a model its extension owns, its columns
        // documented like any other.
        let model = crate::dispatch(
            "query_sql",
            json!({ "sql": "SELECT m.owner, m.kind, count(c.name) FROM v_model m JOIN v_model_column c USING (view) WHERE m.view = 'v_my_gh_pr' GROUP BY m.view" }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(model["rows"][0][0], "my-gh");
        assert_eq!(model["rows"][0][1], "entity");
        assert!(model["rows"][0][2].as_i64().unwrap() > 0);
    }

    #[tokio::test]
    async fn credentials_are_set_through_ipc_and_reported_without_values() {
        let (svc, _dir) = crate::test_support::services().await;
        seed(&svc.layout.project_dir);
        crate::test_support::publish(&svc).await;
        let yaml = svc
            .layout
            .project_dir
            .join(".oxplow/extensions/my-gh/extension.yaml");
        let text = std::fs::read_to_string(&yaml).unwrap();
        std::fs::write(
            &yaml,
            text.replace(
                "entry: sync.sh\n",
                "entry: sync.sh\n    credentials: [GH_PAT]\n",
            )
            .replace(
                "permissions: { process: true }",
                "permissions: { process: true, secrets: [GH_PAT] }",
            ),
        )
        .unwrap();
        crate::test_support::publish(&svc).await;

        crate::dispatch(
            "set_credential",
            json!({ "extension": "my-gh", "name": "GH_PAT", "value": "pat-xyz" }),
            &svc,
        )
        .await
        .unwrap();
        let list = crate::dispatch("list_collectors", json!({}), &svc)
            .await
            .unwrap();
        assert_eq!(
            list[0]["credentials"],
            json!([{ "name": "GH_PAT", "set": true }])
        );
        assert!(!list.to_string().contains("pat-xyz"));

        let err = crate::dispatch(
            "set_credential",
            json!({ "extension": "my-gh", "name": "NOPE", "value": "x" }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
    }
}
