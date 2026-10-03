//! Keeps `v_effort_metric_delta` / `v_effort_observation` current. The
//! metric engine computes an effort's deltas and evidence; lenses can't, so
//! core stores them: when an effort closes (the `effort.evidence` reaction
//! to `effort.finished`), and for every open effort as an **asset** over
//! the tables the evidence reads ([`OpenEffortEvidence`], P7.B6) — a commit
//! to a capture, a fact, a run claim or a token row recomputes it once its
//! inputs go quiet. Its own tables aren't inputs, so it can't loop. The
//! renderer hears the rows move as `ModelsChanged`. See
//! `.context/semantic-layer.md`.

use std::sync::{Arc, Weak};

use crate::assets::{Materializer, Recomputed};

/// The asset: every open effort's evidence.
pub const ASSET: &str = "effort_evidence";

/// What the evidence reads: metric captures and facts (deltas and
/// observations), run claims (`effort_attribution`), token usage.
const INPUTS: [&str; 4] = [
    "metric_capture",
    "fact",
    "effort_attribution",
    "agent_token_usage",
];

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
}
