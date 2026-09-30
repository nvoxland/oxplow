//! Capture-time helper that resolves the `(local_snapshot_id,
//! closest_vcs_rev, vcs_rev_exact)` triple stamped on every
//! file reference. Called by the effort-file recorder and the wiki
//! ref sync so the agent doesn't have to think about it.
//!
//! Resolution policy (matches V20 docs):
//!   * `local_snapshot_id` is the snapshot the caller already pinned
//!     the reference to (effort end snapshot, current wiki snapshot,
//!     etc.).
//!   * If that snapshot row has a `revision` (clean workspace at
//!     capture or re-stamped later), use its id with
//!     `vcs_rev_exact = true`.
//!   * Otherwise read the head via `Vcs::head` and stamp it with
//!     `vcs_rev_exact = false`. The snapshot store's revision stamp
//!     flips exact -> true if and when the snapshot itself gets a
//!     revision attached.
//!   * If neither is available (no commit yet, headless repo) the
//!     `closest_vcs_rev` stays `None`.

use std::path::Path;
use std::sync::Arc;

use oxplow_db::SqliteSnapshotStore;
use oxplow_domain::vcs::Vcs;
use oxplow_domain::DomainError;

/// Resolved version triple ready to stamp onto a file ref. Owned
/// (no borrows) so callers can pass it across `spawn_blocking`
/// boundaries without lifetime gymnastics.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedFileVersion {
    pub local_snapshot_id: i64,
    pub closest_vcs_rev: Option<String>,
    pub vcs_rev_exact: bool,
}

impl ResolvedFileVersion {
    /// Borrowed view suitable for [`oxplow_db::EffortStore::record_file`].
    pub fn as_ref(&self) -> oxplow_db::FileRefVersion<'_> {
        oxplow_db::FileRefVersion {
            local_snapshot_id: self.local_snapshot_id,
            closest_vcs_rev: self.closest_vcs_rev.as_deref(),
            vcs_rev_exact: self.vcs_rev_exact,
        }
    }
}

/// The version triple for a ref pinned to `local_snapshot_id` in
/// workspace `ws`.
pub async fn resolve(
    snapshot_store: &Arc<SqliteSnapshotStore>,
    vcs: &dyn Vcs,
    ws: &Path,
    local_snapshot_id: i64,
) -> Result<ResolvedFileVersion, DomainError> {
    if let Some(rev) = snapshot_store
        .get_snapshot_revision(local_snapshot_id)
        .await?
        .and_then(|r| r.vcs_rev().map(str::to_string))
    {
        return Ok(ResolvedFileVersion {
            local_snapshot_id,
            closest_vcs_rev: Some(rev),
            vcs_rev_exact: true,
        });
    }
    let head = vcs.head(ws).await.ok().and_then(|h| h.revision);
    Ok(ResolvedFileVersion {
        local_snapshot_id,
        closest_vcs_rev: head,
        vcs_rev_exact: false,
    })
}
