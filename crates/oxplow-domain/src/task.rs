//! An effort's declared impacts.

use serde::{Deserialize, Serialize};
use specta::Type;

/// One declared cross-page outcome of an effort — the LLM asserts
/// "this effort created/updated/deleted/referenced/resolved <kind>:<id>".
/// Stored as a JSON list on `effort.impacts_json` and projected
/// into the unified `page_ref` graph as outbound edges from the
/// owning task. Distinct from an effort's files (observed, never
/// declared) — impacts cover wiki pages, tasks, commits, findings,
/// directories, and files alike.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type, schemars::JsonSchema)]
pub struct TaskImpact {
    /// Page kind being impacted — `wiki | work_item | file | directory
    /// | git_commit | finding`, and no other (`oxplow.effort.report` refuses
    /// one). Projected to the canonical `page_ref` kinds (`git_commit`
    /// → `commit`, `directory` → `dir`; see `impact_kind`). A
    /// `work_item`'s id is its ref or one of the active list's own ids.
    pub kind: String,
    /// Canonical id for that page kind (slug, integer string, repo
    /// path, sha — see `page_ref_projections` docs).
    pub id: String,
    /// What the effort did. Free-form but conventionally one of
    /// `created | updated | deleted | referenced | resolved |
    /// completed | reopened`. Persisted in `source_extra` so the
    /// UI can render it without re-querying.
    pub action: Option<String>,
}
