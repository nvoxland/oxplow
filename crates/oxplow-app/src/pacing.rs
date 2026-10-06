//! Paced collector runs (tsk1092): an `on:` collector whose trigger sets
//! `settle`, `at_most` or `idle` doesn't run as its event arrives. The
//! `collector.triggers` consumer records it pending (`pending_run`,
//! the latest event it'll run for) and this module runs it once every
//! condition holds, then clears it. See `.context/metrics.md` → "Pacing".

use std::time::Duration;

use oxplow_config::collectors::Pacing;

/// Whether a pending run may go now. `since_touched` is how long since a
/// triggering event last arrived, `since_run` how long since the
/// collector last ran (`None`: never), `idle` how long the project has been
/// idle (`None`: an agent turn is running).
pub fn due(
    pacing: &Pacing,
    since_touched: Duration,
    since_run: Option<Duration>,
    idle: Option<Duration>,
) -> bool {
    let secs = |s: u32| Duration::from_secs(u64::from(s));
    pacing.settle_secs.is_none_or(|s| since_touched >= secs(s))
        && pacing
            .at_most_secs
            .is_none_or(|s| since_run.is_none_or(|r| r >= secs(s)))
        && pacing
            .idle_secs
            .is_none_or(|s| idle.is_some_and(|i| i >= secs(s)))
}

/// How long the project has been idle at `now`: since the last change
/// (a take that recorded files) or agent activity. `None` while an agent
/// turn is running (an open `agent_turn`; boot recovery closes orphans).
pub async fn idle_for(
    svc: &crate::Services,
    now: oxplow_domain::Timestamp,
) -> Result<Option<Duration>, oxplow_domain::DomainError> {
    let (running, last): (i64, Option<String>) = svc
        .db
        .read(|c| {
            c.query_row(
                "SELECT (SELECT count(*) FROM agent_turn WHERE ended_at IS NULL),
                        (SELECT max(at) FROM event_log
                          WHERE payload_expired_at IS NULL
                            AND (type IN ('agent.turn.started', 'agent.turn.ended', 'agent.tool.finished')
                                 OR (type = 'snapshot.taken'
                                     AND json_extract(payload, '$.unchanged') = 0
                                     AND json_extract(payload, '$.file_count') > 0)))",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await?;
    if running > 0 {
        return Ok(None);
    }
    Ok(Some(
        match last.and_then(|t| oxplow_domain::Timestamp::parse(&t).ok()) {
            Some(at) => between(at, now),
            // Nothing has ever happened: idle as long as can be.
            None => Duration::MAX,
        },
    ))
}

/// `now - then`, zero when `then` is later.
fn between(then: oxplow_domain::Timestamp, now: oxplow_domain::Timestamp) -> Duration {
    Duration::from_millis(u64::try_from(now.unix_ms() - then.unix_ms()).unwrap_or(0))
}

/// A collector `on:` some event with pacing: an entity collector (an
/// extension's) or a fact collector (any owner's).
enum Paced {
    Entity(String, oxplow_config::collectors::CollectorSpec),
    Fact(crate::metrics_service::FactCollector),
    /// Core's deep change analysis of a working tree or open effort
    /// (`crate::change_reactor`, tsk1093).
    Change(crate::change_analysis::ChangeTarget, Pacing),
}

impl Paced {
    fn pacing(&self) -> Option<&Pacing> {
        let trigger = match self {
            Paced::Entity(_, spec) => &spec.trigger,
            Paced::Fact(c) => &c.trigger,
            Paced::Change(_, pacing) => return Some(pacing),
        };
        match trigger {
            oxplow_config::collectors::Trigger::On { pacing, .. } => Some(pacing),
            _ => None,
        }
    }
}

/// How long since `paced` last ran: a collector's last `collector_run`, a
/// change's last finished analysis.
async fn since_run(
    svc: &crate::Services,
    paced: &Paced,
    owner: &str,
    id: &str,
    now: oxplow_domain::Timestamp,
) -> Result<Option<Duration>, oxplow_domain::DomainError> {
    let last = match paced {
        Paced::Change(target, _) => {
            let (kind, key) = match target {
                crate::change_analysis::ChangeTarget::Working { stream_id } => (
                    "working",
                    oxplow_domain::StreamId::try_from_str(stream_id).map(|s| s.value()),
                ),
                crate::change_analysis::ChangeTarget::Effort { effort_id } => (
                    "effort",
                    oxplow_domain::EffortId::try_from_str(effort_id).map(|e| e.value()),
                ),
                _ => return Ok(None),
            };
            let Some(key) = key else { return Ok(None) };
            svc.db
                .read(move |c| {
                    c.query_row(
                        "SELECT max(computed_at) FROM change
                          WHERE kind = ?1 AND (CASE ?1 WHEN 'working' THEN stream_id
                                                       ELSE CAST(target AS INTEGER) END) = ?2",
                        rusqlite::params![kind, key],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .map_err(oxplow_db::map_sql_err)
                })
                .await?
        }
        _ => svc
            .collector_store
            .run_of(owner, id)
            .await?
            .map(|r| r.last_run_at),
    };
    Ok(last
        .and_then(|t| oxplow_domain::Timestamp::parse(&t).ok())
        .map(|t| between(t, now)))
}

/// The job `owner`/`id` names, if it's still paced: a collector that runs
/// `on:` events, or core's change analysis.
fn find(svc: &crate::Services, owner: &str, id: &str) -> Option<Paced> {
    if owner == crate::change_reactor::CORE {
        return crate::change_reactor::target_of(id)
            .map(|t| Paced::Change(t, crate::change_reactor::deep_pacing()));
    }
    if let Some(c) = svc
        .metrics
        .fact_collectors()
        .into_iter()
        .find(|c| c.owner == owner && c.key == id)
    {
        return Some(Paced::Fact(c));
    }
    svc.extension_catalog
        .get(&svc.layout.project_dir)
        .iter()
        .filter(|e| e.name == owner)
        .flat_map(|e| e.collectors.iter())
        .find(|c| c.id == id && c.facts.is_empty())
        .map(|c| Paced::Entity(owner.to_string(), c.clone()))
}

/// Run every pending collector that's due at `now`, once, for the latest
/// event it waited on, and clear it — unless a newer event deferred it
/// again while it ran. One whose collector is gone, no longer paced, or
/// whose event has expired is dropped. Returns how many ran.
pub async fn run_due(
    svc: &crate::Services,
    now: oxplow_domain::Timestamp,
) -> Result<usize, oxplow_domain::DomainError> {
    let pending = svc.collector_store.list_pending().await?;
    if pending.is_empty() {
        return Ok(0);
    }
    let idle = idle_for(svc, now).await?;
    let mut ran = 0;
    for p in pending {
        let Some(paced) = find(svc, &p.owner, &p.id) else {
            svc.collector_store
                .clear_pending(&p.owner, &p.id, i64::MAX)
                .await?;
            continue;
        };
        let Some(pacing) = paced.pacing() else {
            svc.collector_store
                .clear_pending(&p.owner, &p.id, i64::MAX)
                .await?;
            continue;
        };
        let since_touched = oxplow_domain::Timestamp::parse(&p.touched)
            .map(|t| between(t, now))
            .unwrap_or(Duration::MAX);
        let since_run = since_run(svc, &paced, &p.owner, &p.id, now).await?;
        if !due(pacing, since_touched, since_run, idle) {
            continue;
        }
        match run_one(svc, paced, &p).await {
            Ok(true) => ran += 1,
            Ok(false) => {}
            Err(oxplow_domain::DomainError::Busy(_)) => continue,
            Err(e) => return Err(e),
        }
        svc.collector_store
            .clear_pending(&p.owner, &p.id, p.event_seq)
            .await?;
    }
    Ok(ran)
}

/// Run one due job. Whether it ran — a job with nothing to run (a closed
/// effort's analysis, an expired event's collector) is just cleared. A
/// change analysis already running is `Busy`: the row stays for the next
/// pass.
async fn run_one(
    svc: &crate::Services,
    paced: Paced,
    p: &oxplow_db::PendingRun,
) -> Result<bool, oxplow_domain::DomainError> {
    use oxplow_domain::DomainError;
    let event = || async {
        Ok::<_, DomainError>(
            svc.event_log_store
                .get_by_seq(p.event_seq)
                .await?
                .map(std::sync::Arc::new),
        )
    };
    match paced {
        Paced::Change(target, _) => match crate::change_analysis::refresh_change(svc, target).await
        {
            Ok(_) => Ok(true),
            Err(DomainError::NotFound) => Ok(false),
            Err(DomainError::Busy(m)) => Err(DomainError::Busy(m)),
            Err(e) => {
                tracing::warn!(job = %p.id, error = %e, "paced change analysis failed");
                Ok(false)
            }
        },
        Paced::Fact(c) => {
            let Some(event) = event().await? else {
                return Ok(false);
            };
            svc.metrics.run_paced(&c.owner, &c.key, event).await;
            Ok(true)
        }
        Paced::Entity(owner, spec) => {
            let Some(event) = event().await? else {
                return Ok(false);
            };
            let root = svc.layout.project_dir.clone();
            let ctx = crate::collector_runner::Collectors::of(svc, &root);
            if let crate::collector_runner::EventRun::Ran(Err(error)) =
                crate::collector_runner::run_for_event(&ctx, &owner, &spec.id, event).await?
            {
                tracing::warn!(collector = %format!("{owner}/{}", spec.id), %error, "paced collector failed");
            }
            Ok(true)
        }
    }
}

/// Check the pending runs every few seconds (boot).
pub fn spawn(state: std::sync::Arc<crate::Services>) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = run_due(&state, oxplow_domain::Timestamp::now()).await {
                tracing::warn!(error = %e, "running paced collectors failed");
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pacing(settle: Option<u32>, at_most: Option<u32>, idle: Option<u32>) -> Pacing {
        Pacing {
            settle_secs: settle,
            at_most_secs: at_most,
            idle_secs: idle,
            force: Vec::new(),
        }
    }

    const S: fn(u64) -> Duration = Duration::from_secs;

    #[test]
    fn settle_waits_for_quiet() {
        let p = pacing(Some(30), None, None);
        assert!(!due(&p, S(10), None, None));
        assert!(due(&p, S(30), None, None));
    }

    #[test]
    fn at_most_spaces_runs_and_a_first_run_goes() {
        let p = pacing(None, Some(300), None);
        assert!(due(&p, S(0), None, None), "never ran: goes");
        assert!(!due(&p, S(0), Some(S(60)), None));
        assert!(due(&p, S(0), Some(S(300)), None));
    }

    #[test]
    fn idle_waits_for_no_turn_and_quiet() {
        let p = pacing(None, None, Some(120));
        assert!(!due(&p, S(999), None, None), "a turn is running");
        assert!(!due(&p, S(999), None, Some(S(60))));
        assert!(due(&p, S(999), None, Some(S(120))));
    }

    #[test]
    fn every_condition_must_hold() {
        let p = pacing(Some(30), Some(300), Some(120));
        assert!(
            !due(&p, S(30), Some(S(10)), Some(S(500))),
            "too soon after the last run"
        );
        assert!(!due(&p, S(10), Some(S(400)), Some(S(500))), "not settled");
        assert!(due(&p, S(30), Some(S(400)), Some(S(500))));
    }
}
