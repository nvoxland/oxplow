//! Drift detail for a wiki page's file ref (#112).
//!
//! [`list_stale_wiki_pages`] / `get_wiki_page_metadata.stale_refs`
//! tell the agent *which* refs drifted; this answers *what* drifted —
//! the unified diff between the snapshot the ref was pinned to and the
//! file's current on-disk content, so the agent reads only the changed
//! hunks instead of re-opening the whole file.
//!
//! "Current" is the working-tree file, not the latest snapshot: the
//! agent is updating prose against reality, and disk doesn't lag the
//! snapshot-capture debounce.

use std::path::Path;

use oxplow_db::page_ref_projections::{KIND_FILE, KIND_WIKI};
use oxplow_db::{SqlitePageRefStore, SqliteSnapshotStore};
use oxplow_domain::DomainError;
use serde::Serialize;

use crate::blob_store::BlobStore;

/// Cap on the returned diff. A wiki ref that drifted enormously isn't
/// worth dumping in full into the agent's context — it should re-read
/// the file at that point.
const MAX_DIFF_BYTES: usize = 16_000;

/// Outcome of a drift query for one `(wiki page, file ref)` pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WikiRefDrift {
    pub slug: String,
    pub path: String,
    /// Snapshot the `wiki → file` edge was pinned to. `None` when the
    /// ref carries no pin yet.
    pub pinned_snapshot_id: Option<i64>,
    /// `drifted` (diff present) | `unchanged` | `not_a_ref` (the page
    /// doesn't reference this file) | `no_pin` (ref never pinned) |
    /// `binary` (content isn't UTF-8 text, so no line diff).
    pub status: String,
    /// Unified diff (pinned → current). `Some` only when `drifted`.
    pub unified_diff: Option<String>,
    /// `true` when `unified_diff` was capped at [`MAX_DIFF_BYTES`].
    pub truncated: bool,
}

impl WikiRefDrift {
    fn bare(slug: &str, path: &str, pin: Option<i64>, status: &str) -> Self {
        Self {
            slug: slug.to_string(),
            path: path.to_string(),
            pinned_snapshot_id: pin,
            status: status.to_string(),
            unified_diff: None,
            truncated: false,
        }
    }
}

/// Compute drift for a single wiki file ref. Pure-ish: only reads
/// (page_ref pin, snapshot tree, blob content, working-tree file).
pub async fn compute_wiki_ref_drift(
    page_refs: &SqlitePageRefStore,
    snapshots: &SqliteSnapshotStore,
    blobs: &BlobStore,
    project_dir: &Path,
    slug: &str,
    path: &str,
) -> Result<WikiRefDrift, DomainError> {
    // Locate the wiki → file edge and its pin.
    let edges = page_refs.list_outbound(KIND_WIKI, slug, None).await?;
    let Some(edge) = edges
        .into_iter()
        .find(|e| e.target_kind == KIND_FILE && e.target_id == path)
    else {
        return Ok(WikiRefDrift::bare(slug, path, None, "not_a_ref"));
    };
    let Some(pin) = edge.local_snapshot_id else {
        return Ok(WikiRefDrift::bare(slug, path, None, "no_pin"));
    };

    // Pinned content: the file's blob as of the pinned snapshot. Absent
    // from the tree ⇒ the file didn't exist then (added since) ⇒ empty.
    let tree = snapshots.tree_at(pin).await?;
    let pinned = match tree.get(path) {
        Some(hash) if hash.starts_with("oversize:") => {
            return Ok(WikiRefDrift::bare(slug, path, Some(pin), "binary"));
        }
        Some(hash) => match String::from_utf8(read_blob(blobs, hash)?) {
            Ok(s) => s,
            Err(_) => return Ok(WikiRefDrift::bare(slug, path, Some(pin), "binary")),
        },
        None => String::new(),
    };

    // Current content: the working-tree file. Missing ⇒ deleted ⇒ empty.
    let current = match std::fs::read(project_dir.join(path)) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(_) => return Ok(WikiRefDrift::bare(slug, path, Some(pin), "binary")),
        },
        Err(_) => String::new(),
    };

    if pinned == current {
        return Ok(WikiRefDrift::bare(slug, path, Some(pin), "unchanged"));
    }
    let (diff, truncated) = unified_diff(&pinned, &current, path, MAX_DIFF_BYTES);
    Ok(WikiRefDrift {
        slug: slug.to_string(),
        path: path.to_string(),
        pinned_snapshot_id: Some(pin),
        status: "drifted".to_string(),
        unified_diff: Some(diff),
        truncated,
    })
}

fn read_blob(blobs: &BlobStore, hash: &str) -> Result<Vec<u8>, DomainError> {
    blobs
        .read(hash)
        .map_err(|e| DomainError::Storage(format!("blob read {hash}: {e}")))
}

/// Pure line-level unified diff of `old`→`new`, headed with `path`,
/// capped at `max_bytes` (truncated on a line boundary). Returns
/// `(diff_text, truncated)`.
pub fn unified_diff(old: &str, new: &str, path: &str, max_bytes: usize) -> (String, bool) {
    let diff = similar::TextDiff::from_lines(old, new);
    let full = diff
        .unified_diff()
        .context_radius(3)
        .header(&format!("{path} (pinned)"), &format!("{path} (current)"))
        .to_string();
    if full.len() <= max_bytes {
        return (full, false);
    }
    let mut out = String::with_capacity(max_bytes + 32);
    for line in full.split_inclusive('\n') {
        if out.len() + line.len() > max_bytes {
            break;
        }
        out.push_str(line);
    }
    out.push_str("… [diff truncated]\n");
    (out, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_diff_reports_changed_lines() {
        let (d, truncated) = unified_diff("a\nb\nc\n", "a\nB\nc\n", "x.txt", 16_000);
        assert!(!truncated);
        assert!(d.contains("x.txt (pinned)"));
        assert!(d.contains("-b"), "diff: {d}");
        assert!(d.contains("+B"), "diff: {d}");
    }

    async fn seed_stream(db: &oxplow_db::Database, id: i64) {
        use oxplow_domain::stores::StreamStore;
        oxplow_db::SqliteStreamStore::new(db.clone())
            .upsert(&oxplow_domain::Stream {
                id: oxplow_domain::StreamId::new(id),
                kind: oxplow_domain::StreamKind::Primary,
                title: "t".into(),
                branch: "main".into(),
                branch_ref: "refs/heads/main".into(),
                branch_source: "main".into(),
                worktree_path: "/r".into(),
                working_pane: String::new(),
                talking_pane: String::new(),
                working_session_id: String::new(),
                talking_session_id: String::new(),
                custom_prompt: None,
                created_at: oxplow_domain::Timestamp::from_unix_ms(0),
                updated_at: oxplow_domain::Timestamp::from_unix_ms(0),
                archived_at: None,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn compute_drift_diffs_pinned_blob_vs_disk() {
        use oxplow_db::{
            Database, FileSnapshot, PageRefEdge, SqlitePageRefStore, SqliteSnapshotStore,
        };
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path();
        let db = Database::in_memory();
        seed_stream(&db, 1).await;
        let page_refs = SqlitePageRefStore::new(db.clone());
        let snapshots = SqliteSnapshotStore::new(db.clone());
        let blobs = BlobStore::new(project.join(".oxplow/blobs"));

        // Capture the pinned content as snapshot N, blob in the store.
        let pinned = "alpha\nbeta\ngamma\n";
        let hash = blobs.write(pinned.as_bytes()).unwrap();
        let snap = snapshots
            .create_snapshot(oxplow_domain::StreamId::new(1))
            .await
            .unwrap();
        snapshots
            .capture(FileSnapshot {
                id: 0,
                stream_id: oxplow_domain::StreamId::new(1),
                path: "src/x.rs".into(),
                blob_hash: Some(hash),
                size_bytes: pinned.len() as i64,
                captured_at: oxplow_domain::Timestamp::from_unix_ms(0),
                storage: oxplow_db::SnapshotStorage::Oxplow,
                snapshot_id: Some(snap),
                mtime_ms: None,
            })
            .await
            .unwrap();
        // Pin the wiki→file edge at that snapshot.
        page_refs
            .upsert_edge(
                PageRefEdge::new("wiki", "intro", "file", "src/x.rs", "wiki_file_ref")
                    .with_version(snap, None, false),
            )
            .await
            .unwrap();

        // Drifted: disk differs from the pinned blob.
        std::fs::create_dir_all(project.join("src")).unwrap();
        std::fs::write(project.join("src/x.rs"), "alpha\nBETA\ngamma\n").unwrap();
        let drift =
            compute_wiki_ref_drift(&page_refs, &snapshots, &blobs, project, "intro", "src/x.rs")
                .await
                .unwrap();
        assert_eq!(drift.status, "drifted");
        assert_eq!(drift.pinned_snapshot_id, Some(snap));
        let d = drift.unified_diff.unwrap();
        assert!(d.contains("-beta") && d.contains("+BETA"), "diff: {d}");

        // Unchanged: disk matches the pinned blob.
        std::fs::write(project.join("src/x.rs"), pinned).unwrap();
        let same =
            compute_wiki_ref_drift(&page_refs, &snapshots, &blobs, project, "intro", "src/x.rs")
                .await
                .unwrap();
        assert_eq!(same.status, "unchanged");
        assert!(same.unified_diff.is_none());

        // not_a_ref: the page doesn't reference this path.
        let nr = compute_wiki_ref_drift(
            &page_refs,
            &snapshots,
            &blobs,
            project,
            "intro",
            "src/other.rs",
        )
        .await
        .unwrap();
        assert_eq!(nr.status, "not_a_ref");
    }

    #[tokio::test]
    async fn compute_drift_reports_no_pin_for_unpinned_edge() {
        use oxplow_db::{Database, PageRefEdge, SqlitePageRefStore, SqliteSnapshotStore};
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::in_memory();
        let page_refs = SqlitePageRefStore::new(db.clone());
        let snapshots = SqliteSnapshotStore::new(db.clone());
        let blobs = BlobStore::new(tmp.path().join(".oxplow/blobs"));
        // Edge with no version pin.
        page_refs
            .upsert_edge(PageRefEdge::new(
                "wiki",
                "intro",
                "file",
                "src/x.rs",
                "wiki_file_ref",
            ))
            .await
            .unwrap();
        let drift = compute_wiki_ref_drift(
            &page_refs,
            &snapshots,
            &blobs,
            tmp.path(),
            "intro",
            "src/x.rs",
        )
        .await
        .unwrap();
        assert_eq!(drift.status, "no_pin");
        assert_eq!(drift.pinned_snapshot_id, None);
    }

    #[test]
    fn unified_diff_truncates_on_line_boundary() {
        let old = "line\n".repeat(500);
        let new = (0..500).map(|i| format!("line{i}\n")).collect::<String>();
        let (d, truncated) = unified_diff(&old, &new, "big.txt", 200);
        assert!(truncated);
        assert!(d.len() <= 200 + "… [diff truncated]\n".len() + 8);
        assert!(d.ends_with("… [diff truncated]\n"));
        // Truncation lands on a newline boundary (no split mid-line).
        let body = d.strip_suffix("… [diff truncated]\n").unwrap();
        assert!(body.is_empty() || body.ends_with('\n'));
    }
}
