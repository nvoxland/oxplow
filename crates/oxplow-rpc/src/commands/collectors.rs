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
        .map_err(|e| IpcError::from(oxplow_domain::DomainError::from(e)))?;
    svc.events
        .emit(oxplow_app::events::OxplowEvent::ApprovalsChanged);
    Ok(())
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

/// The entry of one of the project's programs (`kind`, `name`), for a
/// person to read before approving it — a bundled extension's from its
/// embedded files (tsk953). UI only.
pub async fn program_source(
    svc: &Services,
    kind: oxplow_app::exec_consent::ProgramKind,
    name: String,
) -> Result<String, IpcError> {
    let programs = list_project_programs(svc).await?;
    let program = programs
        .iter()
        .find(|p| p.kind == kind && p.name == name)
        .ok_or_else(|| IpcError::invalid(format!("no program `{name}`")))?;
    program
        .source(&svc.layout.project_dir)
        .map_err(|e| IpcError::invalid(format!("couldn't read `{name}`: {e}")))
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
    // A running provider restarts on what was just approved; an effect
    // starts after the log's head (it never reacts to what came before).
    match kind {
        oxplow_app::exec_consent::ProgramKind::Provider => svc.providers.approved(&name).await,
        oxplow_app::exec_consent::ProgramKind::Effect => {
            oxplow_app::effects::approved(&svc.db, &name).await?;
        }
        _ => {}
    }
    svc.events
        .emit(oxplow_app::events::OxplowEvent::ApprovalsChanged);
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

    /// tsk953: a person reads a program's entry before approving it — an
    /// effect's script, wherever its extension lives.
    #[tokio::test]
    async fn a_programs_source_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let ext = svc.layout.project_dir.join("oxplow/extensions/acme");
        std::fs::create_dir_all(ext.join("effects")).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: acme\nsharing: private\nintent: { purpose: Effects., origin: null, examples: [] }\neffects:\n  - id: look\n    summary: Looks.\n    on: [work_item.created]\n    entry: effects/look.star\n",
        )
        .unwrap();
        std::fs::write(
            ext.join("effects/look.star"),
            "def transform(x):\n    return {}\n",
        )
        .unwrap();
        let source = crate::dispatch(
            "program_source",
            serde_json::json!({ "kind": "effect", "name": "acme/look" }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(source, "def transform(x):\n    return {}\n");
        let err = crate::dispatch(
            "program_source",
            serde_json::json!({ "kind": "effect", "name": "acme/nope" }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
    }

    #[tokio::test]
    async fn list_and_approve_project_programs_dispatch() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch("list_project_programs", serde_json::json!({}), &svc)
            .await
            .unwrap();
        // Only what comes with oxplow: oxplow-review's follow-up effect,
        // waiting for a person's approval (tsk956).
        let programs = out.as_array().unwrap();
        assert_eq!(programs.len(), 1, "{out}");
        assert_eq!(programs[0]["name"], "oxplow-review/verify-unchecked");
        assert_eq!(programs[0]["approved"], false);
        let err = crate::dispatch(
            "approve_project_program",
            serde_json::json!({ "kind": "collector", "name": "repo.nope", "version": "x" }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
    }

    /// tsk1040: approving a program says so, so what shows its approval
    /// (Settings → Integrations) refreshes instead of staying stale.
    #[tokio::test]
    async fn approving_a_program_announces_it() {
        let (svc, _dir) = crate::test_support::services();
        let mut events = svc.events.subscribe_ui();
        let listed = crate::dispatch("list_project_programs", serde_json::json!({}), &svc)
            .await
            .unwrap();
        let version = listed[0]["version"].as_str().unwrap().to_string();
        crate::dispatch(
            "approve_project_program",
            serde_json::json!({ "kind": "effect", "name": "oxplow-review/verify-unchecked", "version": version }),
            &svc,
        )
        .await
        .unwrap();
        let mut heard = false;
        while let Ok(e) = events.try_recv() {
            heard |= matches!(e, oxplow_app::events::OxplowEvent::ApprovalsChanged);
        }
        assert!(heard, "approving announced nothing");
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
