//! The kind-agnostic attribution ledger store (`effort_attribution`, V40) —
//! claim / acknowledge / unattributed state per `(effort, kind, ref)` for every
//! non-file attribution kind (`run`, `coverage`, …). Files keep their own
//! tables; see `crates/oxplow-app/src/attribution.rs` for the engine. Raw
//! integer ids at this layer (the service/IPC boundary maps prefixed ids).

use rusqlite::params;

use oxplow_domain::{DomainError, EffortId, Timestamp};

use crate::database::{canonical_ts, Database};

/// `Timestamp` → the fixed-width canonical string the `recorded_at` column
/// stores (so lexicographic ordering matches chronological). Mirrors the
/// per-store helper in `effort_store`/`metric_store`.
fn ts_to_string(ts: Timestamp) -> String {
    let raw = serde_json::to_string(&ts)
        .expect("Timestamp serializes to JSON")
        .trim_matches('"')
        .to_string();
    canonical_ts(&raw)
}

/// The attribution states a `(effort, kind, ref)` can hold — exactly one at a
/// time (the claim-first invariant, enforced by the row's primary key).
pub const STATE_CLAIMED: &str = "claimed";
pub const STATE_UNATTRIBUTED: &str = "unattributed";
pub const STATE_ACKNOWLEDGED: &str = "acknowledged";

#[derive(Clone)]
pub struct SqliteAttributionStore {
    db: Database,
}

impl SqliteAttributionStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Set the state of one `(effort, kind, ref)` (upsert). Overwriting is the
    /// state transition — e.g. claiming a previously-unattributed ref replaces
    /// its row, so it's in exactly one state.
    pub async fn set_state(
        &self,
        effort_id: &EffortId,
        kind: &str,
        ref_: &str,
        state: &str,
        detail_json: Option<&str>,
    ) -> Result<(), DomainError> {
        let (eid, kind, ref_, state) = (
            effort_id.value(),
            kind.to_string(),
            ref_.to_string(),
            state.to_string(),
        );
        // A CLAIM is globally exclusive per `(kind, ref)`: a run has at most one
        // owning effort (tsk267). Claiming a ref for this effort removes any
        // OTHER effort's row for it, so concurrent efforts can't both claim the
        // same run and double-count it in rollups. (`unattributed`/`acknowledged`
        // stay per-effort — only a claim takes exclusive ownership.)
        let is_claim = state == STATE_CLAIMED;
        let detail = detail_json.map(str::to_string);
        let now = ts_to_string(Timestamp::now());
        self.db
            .call(move |conn| {
                if is_claim {
                    conn.execute(
                        "DELETE FROM effort_attribution
                         WHERE kind = ?1 AND ref = ?2 AND effort_id != ?3",
                        params![kind, ref_, eid],
                    )?;
                }
                conn.execute(
                    "INSERT OR REPLACE INTO effort_attribution
                       (effort_id, kind, ref, state, detail_json, recorded_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![eid, kind, ref_, state, detail, now],
                )?;
                Ok(())
            })
            .await
    }

    /// All `ref`s for `(effort, kind)` in `state`, sorted.
    pub async fn list_refs(
        &self,
        effort_id: &EffortId,
        kind: &str,
        state: &str,
    ) -> Result<Vec<String>, DomainError> {
        let (eid, kind, state) = (effort_id.value(), kind.to_string(), state.to_string());
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT ref FROM effort_attribution
                     WHERE effort_id = ?1 AND kind = ?2 AND state = ?3
                     ORDER BY ref",
                )?;
                let rows = stmt.query_map(params![eid, kind, state], |r| r.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    /// Replace this effort+kind's `unattributed` residue with `refs` (the
    /// reconcile output). Touches only `unattributed` rows — `claimed`/
    /// `acknowledged` are left intact. `refs` already exclude those (the
    /// reconcile subtracts them), so a plain insert can't collide.
    pub async fn replace_unattributed(
        &self,
        effort_id: &EffortId,
        kind: &str,
        refs: &[String],
    ) -> Result<(), DomainError> {
        let (eid, kind) = (effort_id.value(), kind.to_string());
        let refs = refs.to_vec();
        let now = ts_to_string(Timestamp::now());
        self.db
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM effort_attribution
                     WHERE effort_id = ?1 AND kind = ?2 AND state = 'unattributed'",
                    params![eid, kind],
                )?;
                for r in &refs {
                    conn.execute(
                        "INSERT OR IGNORE INTO effort_attribution
                           (effort_id, kind, ref, state, detail_json, recorded_at)
                         VALUES (?1, ?2, ?3, 'unattributed', NULL, ?4)",
                        params![eid, kind, r, now],
                    )?;
                }
                Ok(())
            })
            .await
    }

    /// `ref`s of `kind` that ANOTHER effort already `claimed` — not this
    /// effort's to flag (the cross-effort dedup; the runs analog of files'
    /// `paths_claimed_by_intervening_efforts`). Sorted, distinct.
    pub async fn refs_claimed_by_other_efforts(
        &self,
        effort_id: &EffortId,
        kind: &str,
    ) -> Result<Vec<String>, DomainError> {
        let (eid, kind) = (effort_id.value(), kind.to_string());
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT ref FROM effort_attribution
                     WHERE kind = ?1 AND state = 'claimed' AND effort_id != ?2
                     ORDER BY ref",
                )?;
                let rows = stmt.query_map(params![kind, eid], |r| r.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// stream + thread + task + two efforts so the FK + cross-effort dedup
    /// have rows to reference.
    async fn fixture() -> (SqliteAttributionStore, EffortId, EffortId) {
        let db = Database::in_memory();
        let db2 = db.clone();
        tokio::task::spawn_blocking(move || {
            db2.with_conn(|conn| {
                let now = "2026-06-26T00:00:00.000000Z";
                conn.execute(
                    "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                     VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'main', '/r', ?1, ?1)",
                    [now],
                )?;
                conn.execute(
                    "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                     VALUES (1, 1, 't', 'active', ?1, ?1)",
                    [now],
                )?;
                conn.execute(
                    "INSERT INTO task (id, thread_id, title, status, priority, created_by, created_at, updated_at)
                     VALUES (1, 1, 't1', 'in_progress', 'medium', 'user', ?1, ?1)",
                    [now],
                )?;
                conn.execute(
                    "INSERT INTO task (id, thread_id, title, status, priority, created_by, created_at, updated_at)
                     VALUES (2, 1, 't2', 'in_progress', 'medium', 'user', ?1, ?1)",
                    [now],
                )?;
                conn.execute(
                    "INSERT INTO task_effort (id, task_id, thread_id, started_at)
                     VALUES (1, 1, 1, ?1), (2, 2, 1, ?1)",
                    [now],
                )?;
                Ok(())
            })
        })
        .await
        .unwrap()
        .unwrap();
        (
            SqliteAttributionStore::new(db),
            EffortId::new(1),
            EffortId::new(2),
        )
    }

    #[tokio::test]
    async fn state_is_single_valued_and_claiming_overrides() {
        let (store, e1, _e2) = fixture().await;
        store
            .set_state(&e1, "run", "run:5", STATE_UNATTRIBUTED, None)
            .await
            .unwrap();
        assert_eq!(
            store
                .list_refs(&e1, "run", STATE_UNATTRIBUTED)
                .await
                .unwrap(),
            vec!["run:5"]
        );
        // Claiming the same ref overrides — now claimed, no longer unattributed.
        store
            .set_state(&e1, "run", "run:5", STATE_CLAIMED, None)
            .await
            .unwrap();
        assert!(store
            .list_refs(&e1, "run", STATE_UNATTRIBUTED)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            store.list_refs(&e1, "run", STATE_CLAIMED).await.unwrap(),
            vec!["run:5"]
        );
    }

    #[tokio::test]
    async fn replace_unattributed_leaves_claimed_intact() {
        let (store, e1, _e2) = fixture().await;
        store
            .set_state(&e1, "run", "run:1", STATE_CLAIMED, None)
            .await
            .unwrap();
        store
            .replace_unattributed(&e1, "run", &["run:2".into(), "run:3".into()])
            .await
            .unwrap();
        // Re-replace with a smaller set — old unattributed cleared, claimed kept.
        store
            .replace_unattributed(&e1, "run", &["run:2".into()])
            .await
            .unwrap();
        assert_eq!(
            store
                .list_refs(&e1, "run", STATE_UNATTRIBUTED)
                .await
                .unwrap(),
            vec!["run:2"]
        );
        assert_eq!(
            store.list_refs(&e1, "run", STATE_CLAIMED).await.unwrap(),
            vec!["run:1"]
        );
    }

    #[tokio::test]
    async fn other_efforts_claims_are_visible_for_dedup() {
        let (store, e1, e2) = fixture().await;
        store
            .set_state(&e2, "run", "run:9", STATE_CLAIMED, None)
            .await
            .unwrap();
        // From e1's view, run:9 is owned by another effort.
        assert_eq!(
            store
                .refs_claimed_by_other_efforts(&e1, "run")
                .await
                .unwrap(),
            vec!["run:9"]
        );
        // e1's own claim is NOT "another effort's".
        store
            .set_state(&e1, "run", "run:1", STATE_CLAIMED, None)
            .await
            .unwrap();
        assert_eq!(
            store
                .refs_claimed_by_other_efforts(&e1, "run")
                .await
                .unwrap(),
            vec!["run:9"]
        );
    }

    #[tokio::test]
    async fn claiming_a_ref_is_exclusive_across_efforts() {
        // tsk267: a run has at most one owning effort. When e2 claims a ref e1
        // already claimed (or holds as unattributed), e1's row is removed — so
        // the run can't be double-counted in two efforts' rollups.
        let (store, e1, e2) = fixture().await;
        store
            .set_state(&e1, "run", "run:9", STATE_CLAIMED, None)
            .await
            .unwrap();
        store
            .set_state(&e2, "run", "run:9", STATE_CLAIMED, None)
            .await
            .unwrap();
        assert!(
            store
                .list_refs(&e1, "run", STATE_CLAIMED)
                .await
                .unwrap()
                .is_empty(),
            "e1's claim is displaced by e2's exclusive claim"
        );
        assert_eq!(
            store.list_refs(&e2, "run", STATE_CLAIMED).await.unwrap(),
            vec!["run:9"]
        );
        // A disclaim (acknowledged) is per-effort and does NOT displace others.
        store
            .set_state(&e1, "run", "run:9", STATE_ACKNOWLEDGED, None)
            .await
            .unwrap();
        assert_eq!(
            store.list_refs(&e2, "run", STATE_CLAIMED).await.unwrap(),
            vec!["run:9"],
            "e2 keeps its claim when e1 merely acknowledges the ref"
        );
    }
}
