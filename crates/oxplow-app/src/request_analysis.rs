//! Keeps each request's stored analysis current (`v_request_metric_delta`,
//! `v_request_observation`): what the metric engine computes over a
//! request's window on one stream — the span of its turns there, the files
//! they changed (`v_turn_file`, claimed or sole) and the captures they made
//! — which pages can't compute. An **asset** over the tables it reads
//! ([`RequestAnalysis`]): a turn opening or ending, a take's files, a run
//! landing recompute the requests that are due once their inputs go quiet.
//! Due is a request with a turn running, one whose stored analysis is
//! stale (its signature moved: a run reported late, a file captured after
//! the turn closed), or one that ended in the last day with none stored.
//! `.context/collection.md` "A request's analysis".

use std::sync::{Arc, Weak};

use crate::assets::{Materializer, Recomputed};
use crate::collection::Window;

/// The asset: every due request's analysis.
pub const ASSET: &str = "request_analysis";

/// What the analysis reads: captures and facts through the **cube asset**
/// (so its series reads are cube-served once the cube has folded what
/// landed), a turn's files, the specs, and the turns themselves.
const INPUTS: [&str; 4] = [
    crate::metric_cube::ASSET,
    "turn_file",
    "metric_spec",
    "agent_turn",
];

/// How far back a request with no stored analysis is due: older ones are
/// analyzed when asked for.
const RECENT_HOURS: i64 = 24;

/// The window `request` covers on `stream`; `None` when it has no turn
/// there.
pub(crate) async fn window(
    svc: &crate::Services,
    request: i64,
    stream: i64,
) -> Result<Option<(Window, oxplow_db::AnalysisSpan)>, oxplow_domain::DomainError> {
    let Some(span) = svc
        .db
        .read(move |c| crate::change_analysis::request_span(c, request, Some(stream)))
        .await?
    else {
        return Ok(None);
    };
    let mut files: Vec<String> = crate::change_analysis::request_files(
        svc,
        &request.to_string(),
        oxplow_domain::StreamId::new(stream),
    )
    .await?
    .into_iter()
    .collect();
    files.sort();
    let parse = |t: &str| {
        oxplow_domain::Timestamp::parse(t)
            .map_err(|e| oxplow_domain::DomainError::Invalid(format!("turn time {t}: {e}")))
    };
    let window = Window {
        key: format!(
            "{} {stream}",
            oxplow_domain::uri::build::request_uri(oxplow_domain::RequestId::new(request))
        ),
        stream: Some(stream),
        started_at: parse(&span.began)?,
        ended_at: span.ended.as_deref().map(parse).transpose()?,
        start_snapshot_id: span.start,
        end_snapshot_id: (!span.running).then_some(span.end).flatten(),
        files,
        members: oxplow_db::Members::Turns(span.turns.clone()),
    };
    let stored = oxplow_db::AnalysisSpan {
        started_at: span.began,
        ended_at: span.ended,
        start_snapshot_id: window.start_snapshot_id,
        end_snapshot_id: window.end_snapshot_id,
    };
    Ok(Some((window, stored)))
}

/// Recompute `request`'s analysis on `stream` and store it.
pub(crate) async fn refresh(
    svc: &crate::Services,
    request: i64,
    stream: i64,
) -> Result<(), oxplow_domain::DomainError> {
    // What it's computed from, read first: an input that lands while it
    // computes leaves it stale, so it's computed again.
    let sig = svc.request_analysis_store.sig(request, stream).await?;
    let Some((window, span)) = window(svc, request, stream).await? else {
        return Ok(());
    };
    let deltas = svc.collection.metric_deltas(&window).await;
    let observations = svc.collection.observations(&window, None).await;
    svc.request_analysis_store
        .replace(request, stream, span, deltas, observations, sig)
        .await
}

/// Every due request's analysis, recomputed when what it reads moves.
pub struct RequestAnalysis {
    svc: Weak<crate::Services>,
}

#[async_trait::async_trait]
impl Materializer for RequestAnalysis {
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
        let since = oxplow_db::ts_to_string(oxplow_domain::Timestamp(
            oxplow_domain::Timestamp::now().0 - time::Duration::hours(RECENT_HOURS),
        ));
        for (request, stream) in svc.request_analysis_store.due(since).await? {
            if let Err(error) = refresh(&svc, request, stream).await {
                tracing::warn!(request, stream, %error, "refreshing a request's analysis failed");
            }
        }
        Ok(Recomputed::default())
    }
}

/// Register the asset.
pub fn register(state: &Arc<crate::Services>) {
    state.assets.register(Arc::new(RequestAnalysis {
        svc: Arc::downgrade(state),
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request's analysis holds its turns' runs and nobody else's, and
    /// one whose inputs haven't moved isn't due again; a run of its turn
    /// reported later makes it due.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_requests_analysis_holds_its_turns_runs_and_is_recomputed_only_when_they_move() {
        let f = crate::test_fixtures::services_with_thread().await;
        let turn: i64 = f
            .svc
            .db
            .transaction(|c| {
                c.execute("INSERT INTO request (id, thread_id) VALUES (1, 1)", [])
                    .map_err(oxplow_db::map_sql_err)?;
                c.execute(
                    "INSERT INTO agent_turn (thread_id, prompt, request_id, started_at)
                     VALUES (1, 'p', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-1 minute'))",
                    [],
                )
                .map_err(oxplow_db::map_sql_err)?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap();
        let run = || async {
            f.svc
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
                    "oxplow.test.record_run",
                    None,
                    None,
                )
                .await
                .unwrap()
                .expect("a run")
        };
        let stamp = |capture: i64, turn: Option<i64>| {
            let db = f.svc.db.clone();
            async move {
                db.transaction(move |c| {
                    c.execute(
                        "UPDATE metric_capture SET turn_id = ?2 WHERE id = ?1",
                        rusqlite::params![capture, turn],
                    )
                    .map_err(oxplow_db::map_sql_err)
                })
                .await
                .unwrap();
            }
        };
        // One run of the request's turn, one of nobody's.
        stamp(run().await, Some(turn)).await;
        stamp(run().await, None).await;
        let asset = RequestAnalysis {
            svc: Arc::downgrade(&f.svc),
        };
        asset.recompute(false).await.unwrap();
        let runs = || async {
            f.svc
                .db
                .read(|c| {
                    c.query_row(
                        "SELECT count(*) FROM request_observation
                          WHERE request_id = 1 AND stream_id = 1 AND kind = 'test-run'",
                        [],
                        |r| r.get::<_, i64>(0),
                    )
                    .map_err(oxplow_db::map_sql_err)
                })
                .await
                .unwrap()
        };
        assert_eq!(runs().await, 1);
        let due = || async {
            f.svc
                .request_analysis_store
                .due(String::new())
                .await
                .unwrap()
        };
        assert_eq!(due().await, vec![], "nothing it reads moved");
        stamp(run().await, Some(turn)).await;
        assert_eq!(due().await, vec![(1, 1)], "a run of its turn landed");
        asset.recompute(false).await.unwrap();
        assert_eq!(runs().await, 2);
    }
}
