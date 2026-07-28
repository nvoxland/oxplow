//! Cores for the `effort` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use std::path::Path;

use oxplow_app::Services;
use oxplow_db::{
    AgentKindTokenUsage, AgentNudge, AgentTokenUsage, EffortAtSnapshot, EffortChangedPaths,
    EffortFile, EffortMetricDelta, EffortObservation, ModelTokenUsage, TaskEffort,
    TaskEffortStore as _, TokenUsageByDay, TokenUsageTotals,
};
use oxplow_domain::{EffortId, TaskId, ThreadId, Timestamp};
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

/// Efforts whose span overlaps `[window_start, window_end]` — the time-range
/// overlay the Metrics Explorer draws as effort bands (tsk233).
pub async fn list_efforts_in_window(
    svc: &Services,
    window_start: Timestamp,
    window_end: Timestamp,
) -> Result<Vec<TaskEffort>, IpcError> {
    Ok(svc
        .effort_store
        .list_in_window(window_start, window_end)
        .await?)
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

/// One effort by id — its snapshot bracket (`start_snapshot_id` /
/// `end_snapshot_id`), task id, and lifecycle stamps. Lets the diff
/// view resolve an `effortDiffRef(effortId)` into the (start, end)
/// snapshot endpoints it diffs, including after a cold history reopen
/// where only the effort id survives. `null` when the id is unknown.
pub async fn get_effort(
    svc: &Services,
    effort_id: EffortId,
) -> Result<Option<TaskEffort>, IpcError> {
    Ok(svc.effort_store.get_effort(&effort_id).await?)
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

/// Every effort whose snapshot window overlaps the half-open range
/// `(range_start, range_end]` — incl. efforts that merely started or
/// ended inside it, contain it, or are still open. Drives the diff
/// view's "other efforts that overlapped this range" roster.
pub async fn list_efforts_overlapping_range(
    svc: &Services,
    range_start: i64,
    range_end: i64,
) -> Result<Vec<TaskEffort>, IpcError> {
    Ok(svc
        .effort_store
        .list_efforts_overlapping_range(range_start, range_end)
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
    // Reads from the metric SUBSTRATE, not the legacy `effort_observation` table
    // (tsk215) — coverage/test/analysis samples in the effort window + their
    // verbatim detail payloads, shaped as the panel's observation rows.
    Ok(svc
        .collection
        .effort_observations_from_metrics(&effort_id.to_string(), kind.as_deref())
        .await)
}

/// Per-metric roll-up over an effort — grouped before→after deltas for the
/// task/effort page's metrics panel. Attributed per family (see metrics.md):
/// per-file gauges by the effort's claimed files, operational by thread,
/// coverage/tests by the effort's own diff.
pub async fn list_effort_metric_deltas(
    svc: &Services,
    effort_id: EffortId,
) -> Result<Vec<EffortMetricDelta>, IpcError> {
    Ok(svc
        .collection
        .effort_metric_deltas(&effort_id.to_string())
        .await)
}

/// Persisted agent nudges (report-less-run / coverage-target) for an effort,
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

/// Summed token totals across every recorded turn (Token Analytics page).
pub async fn token_totals_overall(svc: &Services) -> Result<TokenUsageTotals, IpcError> {
    Ok(svc.token_usage_store.totals_overall().await?)
}

/// Token totals grouped by agent/harness, busiest first.
pub async fn token_usage_by_agent(svc: &Services) -> Result<Vec<AgentKindTokenUsage>, IpcError> {
    Ok(svc.token_usage_store.totals_by_agent_kind().await?)
}

/// Token totals grouped by (agent_kind, model), busiest first.
pub async fn token_usage_by_model(svc: &Services) -> Result<Vec<ModelTokenUsage>, IpcError> {
    Ok(svc.token_usage_store.totals_by_model().await?)
}

/// Token volume bucketed by day over the last `days` days (trend chart).
pub async fn token_usage_by_day(
    svc: &Services,
    days: u32,
) -> Result<Vec<TokenUsageByDay>, IpcError> {
    Ok(svc.token_usage_store.usage_by_day(days).await?)
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
    async fn get_effort_dispatches_and_returns_null_for_missing() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "get_effort",
            serde_json::json!({"effortId": "eff999999"}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_null(), "missing effort → null, got {out}");
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
    async fn list_effort_metric_deltas_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch(
            "list_effort_metric_deltas",
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

    #[tokio::test]
    async fn token_analytics_commands_dispatch() {
        let (svc, _dir) = crate::test_support::services();
        let overall = crate::dispatch("token_totals_overall", serde_json::json!({}), &svc)
            .await
            .unwrap();
        assert_eq!(overall["total_tokens"], 0);
        for name in ["token_usage_by_agent", "token_usage_by_model"] {
            let out = crate::dispatch(name, serde_json::json!({}), &svc)
                .await
                .unwrap();
            assert!(out.is_array());
        }
        let by_day = crate::dispatch("token_usage_by_day", serde_json::json!({"days": 30}), &svc)
            .await
            .unwrap();
        assert!(by_day.is_array());
    }
}
