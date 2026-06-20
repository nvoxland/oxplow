//! Cores for the `effort` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use std::path::Path;

use oxplow_app::Services;
use oxplow_db::{
    AgentNudge, AgentTokenUsage, EffortAtSnapshot, EffortChangedPaths, EffortFile,
    EffortObservation, TaskEffort, TaskEffortStore as _, TokenUsageTotals,
};
use oxplow_domain::{EffortId, TaskId, ThreadId};
use oxplow_fs_watch::WorkspaceFilter;

use crate::error::IpcError;

fn current_filter(svc: &Services) -> WorkspaceFilter {
    let cfg = svc.config.read();
    cfg.as_ref()
        .map(|c| {
            WorkspaceFilter::for_project(
                &svc.layout.project_dir,
                &c.generated.exclude,
                &c.generated.include,
            )
        })
        .unwrap_or_default()
}

pub async fn list_task_efforts(
    svc: &Services,
    item_id: TaskId,
) -> Result<Vec<TaskEffort>, IpcError> {
    Ok(svc.effort_store.list_for_item(item_id).await?)
}

pub async fn get_effort_files(
    svc: &Services,
    effort_id: EffortId,
) -> Result<Vec<EffortFile>, IpcError> {
    let filter = current_filter(svc);
    let rows = svc.effort_store.list_files(&effort_id).await?;
    Ok(rows
        .into_iter()
        .filter(|f| !filter.ignore(Path::new(&f.path), false))
        .collect())
}

pub async fn list_efforts_at_snapshots(
    svc: &Services,
    snapshot_ids: Vec<i64>,
) -> Result<Vec<EffortAtSnapshot>, IpcError> {
    Ok(svc
        .effort_store
        .list_efforts_at_snapshots(snapshot_ids)
        .await?)
}

/// All distinct file paths whose `file_snapshot` rows fall inside
/// this effort's snapshot bracket — the "all changes during this
/// effort" reference list. Returns empty when the effort has no
/// start/end snapshot pin yet. Drives the reference view shown
/// alongside the canonical `task_effort_file` list on
/// `SnapshotDetailPage`.
pub async fn list_changed_paths_for_effort(
    svc: &Services,
    effort_id: EffortId,
) -> Result<EffortChangedPaths, IpcError> {
    let filter = current_filter(svc);
    let split = svc
        .effort_store
        .list_changed_paths_for_effort(&effort_id)
        .await?;
    let keep = |paths: Vec<String>| -> Vec<String> {
        paths
            .into_iter()
            .filter(|p| !filter.ignore(Path::new(p), false))
            .collect()
    };
    Ok(EffortChangedPaths {
        claimed: keep(split.claimed),
        unclaimed: keep(split.unclaimed),
    })
}

/// Collection observations (test-run / diff-coverage) for an effort,
/// newest-first. Optional `kind` filter. Drives the effort-review
/// coverage badge + tests-run list on `TaskPage`.
pub async fn list_effort_observations(
    svc: &Services,
    effort_id: EffortId,
    kind: Option<String>,
) -> Result<Vec<EffortObservation>, IpcError> {
    Ok(svc
        .observation_store
        .list_for_effort(&effort_id.to_string(), kind.as_deref())
        .await?)
}

/// Persisted agent nudges (report-less-run / commit-hygiene) for an effort,
/// newest-first. Drives the collapsed "Agent nudges" debug sub-view on the
/// task page. See `.context/agent-model.md` (Nudge persistence).
pub async fn list_nudges_for_effort(
    svc: &Services,
    effort_id: EffortId,
) -> Result<Vec<AgentNudge>, IpcError> {
    Ok(svc
        .nudge_store
        .list_for_effort(&effort_id.to_string())
        .await?)
}

/// Per-turn agent token-usage rows for an effort, newest-first (tsk104).
/// Drives the per-effort token panel on the task page.
pub async fn list_token_usage_for_effort(
    svc: &Services,
    effort_id: EffortId,
) -> Result<Vec<AgentTokenUsage>, IpcError> {
    Ok(svc
        .token_usage_store
        .list_for_effort(&effort_id.to_string())
        .await?)
}

/// Summed token totals for one effort.
pub async fn get_effort_token_totals(
    svc: &Services,
    effort_id: EffortId,
) -> Result<TokenUsageTotals, IpcError> {
    Ok(svc
        .token_usage_store
        .totals_for_effort(&effort_id.to_string())
        .await?)
}

/// Summed token totals for a whole thread (the Work panel running total).
pub async fn get_thread_token_totals(
    svc: &Services,
    thread_id: ThreadId,
) -> Result<TokenUsageTotals, IpcError> {
    Ok(svc
        .token_usage_store
        .totals_for_thread(&thread_id.to_string())
        .await?)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_task_efforts_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "list_task_efforts",
            serde_json::json!({"itemId": "tsk999999"}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array());
    }

    #[tokio::test]
    async fn list_effort_observations_dispatches_with_optional_kind() {
        let (svc, _dir) = crate::test_support::services();
        // `kind` omitted → None.
        let out = crate::dispatch(
            "list_effort_observations",
            serde_json::json!({"effortId": "eff999999"}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array());
    }

    #[tokio::test]
    async fn list_nudges_for_effort_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "list_nudges_for_effort",
            serde_json::json!({"effortId": "eff999999"}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array());
    }

    #[tokio::test]
    async fn list_token_usage_for_effort_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "list_token_usage_for_effort",
            serde_json::json!({"effortId": "eff999999"}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array());
    }

    #[tokio::test]
    async fn token_totals_dispatch_to_zeroed_totals() {
        let (svc, _dir) = crate::test_support::services();
        let eff = crate::dispatch(
            "get_effort_token_totals",
            serde_json::json!({"effortId": "eff999999"}),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(eff["total_tokens"], 0);
        let thread = crate::dispatch(
            "get_thread_token_totals",
            serde_json::json!({"threadId": "thr999999"}),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(thread["turns"], 0);
    }
}
