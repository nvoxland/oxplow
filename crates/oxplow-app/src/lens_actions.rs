//! Running a lens's declared actions (tsk329). The registry is fixed
//! (`copy`, `add-to-context`, `run-source`), so an extension can offer a
//! button but never run code through one. The UI and MCP both come here.
//! A `run-source` action never approves an exec source: consent is given in
//! Settings → Data, where what runs and which hosts it reaches are shown.
//! See `.context/extensions.md`.

use std::collections::BTreeMap;
use std::path::Path;

use oxplow_db::{SemanticLayer, SqlCell};
use oxplow_domain::DomainError;
use serde::{Deserialize, Serialize};

use crate::extensions::{self, LensActionKind};
use crate::source_runner::{RunSourceError, SourceRunReport};

/// What an action produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LensActionResult {
    /// `copy`: the text to copy.
    pub text: Option<String>,
    /// `run-source`: the sync's row counts.
    pub report: Option<SourceRunReport>,
}

/// Run action `action_id` of lens `lens_id` (resolved in `lens_root`, the
/// stream's worktree) with `params`.
pub async fn run_lens_action(
    svc: &crate::Services,
    lens_root: &Path,
    lens_id: &str,
    action_id: &str,
    params: BTreeMap<String, SqlCell>,
) -> Result<LensActionResult, DomainError> {
    let lens = extensions::find_lens(lens_root, lens_id)?;
    let action = lens
        .actions
        .iter()
        .find(|a| a.id == action_id)
        .cloned()
        .ok_or_else(|| {
            let ids: Vec<&str> = lens.actions.iter().map(|a| a.id.as_str()).collect();
            DomainError::Invalid(format!(
                "lens `{lens_id}` has no action `{action_id}` (it has {ids:?})"
            ))
        })?;
    match action.kind {
        LensActionKind::Copy => {
            let layer = SemanticLayer::new(svc.db.clone());
            let run = extensions::run_lens(&layer, lens_root, lens_id, params).await?;
            Ok(LensActionResult {
                text: Some(extensions::lens_text(&run)),
                report: None,
            })
        }
        LensActionKind::AddToContext => Err(DomainError::Invalid(
            "`add-to-context` is how a person hands this lens to the agent; read it with run_lens"
                .into(),
        )),
        LensActionKind::RunSource => {
            let source = action.source.unwrap_or_default();
            let (ext, id) = source.split_once('/').unwrap_or_default();
            match crate::source_runner::sync_source(svc, ext, id, None).await {
                Ok(report) => Ok(LensActionResult {
                    text: None,
                    report: Some(report),
                }),
                Err(RunSourceError::NotFound) => Err(DomainError::Invalid(format!(
                    "lens `{lens_id}` runs source `{source}`, which doesn't exist"
                ))),
                Err(RunSourceError::NeedsApproval(_)) => Err(DomainError::Invalid(format!(
                    "source `{source}` needs a person's approval first: Settings → Data → Approve & Run"
                ))),
                Err(e) => Err(e.into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    async fn fixture() -> (Arc<crate::Services>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        let svc = Arc::new(crate::Services::in_memory(dir.path()).unwrap());
        svc.streams.ensure_primary().await.unwrap();
        let ext = dir.path().join("oxplow/extensions/acme");
        std::fs::create_dir_all(ext.join("lenses")).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "name: acme\nsources:\n  - id: nums\n    runtime: jaq\n    entry: nums.jq\n    entities:\n      - { name: num, key: n, columns: { n: int } }\n  - id: prog\n    runtime: exec\n    entry: prog.sh\n    entities:\n      - { name: row, key: id, columns: { id: int } }\n",
        )
        .unwrap();
        std::fs::write(ext.join("nums.jq"), "{entities: {num: [{n: 1}, {n: 2}]}}").unwrap();
        std::fs::write(ext.join("prog.sh"), "#!/bin/sh\necho '{\"entities\":{}}'\n").unwrap();
        std::fs::write(
            ext.join("lenses/nums.yaml"),
            "title: Nums\nquery: \"SELECT 'a|b' AS label, :k AS k, 99 AS hidden\"\nparams: [{ name: k, default: 7 }]\ncolumns: [{ key: k }, { key: label, label: Label }]\nactions:\n  - copy\n  - add-to-context\n  - { action: run-source, source: acme/nums }\n  - { action: run-source, id: prog, source: acme/prog }\n  - { action: run-source, id: gone, source: acme/nope }\n",
        )
        .unwrap();
        (svc, dir)
    }

    #[tokio::test]
    async fn copy_returns_the_result_as_markdown() {
        let (svc, dir) = fixture().await;
        let params = BTreeMap::from([("k".to_string(), SqlCell::Int(3))]);
        let out = run_lens_action(&svc, dir.path(), "acme/nums", "copy", params)
            .await
            .unwrap();
        assert_eq!(
            out.text.as_deref(),
            Some("| k | Label |\n| --- | --- |\n| 3 | a\\|b |\n"),
            "the columns the lens shows, in its order; a helper column stays out"
        );
    }

    #[tokio::test]
    async fn run_source_syncs_but_never_approves() {
        let (svc, dir) = fixture().await;
        let mut events = svc.events.subscribe();
        let out = run_lens_action(&svc, dir.path(), "acme/nums", "run-source", BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(out.report.unwrap().row_counts).unwrap(),
            json!({"num": 2})
        );
        assert!(matches!(
            events.recv().await,
            Ok(crate::OxplowEvent::SourceSynced { .. })
        ));
        // An exec source nobody approved: refused, nothing ran.
        let err = run_lens_action(&svc, dir.path(), "acme/nums", "prog", BTreeMap::new())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("approval"), "{err}");
        let err = run_lens_action(&svc, dir.path(), "acme/nums", "gone", BTreeMap::new())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("doesn't exist"), "{err}");
    }

    #[tokio::test]
    async fn add_to_context_and_unknown_actions_are_refused() {
        let (svc, dir) = fixture().await;
        let err = run_lens_action(
            &svc,
            dir.path(),
            "acme/nums",
            "add-to-context",
            BTreeMap::new(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("run_lens"), "{err}");
        let err = run_lens_action(&svc, dir.path(), "acme/nums", "shell", BTreeMap::new())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no action `shell`"), "{err}");
    }
}
