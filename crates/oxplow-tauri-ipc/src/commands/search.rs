use oxplow_db::SearchHit;

use crate::error::IpcError;
use crate::state::AppState;

/// Site-wide BM25 search across tasks, comments, notes, wiki pages, and
/// per-stream file contents. `stream_id` scopes file/stream-bound hits to one
/// worktree (project-global hits like wiki always included); `None` searches
/// everything. `kinds` optionally restricts to a subset
/// (`task|comment|note|wiki|file`). Results are ranked best-first.
#[tauri::command]
#[specta::specta]
pub async fn search(
    state: tauri::State<'_, AppState>,
    query: String,
    stream_id: Option<String>,
    kinds: Option<Vec<String>>,
    limit: Option<u32>,
) -> Result<Vec<SearchHit>, IpcError> {
    oxplow_rpc::commands::search::search(&state, query, stream_id, kinds, limit).await
}
