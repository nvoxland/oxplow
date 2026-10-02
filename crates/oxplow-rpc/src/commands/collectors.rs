//! Cores for the `collectors` command module — extension-declared
//! collectors: list with their last run, approve (a person's consent) and
//! credentials. Collectors are read from the primary stream's worktree:
//! their data is project-global. Running one is the `collector.sync`
//! command. See `.context/semantic-layer.md` → "Collectors".

use oxplow_app::collector_runner::{self, CollectorListing, Collectors};
use oxplow_app::Services;

use crate::error::IpcError;

/// Every declared collector with its last run and whether this machine
/// has approved its current entry script.
pub async fn list_collectors(svc: &Services) -> Result<Vec<CollectorListing>, IpcError> {
    let root = svc.worktrees.resolve(None).await;
    Ok(collector_runner::list_collectors(&Collectors::of(svc, &root)).await?)
}

/// A person approves an exec collector at the listing's `version` they
/// reviewed; one that changed since is refused (UI only; agents can't
/// approve). Running it is the `collector.sync` command.
pub async fn approve_collector(
    svc: &Services,
    owner: String,
    id: String,
    version: String,
) -> Result<(), IpcError> {
    let root = svc.worktrees.resolve(None).await;
    collector_runner::approve_reviewed(&Collectors::of(svc, &root), &owner, &id, &version)
        .map_err(|e| IpcError::from(oxplow_domain::DomainError::from(e)))
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
    let root = svc.worktrees.resolve(None).await;
    Ok(collector_runner::set_credential(
        &Collectors::of(svc, &root),
        &extension,
        &name,
        value.as_deref(),
    )?)
}

/// The programs the project's config would run (`exec` collectors and
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
    let root = svc.worktrees.resolve(None).await;
    svc.extension_catalog.get(&root).to_vec()
}

/// What approving provider `instance` as it is on disk would change
/// against what was approved last (P6b.E3) — what Settings → Data shows
/// before its Approve.
pub async fn provider_declaration_effects(
    svc: &Services,
    instance: String,
) -> Result<oxplow_app::extension_effects::ProviderEffect, IpcError> {
    Ok(svc.providers.declaration_effects(&instance).await?)
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
    // A running provider restarts on what was just approved.
    if kind == oxplow_app::exec_consent::ProgramKind::Provider {
        svc.providers.approved(&name).await;
    }
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
            "manifest: 2\nname: my-gh\nsharing: private\nintent: { purpose: test, origin: null, examples: [] }\ncollectors:\n  - id: gh\n    runtime: exec\n    entry: sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int, title: text } }\n",
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
            serde_json::json!({ "kind": "collector", "name": "repo.nope", "version": "x" }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
    }

    #[tokio::test]
    async fn list_run_and_query_a_collector() {
        let (svc, _dir) = crate::test_support::services();
        seed(&svc.layout.project_dir);

        let list = crate::dispatch("list_collectors", json!({}), &svc)
            .await
            .unwrap();
        assert_eq!(list[0]["owner"], "my-gh");
        assert_eq!(list[0]["spec"]["id"], "gh");
        assert_eq!(list[0]["approved"], false);
        assert_eq!(list[0]["run"], serde_json::Value::Null);

        // `collector.sync` never approves: unapproved, it's refused.
        let sync = || {
            crate::dispatch(
                "run_command",
                json!({
                    "name": "collector.sync",
                    "input": { "owner": "my-gh", "id": "gh" },
                    "confirmed": false,
                }),
                &svc,
            )
        };
        let err = sync().await.unwrap_err();
        assert_eq!(err.code, "INVALID");
        // A person approves the version they reviewed, then it runs.
        crate::dispatch(
            "approve_collector",
            json!({ "owner": "my-gh", "id": "gh", "version": list[0]["version"] }),
            &svc,
        )
        .await
        .unwrap();
        let out = sync().await.unwrap();
        assert_eq!(out["result"]["rowCounts"]["pr"], 1);

        let list = crate::dispatch("list_collectors", json!({}), &svc)
            .await
            .unwrap();
        assert_eq!(list[0]["approved"], true);
        assert_eq!(list[0]["run"]["status"], "ok");
        // The run is logged, in the transaction that wrote its rows.
        let logged = crate::dispatch(
            "query_sql",
            json!({ "sql": "SELECT json_extract(payload, '$.collector'), json_extract(payload, '$.trigger'), json_extract(payload, '$.entities.pr') FROM v_event WHERE type = 'collector.synced'" }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(logged["rows"], json!([["collector:my-gh/gh", "manual", 1]]));

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
