//! The project's own collectors (`collectors:` in `.oxplow/project.yaml`),
//! listed as extensions' are (`collector_runner::list_collectors`): their
//! last run, consent and health, so Settings → Data shows them, syncs them
//! by hand and offers Enable Again. See `.context/collection.md` "Report
//! collectors".

#[cfg(test)]
use std::sync::Arc;

use oxplow_domain::DomainError;

use crate::collector_runner::{contribution_key, CollectorListing};
use crate::exec_consent::ProgramKind;
use crate::Services;

/// The project's collectors as listings, by id: what the config declares
/// (`Services::config`), each with its last run (`collector_run`, owner
/// `project`), why failures disabled it, and whether this machine approved
/// the program behind it. A bundled parser or a derived script that calls
/// no model runs without an approval (`approved`, no `version`); an exec
/// collector's is the Programs row's (`exec_consent::list`), at its
/// `version`.
pub async fn listings(svc: &Services) -> Result<Vec<CollectorListing>, DomainError> {
    let config = svc
        .config
        .read()
        .map(|c| c.clone())
        .unwrap_or_else(|p| p.into_inner().clone());
    if config.collectors.is_empty() {
        return Ok(Vec::new());
    }
    let runs = svc.collector_store.list_runs().await?;
    let health = crate::contribution_health::ContributionHealth::new(
        svc.db.clone(),
        svc.event_log_store.vocabulary().clone(),
    );
    let programs = crate::exec_consent::list(&svc.approvals, &svc.layout.project_dir, &config, &[]);
    let owner = oxplow_config::collectors::PROJECT;
    let mut specs = config.collectors.clone();
    specs.sort_by(|a, b| a.id.cmp(&b.id));
    let mut out = Vec::new();
    for spec in specs {
        let program = programs
            .iter()
            .find(|p| p.kind == ProgramKind::Collector && p.name == spec.id);
        out.push(CollectorListing {
            owner: owner.to_string(),
            disabled: health
                .disabled_reason(&contribution_key(owner, &spec.id))
                .await?,
            run: runs
                .iter()
                .find(|r| r.owner == owner && r.id == spec.id)
                .cloned(),
            approved: program.is_none_or(|p| p.approved),
            network_enforced: crate::net_sandbox::enforced(),
            credentials: Vec::new(),
            version: program.and_then(|p| p.version.clone()),
            spec,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::Actor;
    use serde_json::json;

    async fn services() -> (Arc<Services>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let svc = Arc::new(Services::in_memory(dir.path()).unwrap());
        svc.streams.ensure_primary().await.unwrap();
        svc.metrics.seed_catalog().await;
        (svc, dir)
    }

    fn configure(svc: &Services, dir: &std::path::Path, yaml: &str) {
        std::fs::create_dir_all(dir.join(".oxplow")).unwrap();
        std::fs::write(dir.join(".oxplow/project.yaml"), yaml).unwrap();
        svc.reload_config_from_disk().unwrap();
    }

    const CONFIG: &str = "collectors:\n\
        \x20 - { id: tests.cov, records: coverage, entry: \"oxplow:lcov\", report: { path: target/lcov.info } }\n\
        \x20 - { id: repo.answer, doc: answer, runtime: starlark, entry: count.star, facts: [oxplow.ast_hit] }\n\
        \x20 - { id: repo.fetch, doc: fetch, runtime: exec, entry: tools/fetch.sh, facts: [oxplow.ast_hit] }\n";

    /// The report collectors and the project's other collectors are listed
    /// with owner `project`: nothing run yet, bundled parsers and derived
    /// scripts need no approval, and an exec collector waits for one.
    #[tokio::test]
    async fn the_projects_collectors_are_listed_with_their_consent() {
        let (svc, dir) = services().await;
        std::fs::create_dir_all(dir.path().join("tools")).unwrap();
        std::fs::write(dir.path().join("tools/fetch.sh"), "#!/bin/sh\necho '{}'\n").unwrap();
        configure(&svc, dir.path(), CONFIG);
        let listed = listings(&svc).await.unwrap();
        let ids: Vec<(&str, &str)> = listed
            .iter()
            .map(|l| (l.owner.as_str(), l.spec.id.as_str()))
            .collect();
        assert_eq!(
            ids,
            vec![
                ("project", "repo.answer"),
                ("project", "repo.fetch"),
                ("project", "tests.cov")
            ]
        );
        let by = |id: &str| listed.iter().find(|l| l.spec.id == id).unwrap();
        assert!(by("tests.cov").approved && by("tests.cov").version.is_none());
        assert!(by("repo.answer").approved);
        assert!(
            !by("repo.fetch").approved,
            "an exec collector waits for a person"
        );
        assert!(
            by("repo.fetch").version.is_some(),
            "with the version to approve"
        );
        assert!(listed
            .iter()
            .all(|l| l.run.is_none() && l.disabled.is_none()));
    }

    /// A sync lands its run on the listing; a failing collector disabled by
    /// the failure policy says why.
    #[tokio::test]
    async fn a_project_collectors_run_and_health_show_on_its_listing() {
        let (svc, dir) = services().await;
        std::fs::write(
            dir.path().join("count.star"),
            "def transform(input):\n    return {\"facts\": [{\"measure\": \"oxplow.ast_hit\", \"value\": 1, \"rule\": \"answer\", \"subject\": \"tree:.\"}]}\n",
        )
        .unwrap();
        configure(&svc, dir.path(), CONFIG);
        svc.commands
            .run(
                &Actor::Human,
                crate::collector_runner::SYNC,
                json!({ "owner": "project", "id": "repo.answer" }),
                false,
            )
            .await
            .unwrap();
        let listed = listings(&svc).await.unwrap();
        let ran = listed.iter().find(|l| l.spec.id == "repo.answer").unwrap();
        assert_eq!(ran.run.as_ref().map(|r| r.status.as_str()), Some("ok"));

        let health = crate::contribution_health::ContributionHealth::new(
            svc.db.clone(),
            svc.event_log_store.vocabulary().clone(),
        );
        let key = contribution_key("project", "tests.cov");
        for _ in 0..8 {
            let _ = health.failed(&key, "report not found").await;
        }
        let listed = listings(&svc).await.unwrap();
        let cov = listed.iter().find(|l| l.spec.id == "tests.cov").unwrap();
        assert!(cov.disabled.is_some(), "{:?}", cov.disabled);
    }
}
