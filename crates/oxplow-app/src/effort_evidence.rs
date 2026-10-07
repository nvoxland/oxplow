//! Keeps `v_effort_metric_delta` / `v_effort_observation` current. The
//! metric engine computes an effort's deltas and evidence; lenses can't, so
//! core stores them: when an effort closes (the `effort.evidence` reaction
//! to `effort.finished`), and for every open effort as an **asset** over
//! the tables the evidence reads ([`OpenEffortEvidence`], P7.B6) — a commit
//! to a capture, a fact or a run claim recomputes it once its inputs go
//! quiet. The same asset recomputes a **closed** effort whose
//! claims moved since its evidence was stored (`effort_evidence_state`,
//! tsk889): a run claimed after the close lands. Its own tables aren't inputs, so it can't loop. The
//! renderer hears the rows move as `ModelsChanged`. See
//! `.context/semantic-layer.md`.

use std::sync::{Arc, Weak};

use crate::assets::{Materializer, Recomputed};

/// The asset: every open effort's evidence.
pub const ASSET: &str = "effort_evidence";

/// What the evidence reads: metric captures and facts (deltas and
/// observations; a run is a capture stamped with its effort) and the
/// effort's files (the File family's deltas). Captures and facts reach it
/// through the **cube asset**, which reads them: the evidence recomputes
/// after the cube has folded what landed, never while it's pending, so its
/// series reads are cube-served instead of folding a measure's whole
/// history (which at startup, racing the cube's backfill, cost gigabytes).
const INPUTS: [&str; 2] = [crate::metric_cube::ASSET, "effort_file"];

/// Recompute one effort's evidence; failures are logged.
pub(crate) async fn refresh(state: &crate::Services, effort_id: i64) {
    let id = oxplow_domain::EffortId::new(effort_id).to_string();
    if let Err(error) = state
        .collection
        .refresh_effort_evidence(&id, &state.effort_evidence_store)
        .await
    {
        tracing::warn!(effort_id, %error, "refreshing effort evidence failed");
    }
}

/// Every open effort's evidence, recomputed when what it reads moves.
pub struct OpenEffortEvidence {
    svc: Weak<crate::Services>,
}

#[async_trait::async_trait]
impl Materializer for OpenEffortEvidence {
    fn asset(&self) -> &str {
        ASSET
    }

    fn inputs(&self) -> Vec<String> {
        INPUTS.iter().map(|t| t.to_string()).collect()
    }

    async fn recompute(&self, _full: bool) -> Result<Recomputed, oxplow_domain::DomainError> {
        let Some(svc) = self.svc.upgrade() else {
            return Ok(Recomputed::default());
        };
        for e in svc.effort_store.list_all_open().await? {
            refresh(&svc, e.id.value()).await;
        }
        // A closed effort whose runs or files moved since its evidence was
        // stored — a run collected late, an adoption's restamp.
        for id in svc.effort_evidence_store.stale_closed().await? {
            refresh(&svc, id).await;
        }
        Ok(Recomputed::default())
    }
}

/// Register the asset.
pub fn register(state: &Arc<crate::Services>) {
    state.assets.register(Arc::new(OpenEffortEvidence {
        svc: Arc::downgrade(state),
    }));
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// P7.B6: open efforts' evidence is an asset over the tables it
    /// reads — a commit to one recomputes it (its `asset_state` row), with
    /// no in-memory bus in the loop.
    #[tokio::test]
    async fn open_effort_evidence_is_an_asset_over_its_inputs() {
        let f = crate::test_fixtures::services_with_effort().await;
        register(&f.svc);
        f.svc
            .assets
            .changed(&oxplow_db::changes::Changed::inserted(["fact".to_string()]));
        let computed = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let at: Option<String> = f
                    .svc
                    .db
                    .read(|c| {
                        use rusqlite::OptionalExtension;
                        c.query_row(
                            "SELECT computed_at FROM asset_state WHERE asset = ?1",
                            [ASSET],
                            |r| r.get(0),
                        )
                        .optional()
                        .map_err(oxplow_db::map_sql_err)
                    })
                    .await
                    .unwrap();
                if at.is_some() {
                    return at;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the evidence recomputed after its input moved");
        assert!(computed.is_some());
    }

    /// A run that becomes an effort's after it closed (a late collection of
    /// a tool call it held, an adoption's restamp) lands in its evidence:
    /// the asset recomputes every closed effort whose runs moved.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_run_attributed_after_the_close_lands_in_the_closed_efforts_evidence() {
        let f = crate::test_fixtures::services_with_effort().await;
        let effort = f.effort.value();
        f.svc
            .db
            .transaction(move |tx| {
                tx.execute(
                    "UPDATE effort SET ended_at = ?2 WHERE id = ?1",
                    rusqlite::params![effort, oxplow_domain::Timestamp::now().to_string()],
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        refresh(&f.svc, effort).await;
        let runs = || async {
            f.svc
                .effort_evidence_store
                .list_observations(effort, Some("test-run".into()))
                .await
                .unwrap()
                .len()
        };
        assert_eq!(runs().await, 0);
        let run = f
            .svc
            .collection
            .record_test_run(
                &f.thread,
                "cargo test",
                Some(0),
                Some(10),
                Some(1),
                Some(0),
                Some(1),
                "asserted",
                "test.record_run",
                None,
                None,
            )
            .await
            .unwrap()
            .expect("a run");
        f.svc
            .db
            .transaction(move |tx| {
                tx.execute(
                    "UPDATE metric_capture SET effort_id = ?2 WHERE id = ?1",
                    rusqlite::params![run, effort],
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        OpenEffortEvidence {
            svc: Arc::downgrade(&f.svc),
        }
        .recompute(false)
        .await
        .unwrap();
        assert_eq!(runs().await, 1);
    }
}
