//! Wiki page freshness reader.
//!
//! `list_wiki_freshness(slug)` returns one row per file/directory
//! ref the wiki page carries, joining the captured snapshot pin on
//! `page_ref` with the latest `file_snapshot` for that path so the
//! UI can render a per-ref staleness flag. `mark_wiki_ref_verified`
//! and `mark_all_wiki_refs_verified` re-stamp the pin to the
//! current resolved version when the user explicitly confirms the
//! page is still accurate.

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_app::file_ref_version;
use oxplow_db::page_ref_projections::{KIND_FILE, KIND_WIKI, RT_WIKI_FILE};

use crate::error::IpcError;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct WikiRefFreshness {
    pub path: String,
    /// The snapshot the ref was captured against. 0 when the
    /// wiki sync had no snapshot service available.
    pub local_snapshot_id: i64,
    /// Closest known git commit at capture time; populated only
    /// when the worktree had a HEAD.
    pub closest_git_version: Option<String>,
    /// `true` when the local snapshot is byte-equal to the recorded
    /// commit (capture was on a clean worktree, or
    /// `set_snapshot_git_commit` later attached HEAD to the snapshot).
    pub git_version_exact: bool,
    /// The latest `snapshot.id` whose `file_snapshot.path` matches
    /// this target. `None` when the file hasn't been captured (e.g.
    /// it's outside the workspace or has never been touched since
    /// the snapshot service booted).
    pub latest_snapshot_id: Option<i64>,
    /// `true` when `latest_snapshot_id > local_snapshot_id`. The
    /// renderer paints a "stale" chip on these rows.
    pub stale: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn list_wiki_freshness(
    state: tauri::State<'_, AppState>,
    slug: String,
) -> Result<Vec<WikiRefFreshness>, IpcError> {
    let raw = state.page_ref_store.list_wiki_file_freshness(&slug).await?;
    Ok(raw
        .into_iter()
        .map(|(path, local, git, exact, latest)| WikiRefFreshness {
            path,
            local_snapshot_id: local.unwrap_or(0),
            closest_git_version: git,
            git_version_exact: exact,
            latest_snapshot_id: latest,
            stale: matches!((latest, local), (Some(l), Some(loc)) if l > loc)
                || matches!((latest, local), (Some(_), None)),
        })
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn mark_wiki_ref_verified(
    state: tauri::State<'_, AppState>,
    slug: String,
    path: String,
) -> Result<(), IpcError> {
    let resolved = resolve_current(&state).await;
    let (snap, git, exact) = match resolved {
        Some(v) => (
            v.local_snapshot_id,
            v.closest_git_version,
            v.git_version_exact,
        ),
        None => (0, None, false),
    };
    state
        .page_ref_store
        .restamp_edge_version(
            KIND_WIKI,
            &slug,
            KIND_FILE,
            &path,
            RT_WIKI_FILE,
            snap,
            git,
            exact,
        )
        .await?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn mark_all_wiki_refs_verified(
    state: tauri::State<'_, AppState>,
    slug: String,
) -> Result<usize, IpcError> {
    let resolved = resolve_current(&state).await;
    let (snap, git, exact) = match resolved {
        Some(v) => (
            v.local_snapshot_id,
            v.closest_git_version,
            v.git_version_exact,
        ),
        None => (0, None, false),
    };
    let edges = state
        .page_ref_store
        .list_outbound(KIND_WIKI, &slug, None)
        .await?;
    let mut count = 0;
    for e in edges {
        if e.target_kind != KIND_FILE {
            continue;
        }
        state
            .page_ref_store
            .restamp_edge_version(
                &e.source_kind,
                &e.source_id,
                &e.target_kind,
                &e.target_id,
                &e.ref_type,
                snap,
                git.clone(),
                exact,
            )
            .await?;
        count += 1;
    }
    Ok(count)
}

async fn resolve_current(state: &AppState) -> Option<file_ref_version::ResolvedFileVersion> {
    // Wiki pages live under `<project>/.oxplow/wiki/` and are
    // project-wide (shared across streams), so we deliberately tag
    // their freshness against the primary stream's snapshot service.
    // A proper fix would be a dedicated wiki pseudo-stream — see
    // follow-up filed under epic #28.
    let svc = state.snapshot_captures.primary()?;
    let stream_id = *svc.stream_id();
    let snapshot_id = svc
        .store()
        .latest_snapshot_id_for_stream(stream_id)
        .await
        .ok()
        .flatten()?;
    file_ref_version::resolve(svc.store(), svc.project_dir(), snapshot_id)
        .await
        .ok()
}
