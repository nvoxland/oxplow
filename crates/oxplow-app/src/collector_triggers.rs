//! The `collector.triggers` pump consumer (P7.B3): runs each enabled
//! collector whose `trigger: { on: [...] }` names a logged event.
//!
//! One async consumer for every `on:` collector. For each event it runs
//! the collectors whose types include the event's and whose `where`
//! matches its payload, serially, each through
//! [`collector_runner::run_for_event`](crate::collector_runner::run_for_event):
//! the run's `input` binds the event's anchors, its script gets the event
//! as `input.event`, and its rows, `collector_run` row (the event as
//! `last_event_id`) and `collector.synced@1` (caused by the event, deduped
//! per event) commit together — so a redelivered event writes nothing.
//! A collector that fails is recorded and announced like any run; it
//! doesn't dead-letter the event, so one broken collector never holds up
//! the others. An exec collector nobody approved is recorded as
//! `needs_approval` and doesn't run.
//!
//! `after()` is the union of the collectors' `after:` lists, limited to
//! consumers the pump has: an event reaches the collectors once each has
//! handled it.
//!
//! It holds `Services` weakly: the pump that runs it is part of it.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use oxplow_config::collectors::{CollectorSpec, Trigger};
use oxplow_domain::events::schema::EventType;
use oxplow_domain::{DomainError, StoredEvent};

use crate::collector_runner::{self, Collectors, EventRun};
use crate::event_pump::AsyncEventConsumer;
use crate::Services;

/// The consumer's name: its checkpoint and dead letters.
pub const NAME: &str = "collector.triggers";

pub struct CollectorTriggers {
    services: Weak<Services>,
}

impl CollectorTriggers {
    pub fn new(services: Weak<Services>) -> Self {
        Self { services }
    }
}

/// Register the consumer on `svc`'s pump (boot, before it spawns).
pub fn register(svc: &Arc<Services>) {
    svc.event_pump
        .register_async(Arc::new(CollectorTriggers::new(Arc::downgrade(svc))));
}

/// Whether `spec` runs for `event`: its `on:` names the event's type and
/// its `where` matches the payload.
pub fn triggered_by(spec: &CollectorSpec, event: &StoredEvent) -> bool {
    matches!(&spec.trigger, Trigger::On { events, .. } if events.contains(&event.envelope.event_type))
        && matches_where(&spec.trigger, event)
}

/// Whether each `where` field of `event`'s payload equals its value (an
/// `on:` trigger without one matches every event of its types).
pub fn matches_where(trigger: &Trigger, event: &StoredEvent) -> bool {
    let Trigger::On { filter, .. } = trigger else {
        return false;
    };
    filter
        .iter()
        .all(|(field, want)| match event.envelope.payload.get(field) {
            Some(serde_json::Value::String(s)) => s == want,
            Some(serde_json::Value::Bool(b)) => want.parse::<bool>().ok() == Some(*b),
            Some(serde_json::Value::Number(n)) => {
                want.parse::<serde_json::Number>().ok().as_ref() == Some(n)
            }
            _ => false,
        })
}

/// Hand `event` to the fact engine, which runs the fact collectors it
/// triggers over one corpus: a snapshot's files (its whole tree for a
/// whole-tree collector, which alone runs on a take that recorded none),
/// an effort's end snapshot, or else the stream's latest snapshot.
async fn run_fact_collectors(svc: &Services, event: Arc<StoredEvent>) -> Result<(), DomainError> {
    let anchors = &event.envelope.anchors;
    match event.envelope.event_type.as_str() {
        "snapshot.taken" => {
            if let (Some(stream), Some(snapshot)) = (anchors.stream_id, anchors.snapshot_id) {
                svc.metrics
                    .run_snapshot_collectors(stream, snapshot, false, Some(event.clone()))
                    .await;
            }
        }
        "effort.finished" => {
            let effort = event.envelope.payload["effort"]
                .as_str()
                .and_then(|r| r.strip_prefix("effort:"))
                .and_then(oxplow_domain::EffortId::try_from_str)
                .ok_or_else(|| {
                    DomainError::Invalid(format!("effort.finished seq {}: no effort", event.seq))
                })?;
            if let Some(thread) = anchors.thread_id {
                svc.metrics
                    .run_effort_collectors(&thread, &effort, Some(event.clone()))
                    .await;
            }
        }
        _ => svc.metrics.run_event_collectors(event.clone()).await,
    }
    Ok(())
}

impl CollectorTriggers {
    /// The enabled extensions' `on:` collectors that write entities, with
    /// their owners (collected data is project-wide, so the primary
    /// worktree's). Fact collectors come from the fact engine.
    fn entity_collectors(svc: &Services) -> Vec<(String, CollectorSpec)> {
        svc.extension_catalog
            .get(&svc.layout.project_dir)
            .iter()
            .flat_map(|ext| {
                ext.collectors
                    .iter()
                    .filter(|c| c.facts.is_empty() && matches!(c.trigger, Trigger::On { .. }))
                    .map(|c| (ext.name.clone(), c.clone()))
            })
            .collect()
    }

    /// Every `on:` collector's trigger and `after:`, fact and entity alike.
    fn triggers(svc: &Services) -> Vec<(String, Trigger, Vec<String>)> {
        let entities = Self::entity_collectors(svc)
            .into_iter()
            .map(|(owner, s)| (format!("{owner}/{}", s.id), s.trigger, s.after));
        let facts = svc
            .metrics
            .fact_collectors()
            .into_iter()
            .filter(|c| matches!(c.trigger, Trigger::On { .. }))
            .map(|c| (format!("{}/{}", c.owner, c.key), c.trigger, c.after));
        entities.chain(facts).collect()
    }
}

#[async_trait]
impl AsyncEventConsumer for CollectorTriggers {
    fn name(&self) -> &'static str {
        NAME
    }

    fn after(&self) -> Vec<String> {
        let Some(svc) = self.services.upgrade() else {
            return Vec::new();
        };
        let known = svc.event_pump.consumer_names();
        let mut after: Vec<String> = Vec::new();
        for (collector, _, names) in Self::triggers(&svc) {
            for name in names {
                if !known.contains(&name.as_str()) {
                    tracing::warn!(%collector, consumer = %name, "`after` names no consumer; ignored");
                } else if !after.contains(&name) {
                    after.push(name);
                }
            }
        }
        after
    }

    fn handles(&self, event_type: &str) -> bool {
        // A collector's own run never triggers one (a loop otherwise).
        if event_type == oxplow_domain::events::schema::CollectorSynced::TYPE {
            return false;
        }
        let Some(svc) = self.services.upgrade() else {
            return false;
        };
        Self::triggers(&svc).iter().any(|(_, trigger, _)| {
            matches!(trigger, Trigger::On { events, .. } if events.iter().any(|t| t == event_type))
        })
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        // Shutting down: the checkpoint stays, a restart delivers it.
        let Some(svc) = self.services.upgrade() else {
            return Err(DomainError::Busy("services are shutting down".into()));
        };
        let event = Arc::new(event.clone());
        let event_type = event.envelope.event_type.as_str();
        if svc
            .metrics
            .fact_collectors()
            .iter()
            .any(|c| c.runs_on(event_type) && matches_where(&c.trigger, &event))
        {
            run_fact_collectors(&svc, event.clone()).await?;
        }
        let root = svc.layout.project_dir.clone();
        let ctx = Collectors::of(&svc, &root);
        for (owner, spec) in Self::entity_collectors(&svc) {
            if !triggered_by(&spec, &event) {
                continue;
            }
            match collector_runner::run_for_event(&ctx, &owner, &spec.id, event.clone()).await? {
                EventRun::Ran(Err(error)) => {
                    tracing::warn!(collector = %format!("{owner}/{}", spec.id), %error, "collector failed");
                }
                EventRun::Ran(Ok(_)) | EventRun::Skipped | EventRun::NeedsApproval => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::events::Anchors;
    use oxplow_domain::Envelope;
    use serde_json::json;

    const HEAD: &str =
        "manifest: 2\nname: work\nsharing: private\nintent: { purpose: t, origin: null, examples: [] }\n";

    fn extension(root: &std::path::Path, collectors: &str, files: &[(&str, &str)]) {
        let dir = root.join("oxplow/extensions/work");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("extension.yaml"),
            format!("{HEAD}collectors:\n{collectors}"),
        )
        .unwrap();
        for (name, body) in files {
            std::fs::write(dir.join(name), body).unwrap();
        }
    }

    const SEEN: &str = "def transform(input):\n    return {\"entities\": {\"seen\": [{\"ev\": r[\"ev\"], \"effort\": r[\"effort\"], \"kind\": input[\"event\"][\"type\"]} for r in input[\"rows\"]]}}\n";

    fn on_claims(filter: &str) -> String {
        format!(
            "  - id: seen\n    runtime: starlark\n    entry: seen.star\n    trigger: {{ on: [effort.claim_verified]{filter} }}\n    input: \"SELECT :effort_id AS effort, :event_id AS ev\"\n    sync: upsert\n    entities:\n      - {{ name: seen, key: ev, columns: {{ ev: int, effort: int, kind: text }} }}\n"
        )
    }

    async fn log(svc: &Services, effort: oxplow_domain::EffortId, evidence: &str) -> StoredEvent {
        let env = Envelope::typed::<oxplow_domain::events::schema::EffortClaimVerified>(
            "human",
            &oxplow_domain::events::schema::EffortClaimVerifiedV1 {
                claim: "claim:1".into(),
                effort: None,
                evidence: Some(evidence.into()),
            },
        )
        .with_anchors(Anchors {
            effort_id: Some(effort),
            ..Anchors::default()
        });
        let id = env.id.clone();
        svc.event_log_store.append(env).await.unwrap();
        svc.event_log_store.get(id).await.unwrap().unwrap()
    }

    async fn rows(svc: &Services, sql: &str) -> serde_json::Value {
        let out = svc.sql.query_sql(sql, vec![], None).await.unwrap();
        serde_json::to_value(out.rows).unwrap()
    }

    /// An `on:` collector runs for the event with its anchors bound and the
    /// event in its input, logs one `collector.synced` caused by it, and a
    /// redelivered event writes nothing.
    #[tokio::test]
    async fn an_on_collector_runs_once_per_event_with_its_anchors() {
        let fx = crate::test_fixtures::services_with_effort().await;
        extension(
            &fx.svc.layout.project_dir,
            &on_claims(""),
            &[("seen.star", SEEN)],
        );
        let consumer = CollectorTriggers::new(Arc::downgrade(&fx.svc));
        let ev = log(&fx.svc, fx.effort, "reviewer").await;
        assert!(consumer.handles(&ev.envelope.event_type));
        consumer.handle(&ev).await.unwrap();
        consumer.handle(&ev).await.unwrap();
        assert_eq!(
            rows(&fx.svc, "SELECT ev, effort, kind FROM v_work_seen").await,
            json!([[ev.seq, fx.effort.value(), "effort.claim_verified"]])
        );
        assert_eq!(
            rows(
                &fx.svc,
                "SELECT json_extract(payload, '$.trigger'), json_extract(payload, '$.status'), cause FROM v_event WHERE type = 'collector.synced'"
            )
            .await,
            json!([["on", "ok", ev.envelope.id.to_string()]])
        );
        assert_eq!(
            rows(&fx.svc, "SELECT last_event_id FROM v_collector_run").await,
            json!([[ev.seq]])
        );
    }

    /// A `where` filter skips an event whose payload doesn't match.
    #[tokio::test]
    async fn a_where_filter_skips() {
        let fx = crate::test_fixtures::services_with_effort().await;
        extension(
            &fx.svc.layout.project_dir,
            &on_claims(", where: { evidence: reviewer }"),
            &[("seen.star", SEEN)],
        );
        let consumer = CollectorTriggers::new(Arc::downgrade(&fx.svc));
        let other = log(&fx.svc, fx.effort, "run:3").await;
        consumer.handle(&other).await.unwrap();
        assert_eq!(
            rows(&fx.svc, "SELECT count(*) FROM v_collector_run").await,
            json!([[0]])
        );
        let mine = log(&fx.svc, fx.effort, "reviewer").await;
        consumer.handle(&mine).await.unwrap();
        assert_eq!(
            rows(&fx.svc, "SELECT ev FROM v_work_seen").await,
            json!([[mine.seq]])
        );
    }

    /// A failing collector is recorded and announced, not dead-lettered:
    /// the handler succeeds and the other collectors still run.
    #[tokio::test]
    async fn a_failing_collector_is_recorded_not_dead_lettered() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let broken = on_claims("")
            .replace("id: seen", "id: broken")
            .replace("entry: seen.star", "entry: broken.star")
            .replace("name: seen", "name: broken");
        extension(
            &fx.svc.layout.project_dir,
            &format!("{broken}{}", on_claims("")),
            &[
                ("seen.star", SEEN),
                ("broken.star", "def transform(input):\n    return 1 // 0\n"),
            ],
        );
        let consumer = CollectorTriggers::new(Arc::downgrade(&fx.svc));
        let ev = log(&fx.svc, fx.effort, "reviewer").await;
        consumer.handle(&ev).await.unwrap();
        assert_eq!(
            rows(
                &fx.svc,
                "SELECT id, status FROM v_collector_run ORDER BY id"
            )
            .await,
            json!([["broken", "error"], ["seen", "ok"]])
        );
        assert_eq!(
            rows(&fx.svc, "SELECT json_extract(payload, '$.status') FROM v_event WHERE type = 'collector.synced' ORDER BY seq").await,
            json!([["error"], ["ok"]])
        );
    }

    /// An exec collector on an event runs only once a person approved it;
    /// until then each event records `needs_approval` and runs nothing.
    #[tokio::test]
    async fn an_unapproved_exec_collector_records_needs_approval() {
        let fx = crate::test_fixtures::services_with_effort().await;
        extension(
            &fx.svc.layout.project_dir,
            "  - id: pull\n    runtime: exec\n    entry: pull.sh\n    trigger: { on: [effort.claim_verified] }\n    entities:\n      - { name: pr, key: n, columns: { n: int } }\n",
            &[("pull.sh", "#!/bin/sh\necho '{\"entities\":{\"pr\":[{\"n\":1}]}}'\n")],
        );
        let consumer = CollectorTriggers::new(Arc::downgrade(&fx.svc));
        let ev = log(&fx.svc, fx.effort, "reviewer").await;
        consumer.handle(&ev).await.unwrap();
        assert_eq!(
            rows(&fx.svc, "SELECT status, last_event_id FROM v_collector_run").await,
            json!([["needs_approval", ev.seq]])
        );
        assert_eq!(
            rows(
                &fx.svc,
                "SELECT count(*) FROM v_event WHERE type = 'collector.synced'"
            )
            .await,
            json!([[0]])
        );
    }

    /// `after()` is the collectors' `after:` lists, limited to consumers
    /// the pump has; a collector's own type never triggers one.
    #[tokio::test]
    async fn after_and_handles_follow_the_declarations() {
        let fx = crate::test_fixtures::services_with_effort().await;
        crate::effort_reactors::register(&fx.svc);
        extension(
            &fx.svc.layout.project_dir,
            &on_claims("").replace(
                "    input:",
                "    after: [effort.evidence, no.such_consumer]\n    input:",
            ),
            &[("seen.star", SEEN)],
        );
        let consumer = CollectorTriggers::new(Arc::downgrade(&fx.svc));
        assert_eq!(consumer.after(), vec!["effort.evidence".to_string()]);
        assert!(consumer.handles("effort.claim_verified"));
        assert!(!consumer.handles("snapshot.taken"));
        assert!(!consumer.handles("collector.synced"));
    }
}
