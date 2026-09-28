//! Cores for the `sources` command module — extension-declared data
//! sources: list with state, and run (with the human's consent). Sources
//! are read from the primary stream's worktree: their data is
//! project-global. See `.context/semantic-layer.md`.

use oxplow_app::source_runner::{self, SourceListing, SourceRunReport, Sources};
use oxplow_app::Services;

use crate::error::IpcError;

/// Every declared source with its last run state and whether this
/// machine has approved its current entry script.
pub async fn list_sources(svc: &Services) -> Result<Vec<SourceListing>, IpcError> {
    let root = svc.git.resolve_repo_dir(None).await;
    Ok(source_runner::list_sources(&Sources::of(svc, &root)).await?)
}

/// Run a source now. `approve` is the listing's `version` the person
/// reviewed: consent is recorded for exactly that version first, and a
/// source that changed since is refused (UI only; agents can't approve).
pub async fn run_source(
    svc: &Services,
    extension: String,
    source_id: String,
    approve: Option<String>,
) -> Result<SourceRunReport, IpcError> {
    source_runner::sync_source(svc, &extension, &source_id, approve.as_deref())
        .await
        .map_err(|e| IpcError::from(oxplow_domain::DomainError::from(e)))
}

/// Set (or clear with `null`) a credential an extension's source declares.
/// The value goes to the keychain and never comes back. UI only.
pub async fn set_source_credential(
    svc: &Services,
    extension: String,
    name: String,
    value: Option<String>,
) -> Result<(), IpcError> {
    let root = svc.git.resolve_repo_dir(None).await;
    Ok(source_runner::set_source_credential(
        &Sources::of(svc, &root),
        &extension,
        &name,
        value.as_deref(),
    )?)
}

/// The programs the project's config would run (`exec` gauges and
/// collection plugins) and whether this machine approved each. UI only.
pub async fn list_project_programs(
    svc: &Services,
) -> Result<Vec<oxplow_app::exec_consent::ProjectProgram>, IpcError> {
    let config = svc
        .config
        .read()
        .map(|c| c.clone())
        .unwrap_or_else(|p| p.into_inner().clone());
    Ok(oxplow_app::exec_consent::list(
        &svc.approvals,
        &svc.layout.project_dir,
        &config,
        &shared_extensions(svc).await,
    ))
}

/// The primary worktree's extensions, whose advisories are approved here.
async fn shared_extensions(svc: &Services) -> Vec<oxplow_app::extensions::Extension> {
    let root = svc.git.resolve_repo_dir(None).await;
    oxplow_app::extensions::load_extensions(&root)
}

/// Approve one of the project's programs as it is now. UI only: consent to
/// run a program from the repo is a person's (tsk331).
pub async fn approve_project_program(
    svc: &Services,
    kind: oxplow_app::exec_consent::ProgramKind,
    name: String,
    version: String,
) -> Result<Vec<oxplow_app::exec_consent::ProjectProgram>, IpcError> {
    let config = svc
        .config
        .read()
        .map(|c| c.clone())
        .unwrap_or_else(|p| p.into_inner().clone());
    let extensions = shared_extensions(svc).await;
    oxplow_app::exec_consent::approve_program(
        &svc.approvals,
        &svc.layout.project_dir,
        &config,
        &extensions,
        kind,
        &name,
        &version,
    )
    .map_err(IpcError::invalid)?;
    Ok(oxplow_app::exec_consent::list(
        &svc.approvals,
        &svc.layout.project_dir,
        &config,
        &shared_extensions(svc).await,
    ))
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
    async fn list_and_approve_project_programs_dispatch() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch("list_project_programs", serde_json::json!({}), &svc)
            .await
            .unwrap();
        assert_eq!(out, serde_json::json!([]), "no exec programs configured");
        let err = crate::dispatch(
            "approve_project_program",
            serde_json::json!({ "kind": "gauge", "name": "repo.nope", "version": "x" }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
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
            json!({ "extension": "my-gh", "sourceId": "gh", "approve": list[0]["version"] }),
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

    #[tokio::test]
    async fn credentials_are_set_through_ipc_and_reported_without_values() {
        let (svc, _dir) = crate::test_support::services();
        seed(&svc.layout.project_dir);
        let yaml = svc
            .layout
            .project_dir
            .join("oxplow/extensions/my-gh/extension.yaml");
        let text = std::fs::read_to_string(&yaml).unwrap();
        std::fs::write(
            &yaml,
            text.replace(
                "entry: sync.sh\n",
                "entry: sync.sh\n    credentials: [GH_PAT]\n",
            ),
        )
        .unwrap();

        crate::dispatch(
            "set_source_credential",
            json!({ "extension": "my-gh", "name": "GH_PAT", "value": "pat-xyz" }),
            &svc,
        )
        .await
        .unwrap();
        let list = crate::dispatch("list_sources", json!({}), &svc)
            .await
            .unwrap();
        assert_eq!(
            list[0]["credentials"],
            json!([{ "name": "GH_PAT", "set": true }])
        );
        assert!(!list.to_string().contains("pat-xyz"));

        let err = crate::dispatch(
            "set_source_credential",
            json!({ "extension": "my-gh", "name": "NOPE", "value": "x" }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
    }
}
