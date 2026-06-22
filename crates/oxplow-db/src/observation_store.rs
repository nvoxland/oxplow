//! The `EffortObservation` wire type — the shape the effort-review panel reads.
//!
//! The legacy `effort_observation` table + its store were **retired** (tsk215):
//! coverage/test/analysis facts now live in the metric substrate
//! (`metric_sample` + `metric_finding`), and the IPC reconstructs these rows
//! from there (`CollectionService::effort_observations_from_metrics`). This type
//! survives only as the read/IPC shape so the renderer is unchanged.

use serde::{Deserialize, Serialize};
use specta::Type;

use oxplow_domain::Timestamp;

/// One effort-review observation row (reconstructed from the metric substrate).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct EffortObservation {
    pub id: i64,
    pub stream_id: String,
    pub effort_id: String,
    /// Well-known kind: `test-run` | `diff-coverage` | `static-analysis`
    /// (open-ended).
    pub kind: String,
    /// `observed` (oxplow saw it directly) | `asserted` (agent reported it).
    pub provenance: String,
    /// Free-form origin tag, e.g. `post-tool-bash` / `agent`.
    pub source: String,
    /// Headline numeric (e.g. coverage %); kind-specific, nullable.
    pub metric_value: Option<f64>,
    /// Kind-specific structured payload (parsed by the UI, opaque to Rust).
    pub payload_json: Option<String>,
    /// Freshness pin — the snapshot this was captured against.
    pub local_snapshot_id: Option<i64>,
    pub closest_git_version: Option<String>,
    pub git_version_exact: bool,
    pub created_at: Timestamp,
}
