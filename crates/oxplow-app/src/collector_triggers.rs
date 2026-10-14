//! The `collector.triggers` pump consumer (P7.B3): runs each enabled
//! collector whose `trigger: { on: [...] }` names a logged event.
//!
//! One async consumer for every `on:` collector. For each event it runs
//! the collectors whose types include the event's and whose `where`
//! matches its payload, serially, each through
//! [`collector_runner::run_for_event`](crate::collector_runner::run_for_event):
//! the run's `input` binds the event's anchors, its script gets the event
//! as `input.event`, and its rows (an entity collector's) or capture (a fact
//! collector's), `collector_run` row (the event as `last_event_id`) and
//! `collector.synced@1` (caused by the event, deduped per event) commit
//! together — so a redelivered event writes nothing.
//! A collector that fails is recorded and announced like any run; it
//! doesn't dead-letter the event, so one broken collector never holds up
//! the others. An exec collector nobody approved is recorded as
//! `needs_approval` and doesn't run.
//!
//! `after_for(event)` is the union of the `after:` lists of the
//! collectors that event triggers (limited to consumers the pump has): an
//! event reaches the collectors once each of those has handled it, so a
//! collector's `after:` holds up only its own events (tsk711). `after()`, the
//! union over every collector, orders the consumers when the pump settles.
//!
//! It holds `Services` weakly: the pump that runs it is part of it.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use oxplow_config::collectors::{CollectorSpec, Trigger};
use oxplow_domain::events::schema::EventType;
use oxplow_domain::{DomainError, StoredEvent};

use crate::collector_runner::{self, Collectors, EventRun};
use crate::event_pump::AsyncEventConsumer;
use crate::extensions::descriptors::{gates, placement};
use crate::Services;
use oxplow_domain::contribution::ContributionKind;

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
    payload_matches(filter, &event.envelope.payload)
}

/// Whether `payload` has each `filter` field equal to its value. The value
/// is the YAML text: a string matches exactly, a boolean by `true`/`false`, a
/// number by value (`1` matches `1.0`). A field the payload lacks, or holds
/// as an object, list or null, never matches.
pub fn payload_matches(
    filter: &std::collections::BTreeMap<String, String>,
    payload: &serde_json::Value,
) -> bool {
    filter.iter().all(|(field, want)| match payload.get(field) {
        Some(serde_json::Value::String(s)) => s == want,
        Some(serde_json::Value::Bool(b)) => want.parse::<bool>().ok() == Some(*b),
        Some(serde_json::Value::Number(n)) => {
            matches!((n.as_f64(), want.parse::<f64>()), (Some(a), Ok(b)) if a == b)
        }
        _ => false,
    })
}

/// Hand `event` to the fact engine, which runs the fact collectors it
/// triggers over one corpus: a snapshot's files (its whole tree for a
/// whole-tree collector, which alone runs on a take that recorded none),
/// an effort's end snapshot, or else the stream's latest snapshot.
/// Only those `select` picks: one level of the compiled order, of the ones
/// that run at once (a collector whose pacing defers this event's type
/// waits for `crate::pacing`).
async fn run_fact_collectors(
    svc: &Services,
    event: Arc<StoredEvent>,
    select: &(dyn Fn(&crate::metrics_service::FactCollector) -> bool + Sync),
) -> Result<(), DomainError> {
    let anchors = &event.envelope.anchors;
    let event_type = event.envelope.event_type.clone();
    let now = |c: &crate::metrics_service::FactCollector| !c.defers(&event_type) && select(c);
    match event_type.as_str() {
        "snapshot.taken" => {
            if let (Some(stream), Some(snapshot)) = (anchors.stream_id, anchors.snapshot_id) {
                svc.metrics
                    .run_snapshot_collectors(
                        stream,
                        snapshot,
                        false,
                        Some(event.clone()),
                        &now,
                        None,
                    )
                    .await;
            }
        }
        "effort.finished" => {
            let effort = crate::effort_lifecycle::effort_of(&event)?;
            if let Some(thread) = anchors.thread_id {
                svc.metrics
                    .run_effort_collectors(&thread, &effort, Some(event.clone()), &now)
                    .await;
            }
        }
        _ => svc.metrics.run_event_collectors(event.clone(), &now).await,
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

    /// Every `on:` collector's trigger, level and what it waits on, fact
    /// and entity alike, by `(owner, id)`.
    fn triggers(svc: &Services) -> Vec<Triggered> {
        let load = svc.extension_catalog.get(&svc.layout.project_dir);
        let place = |owner: &str, id: &str| {
            load.iter()
                .find(|e| e.name == owner)
                .map(|e| placement(e, ContributionKind::Collector, id))
                .unwrap_or_default()
        };
        let entities = Self::entity_collectors(svc).into_iter().map(|(owner, s)| {
            let (level, after) = place(&owner, &s.id);
            Triggered {
                owner,
                id: s.id,
                trigger: s.trigger,
                level,
                after,
            }
        });
        let facts = svc
            .metrics
            .fact_collectors()
            .into_iter()
            .filter(|c| matches!(c.trigger, Trigger::On { .. }))
            .map(|c| {
                let (level, after) = place(&c.owner, &c.key);
                Triggered {
                    owner: c.owner,
                    id: c.key,
                    trigger: c.trigger,
                    level,
                    after,
                }
            });
        entities.chain(facts).collect()
    }

    /// The consumers the collectors `event_type` triggers wait on (all of
    /// them for `None`): the core consumers their `after:` names, and
    /// `effect.triggers` when one runs after an effect.
    fn predecessors(&self, event_type: Option<&str>) -> Vec<String> {
        let Some(svc) = self.services.upgrade() else {
            return Vec::new();
        };
        // Load checked every name against core's consumers; one this pump
        // doesn't run (a test's partial pump) has nothing to wait for.
        let running = svc.event_pump.consumer_names();
        let mut out: Vec<String> = Vec::new();
        for t in Self::triggers(&svc) {
            let triggered = match (event_type, &t.trigger) {
                (None, _) => true,
                (Some(e), Trigger::On { events, .. }) => events.iter().any(|x| x == e),
                (Some(_), _) => false,
            };
            if !triggered {
                continue;
            }
            for gate in gates(ContributionKind::Collector, &t.after) {
                if running.contains(&gate.as_str()) && !out.contains(&gate) {
                    out.push(gate);
                }
            }
        }
        out
    }
}

/// An `on:` collector as the consumer schedules it.
struct Triggered {
    owner: String,
    id: String,
    trigger: Trigger,
    /// Its compiled level: it runs after every collector of a lower one.
    level: u32,
    after: Vec<oxplow_domain::ordering::After>,
}

#[async_trait]
impl AsyncEventConsumer for CollectorTriggers {
    fn name(&self) -> &'static str {
        NAME
    }

    fn after(&self) -> Vec<String> {
        self.predecessors(None)
    }

    /// Only the collectors `event_type` triggers wait on their `after:`
    /// (tsk711): a `snapshot.taken` collector doesn't wait on the
    /// `change.analyze` an `effort.finished` collector names.
    fn after_for(&self, event_type: &str) -> Vec<String> {
        self.predecessors(Some(event_type))
    }

    fn handles(&self, event_type: &str) -> bool {
        // A collector's own run never triggers one (a loop otherwise).
        if event_type == oxplow_domain::events::schema::CollectorSynced::TYPE {
            return false;
        }
        let Some(svc) = self.services.upgrade() else {
            return false;
        };
        Self::triggers(&svc).iter().any(|t| {
            matches!(&t.trigger, Trigger::On { events, .. } if events.iter().any(|e| e == event_type))
        })
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        // Shutting down: the checkpoint stays, a restart delivers it.
        let Some(svc) = self.services.upgrade() else {
            return Err(DomainError::Busy("services are shutting down".into()));
        };
        svc.sql
            .read_generation(async { self.handle_pinned_impl(&svc, event).await })
            .await?
    }
}

impl CollectorTriggers {
    async fn handle_pinned_impl(
        &self,
        svc: &Arc<Services>,
        event: &StoredEvent,
    ) -> Result<(), DomainError> {
        let event = Arc::new(event.clone());
        let event_type = event.envelope.event_type.as_str();
        let now = oxplow_domain::Timestamp::now().to_text();
        let level_of: std::collections::BTreeMap<(String, String), u32> = Self::triggers(svc)
            .into_iter()
            .map(|t| ((t.owner, t.id), t.level))
            .collect();
        let level = |owner: &str, id: &str| {
            level_of
                .get(&(owner.to_string(), id.to_string()))
                .copied()
                .unwrap_or(0)
        };
        let facts: Vec<_> = svc
            .metrics
            .fact_collectors()
            .into_iter()
            .filter(|c| c.runs_on(event_type) && matches_where(&c.trigger, &event))
            .collect();
        // The paced ones wait, recorded pending (`crate::pacing`).
        for c in facts.iter().filter(|c| c.defers(event_type)) {
            svc.collector_store
                .mark_pending(&c.owner, &c.key, event.seq, &now)
                .await?;
        }
        let mut entities = Vec::new();
        for (owner, spec) in Self::entity_collectors(svc) {
            if !triggered_by(&spec, &event) {
                continue;
            }
            if matches!(&spec.trigger, Trigger::On { pacing, .. } if pacing.defers(event_type)) {
                svc.collector_store
                    .mark_pending(&owner, &spec.id, event.seq, &now)
                    .await?;
                continue;
            }
            entities.push((owner, spec));
        }
        // Level by level, each after every collector it names: a level's
        // fact collectors over one corpus, then its entity collectors.
        let mut levels: Vec<u32> = facts
            .iter()
            .filter(|c| !c.defers(event_type))
            .map(|c| level(&c.owner, &c.key))
            .chain(entities.iter().map(|(o, s)| level(o, &s.id)))
            .collect();
        levels.sort();
        levels.dedup();
        let root = svc.layout.project_dir.clone();
        let ctx = Collectors::of(svc, &root);
        for at in levels {
            if facts
                .iter()
                .any(|c| !c.defers(event_type) && level(&c.owner, &c.key) == at)
            {
                run_fact_collectors(svc, event.clone(), &|c| level(&c.owner, &c.key) == at).await?;
            }
            let mut here: Vec<&(String, CollectorSpec)> = entities
                .iter()
                .filter(|(o, s)| level(o, &s.id) == at)
                .collect();
            here.sort_by(|a, b| (&a.0, &a.1.id).cmp(&(&b.0, &b.1.id)));
            for (owner, spec) in here {
                match collector_runner::run_for_event(&ctx, owner, &spec.id, event.clone()).await? {
                    EventRun::Ran(Err(error)) => {
                        tracing::warn!(collector = %format!("{owner}/{}", spec.id), %error, "collector failed");
                    }
                    EventRun::Guarded => {
                        tracing::warn!(collector = %format!("{owner}/{}", spec.id), "collector skipped by the loop guard");
                    }
                    EventRun::Ran(Ok(_)) | EventRun::Skipped | EventRun::NeedsApproval => {}
                }
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

    async fn extension(svc: &std::sync::Arc<Services>, collectors: &str, files: &[(&str, &str)]) {
        let dir = svc.layout.project_dir.join("oxplow/extensions/work");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("extension.yaml"),
            format!("{HEAD}collectors:\n{collectors}"),
        )
        .unwrap();
        for (name, body) in files {
            std::fs::write(dir.join(name), body).unwrap();
        }
        crate::test_fixtures::activate_with_problems(svc).await;
    }

    const SEEN: &str = "def transform(input):\n    return {\"entities\": {\"seen\": [{\"ev\": r[\"ev\"], \"id\": r[\"id\"], \"effort\": r[\"effort\"], \"kind\": input[\"event\"][\"type\"]} for r in input[\"rows\"]]}}\n";

    fn on_claims(filter: &str) -> String {
        format!(
            "  - id: seen\n    runtime: starlark\n    entry: seen.star\n    trigger: {{ on: [effort.claim_verified]{filter} }}\n    input: \"SELECT :effort_id AS effort, :event_seq AS ev, :event_id AS id\"\n    sync: upsert\n    entities:\n      - {{ name: seen, key: ev, columns: {{ ev: int, id: text, effort: int, kind: text }} }}\n"
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

    /// A `where` value names the payload's field as written in YAML (always
    /// a string); a number matches by value whatever its JSON spelling, and a
    /// field the payload lacks never matches.
    #[test]
    fn a_where_filter_compares_by_value_and_a_missing_field_never_matches() {
        let filter = |pairs: &[(&str, &str)]| -> std::collections::BTreeMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        let payload = json!({ "file_count": 1.0, "unchanged": false, "trigger": "git_refs" });
        assert!(payload_matches(&filter(&[("file_count", "1")]), &payload));
        assert!(payload_matches(&filter(&[("file_count", "1.0")]), &payload));
        assert!(!payload_matches(&filter(&[("file_count", "2")]), &payload));
        assert!(payload_matches(
            &filter(&[("unchanged", "false")]),
            &payload
        ));
        assert!(payload_matches(
            &filter(&[("trigger", "git_refs")]),
            &payload
        ));
        assert!(!payload_matches(&filter(&[("missing", "x")]), &payload));
        assert!(
            payload_matches(&filter(&[]), &payload),
            "no filter matches every event"
        );
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
        extension(&fx.svc, &on_claims(""), &[("seen.star", SEEN)]).await;
        let consumer = CollectorTriggers::new(Arc::downgrade(&fx.svc));
        let ev = log(&fx.svc, fx.effort, "reviewer").await;
        assert!(consumer.handles(&ev.envelope.event_type));
        consumer.handle(&ev).await.unwrap();
        consumer.handle(&ev).await.unwrap();
        // tsk1002: `:event_id` is the event's id and `:event_seq` its seq,
        // as an effect's `input` binds them.
        assert_eq!(
            rows(&fx.svc, "SELECT ev, id, effort, kind FROM v_work_seen").await,
            json!([[
                ev.seq,
                ev.envelope.id.to_string(),
                fx.effort.value(),
                "effort.claim_verified"
            ]])
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

    /// A paced collector doesn't run as its event arrives: it's
    /// recorded pending (the latest event wins) and runs once its pacing
    /// allows, for that event, then clears.
    #[tokio::test]
    async fn a_settled_collector_waits_then_runs_once_for_the_latest_event() {
        let fx = crate::test_fixtures::services_with_effort().await;
        extension(&fx.svc, &on_claims(", settle: 30s"), &[("seen.star", SEEN)]).await;
        let consumer = CollectorTriggers::new(Arc::downgrade(&fx.svc));
        let first = log(&fx.svc, fx.effort, "reviewer").await;
        consumer.handle(&first).await.unwrap();
        let latest = log(&fx.svc, fx.effort, "reviewer").await;
        consumer.handle(&latest).await.unwrap();
        assert_eq!(
            rows(&fx.svc, "SELECT count(*) FROM v_collector_run").await,
            json!([[0]]),
            "not run yet"
        );
        assert_eq!(
            rows(&fx.svc, "SELECT owner, id, event_seq FROM v_pending_run").await,
            json!([["work", "seen", latest.seq]])
        );
        let now = oxplow_domain::Timestamp::now();
        assert_eq!(
            crate::pacing::run_due(&fx.svc, now).await.unwrap(),
            0,
            "not settled"
        );
        let later = oxplow_domain::Timestamp::from_unix_ms(now.unix_ms() + 31_000);
        assert_eq!(crate::pacing::run_due(&fx.svc, later).await.unwrap(), 1);
        assert_eq!(
            rows(&fx.svc, "SELECT ev FROM v_work_seen").await,
            json!([[latest.seq]]),
            "once, for the latest event"
        );
        assert_eq!(
            rows(&fx.svc, "SELECT count(*) FROM v_pending_run").await,
            json!([[0]])
        );
    }

    /// An `idle` collector waits while an agent turn runs, and
    /// runs once none does and nothing has changed for that long.
    #[tokio::test]
    async fn an_idle_collector_waits_out_a_running_turn() {
        let fx = crate::test_fixtures::services_with_effort().await;
        extension(&fx.svc, &on_claims(", idle: 2m"), &[("seen.star", SEEN)]).await;
        let consumer = CollectorTriggers::new(Arc::downgrade(&fx.svc));
        consumer
            .handle(&log(&fx.svc, fx.effort, "reviewer").await)
            .await
            .unwrap();
        let thread = fx.thread.value();
        fx.svc
            .db
            .transaction(move |tx| {
                tx.execute(
                    "INSERT INTO agent_turn (thread_id, prompt, started_at) VALUES (?1, 'p', '2026-01-01T00:00:00.000000Z')",
                    [thread],
                )
                .map_err(oxplow_db::map_sql_err)
                .map(|_| ())
            })
            .await
            .unwrap();
        let later = oxplow_domain::Timestamp::from_unix_ms(
            oxplow_domain::Timestamp::now().unix_ms() + 600_000,
        );
        assert_eq!(
            crate::pacing::run_due(&fx.svc, later).await.unwrap(),
            0,
            "a turn runs"
        );
        fx.svc
            .db
            .transaction(|tx| {
                tx.execute(
                    "UPDATE agent_turn SET ended_at = '2026-01-01T00:00:01.000000Z'",
                    [],
                )
                .map_err(oxplow_db::map_sql_err)
                .map(|_| ())
            })
            .await
            .unwrap();
        assert_eq!(crate::pacing::run_due(&fx.svc, later).await.unwrap(), 1);
    }

    /// A `where` filter skips an event whose payload doesn't match.
    #[tokio::test]
    async fn a_where_filter_skips() {
        let fx = crate::test_fixtures::services_with_effort().await;
        extension(
            &fx.svc,
            &on_claims(", where: { evidence: reviewer }"),
            &[("seen.star", SEEN)],
        )
        .await;
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
            &fx.svc,
            &format!("{broken}{}", on_claims("")),
            &[
                ("seen.star", SEEN),
                ("broken.star", "def transform(input):\n    return 1 // 0\n"),
            ],
        )
        .await;
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
        extension(&fx.svc,
            "  - id: pull\n    runtime: exec\n    entry: pull.sh\n    trigger: { on: [effort.claim_verified] }\n    entities:\n      - { name: pr, key: n, columns: { n: int } }\n",
            &[("pull.sh", "#!/bin/sh\necho '{\"entities\":{\"pr\":[{\"n\":1}]}}'\n")],
        ).await;
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

    /// `after()` is the core consumers the collectors' `after:` lists
    /// name; a collector's own type never triggers one.
    #[tokio::test]
    async fn after_and_handles_follow_the_declarations() {
        let fx = crate::test_fixtures::services_with_effort().await;
        crate::effort_reactors::register(&fx.svc);
        extension(
            &fx.svc,
            &on_claims("").replace("    input:", "    after: [effort.evidence]\n    input:"),
            &[("seen.star", SEEN)],
        )
        .await;
        let consumer = CollectorTriggers::new(Arc::downgrade(&fx.svc));
        assert_eq!(
            consumer.after_for("effort.claim_verified"),
            vec!["effort.evidence".to_string()]
        );
        // tsk711: predecessors follow the event's own collectors — an event
        // no collector waiting on `effort.evidence` handles doesn't wait.
        assert_eq!(consumer.after_for("snapshot.taken"), Vec::<String>::new());
        assert_eq!(
            consumer.after(),
            vec!["effort.evidence".to_string()],
            "every predecessor, for ordering"
        );
        assert!(consumer.handles("effort.claim_verified"));
        // The default-on code gauges handle `snapshot.taken` (tsk1034); an
        // event no collector names isn't handled.
        assert!(consumer.handles("snapshot.taken"));
        assert!(!consumer.handles("work_item.created"));
        assert!(!consumer.handles("collector.synced"));
    }

    /// On one event, entity collectors run in their compiled order: each
    /// after the collectors its `after:` names.
    #[tokio::test]
    async fn collectors_run_after_the_collectors_they_name() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let one = |id: &str, after: &str| {
            on_claims("")
                .replace("  - id: seen\n", &format!("  - id: {id}\n"))
                .replace("name: seen,", &format!("name: {id},"))
                .replace("entry: seen.star", &format!("entry: {id}.star"))
                .replace("    input:", &format!("    after: [{after}]\n    input:"))
        };
        extension(
            &fx.svc,
            &format!("{}{}", one("later", "sooner"), one("sooner", "")),
            &[
                ("later.star", &SEEN.replace("\"seen\"", "\"later\"")),
                ("sooner.star", &SEEN.replace("\"seen\"", "\"sooner\"")),
            ],
        )
        .await;
        let ev = log(&fx.svc, fx.effort, "e").await;
        CollectorTriggers::new(Arc::downgrade(&fx.svc))
            .handle(&ev)
            .await
            .unwrap();
        assert_eq!(
            rows(
                &fx.svc,
                "SELECT json_extract(payload, '$.collector') FROM v_event WHERE type = 'collector.synced' AND json_extract(payload, '$.collector') LIKE 'collector:work/%' ORDER BY seq",
            )
            .await,
            json!([["collector:work/sooner"], ["collector:work/later"]])
        );
    }

    /// A collector naming nothing in `after:` isn't loaded: the order
    /// can't be honored.
    #[tokio::test]
    async fn a_collector_after_nothing_is_a_load_error() {
        let fx = crate::test_fixtures::services_with_effort().await;
        extension(
            &fx.svc,
            &on_claims("").replace("    input:", "    after: [no.such_consumer]\n    input:"),
            &[("seen.star", SEEN)],
        )
        .await;
        // Held back from the active catalog; the load says why.
        let load = fx
            .svc
            .extension_catalog
            .snapshot(&fx.svc.layout.project_dir);
        let work = load.extensions().iter().find(|e| e.name == "work").unwrap();
        assert!(work.collectors.is_empty());
        assert!(
            work.errors
                .iter()
                .any(|e| e.contains("`no.such_consumer` is none of")),
            "{:?}",
            work.errors
        );
    }

    /// `acme-pr`: it declares `acme_pr.merged`, `.ping` and `.pong`
    /// (payload `{ number }`), and the given collectors.
    async fn emitter(svc: &std::sync::Arc<Services>, collectors: &str, files: &[(&str, &str)]) {
        let dir = svc.layout.project_dir.join("oxplow/extensions/acme-pr");
        std::fs::create_dir_all(&dir).unwrap();
        let types: String = ["merged", "ping", "pong"]
            .iter()
            .map(|t| {
                format!(
                    "    - {{ type: acme_pr.{t}, v: 1, schema: number.json, summary: A {t}. }}\n"
                )
            })
            .collect();
        std::fs::write(
            dir.join("extension.yaml"),
            format!("manifest: 2\nname: acme-pr\nsharing: private\nintent: {{ purpose: t, origin: null, examples: [] }}\nevent_types:\n  types:\n{types}collectors:\n{collectors}"),
        )
        .unwrap();
        std::fs::write(
            dir.join("number.json"),
            r#"{"type": "object", "required": ["number"], "properties": {"number": {"type": "integer"}}}"#,
        )
        .unwrap();
        for (name, body) in files {
            std::fs::write(dir.join(name), body).unwrap();
        }
        crate::test_fixtures::activate_with_problems(svc).await;
    }

    /// A manual collector `prs` storing one `pr` row and returning `events`.
    fn prs(trigger: &str) -> String {
        format!("  - id: prs\n    runtime: starlark\n    entry: prs.star\n{trigger}    entities:\n      - {{ name: pr, key: n, columns: {{ n: int }} }}\n")
    }

    fn prs_script(events: &str) -> String {
        format!("def transform(input):\n    return {{\"entities\": {{\"pr\": [{{\"n\": 12}}]}}, \"events\": {events}}}\n")
    }

    async fn sync(
        svc: &Services,
        id: &str,
    ) -> Result<collector_runner::CollectorRunReport, String> {
        let root = svc.layout.project_dir.clone();
        collector_runner::run_collector(
            &Collectors::of(svc, &root),
            &oxplow_domain::run::RunContext::core(
                oxplow_domain::Actor::Human,
                oxplow_domain::run::Mode::External,
            ),
            "acme-pr",
            id,
            collector_runner::RunTrigger::Manual,
        )
        .await
        .map_err(|e| match e {
            collector_runner::RunCollectorError::Failed(m) => m,
            other => format!("{other:?}"),
        })
    }

    /// P9.D2: a collector's script may return `events` of its extension's
    /// own declared types. They land with the run — its rows, its
    /// `collector_run`, its `collector.synced` — in one transaction, from
    /// `collector:<owner>/<id>`, caused by that `collector.synced`.
    #[tokio::test]
    async fn a_collector_emits_its_extensions_events_with_its_run() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        emitter(svc,
            &prs(""),
            &[(
                "prs.star",
                &prs_script("[{\"type\": \"acme_pr.merged\", \"payload\": {\"number\": 12}, \"subject\": [\"collector:acme-pr/prs\"]}]"),
            )],
        ).await;
        crate::test_fixtures::activate_with_problems(svc).await;
        sync(svc, "prs").await.unwrap();
        assert_eq!(rows(svc, "SELECT n FROM v_acme_pr_pr").await, json!([[12]]));
        let synced = rows(
            svc,
            "SELECT id FROM v_event WHERE type = 'collector.synced' AND json_extract(payload, '$.status') = 'ok'",
        )
        .await;
        assert_eq!(
            rows(
                svc,
                "SELECT source, cause, json_extract(payload, '$.number'), v FROM v_event WHERE type = 'acme_pr.merged'"
            )
            .await,
            json!([["collector:acme-pr/prs", synced[0][0], 12, 1]])
        );
        // What a preview (and `oxplow extension test`) shows: the events it
        // would emit, stored nowhere.
        let root = svc.layout.project_dir.clone();
        let preview = collector_runner::preview_collector(
            &Collectors::of(svc, &root),
            "acme-pr",
            "prs",
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_value(&preview.events).unwrap(),
            json!([{ "type": "acme_pr.merged", "v": 1, "payload": { "number": 12 }, "subject": ["collector:acme-pr/prs"] }])
        );
        assert_eq!(
            rows(
                svc,
                "SELECT count(*) FROM v_event WHERE type = 'acme_pr.merged'"
            )
            .await,
            json!([[1]])
        );
    }

    /// P9.D2: what a collector may not emit fails the whole run — nothing
    /// is stored, no event is logged, and the run is recorded as an error:
    /// another namespace's type, a type that doesn't validate, a type it
    /// runs on (it would trigger itself), or more than the cap.
    #[tokio::test]
    async fn a_collector_may_not_emit_what_isnt_its_own_or_what_it_runs_on() {
        let many = format!(
            "[{{\"type\": \"acme_pr.merged\", \"payload\": {{\"number\": i}}}} for i in range({})]",
            collector_runner::MAX_RUN_EVENTS + 1
        );
        for (trigger, events, says) in [
            (
                "",
                "[{\"type\": \"work_item.created\", \"payload\": {}}]".to_string(),
                "may emit only the event types `acme-pr` declares",
            ),
            (
                "",
                "[{\"type\": \"other_ext.thing\", \"payload\": {}}]".to_string(),
                "may emit only the event types `acme-pr` declares",
            ),
            (
                "",
                "[{\"type\": \"acme_pr.merged\", \"payload\": {\"number\": \"twelve\"}}]"
                    .to_string(),
                "number",
            ),
            (
                "    trigger: { on: [acme_pr.merged] }\n",
                "[{\"type\": \"acme_pr.merged\", \"payload\": {\"number\": 1}}]".to_string(),
                "a type it runs on",
            ),
            ("", many, "at most"),
        ] {
            // Each on its own project: three failures in a row would
            // disable the collector, which is another rule's business.
            let fx = crate::test_fixtures::services_with_effort().await;
            let svc = &fx.svc;
            emitter(svc, &prs(trigger), &[("prs.star", &prs_script(&events))]).await;
            crate::test_fixtures::activate_with_problems(svc).await;
            let refused = sync(svc, "prs").await.unwrap_err();
            assert!(refused.contains(says), "{says}: {refused}");
            assert_eq!(
                rows(
                    svc,
                    "SELECT count(*) FROM v_event WHERE type LIKE 'acme_pr.%'"
                )
                .await,
                json!([[0]]),
                "{says}"
            );
            assert_eq!(
                rows(svc, "SELECT status FROM v_collector_run WHERE id = 'prs'").await,
                json!([["error"]]),
                "{says}"
            );
            // Nothing was stored.
            assert!(
                svc.sql
                    .query_sql("SELECT n FROM v_acme_pr_pr", vec![], None)
                    .await
                    .map(|r| r.rows.is_empty())
                    .unwrap_or(true),
                "{says}"
            );
        }
    }

    /// P9.D2: two collectors that run on each other's events stop: an
    /// event a collector's own run led to never triggers it again, and a
    /// chain of `MAX_CHAIN` reactions isn't extended. A guarded run is
    /// recorded as skipped, not as a failure.
    #[tokio::test]
    async fn collectors_on_each_others_events_stop_at_the_guard() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let echo = |id: &str, on: &str| {
            format!("  - id: {id}\n    runtime: starlark\n    entry: {id}.star\n    trigger: {{ on: [acme_pr.{on}] }}\n    sync: upsert\n    entities:\n      - {{ name: {id}, key: n, columns: {{ n: int }} }}\n")
        };
        let script = |id: &str| {
            format!("def transform(input):\n    n = input[\"event\"][\"payload\"][\"number\"] + 1\n    return {{\"entities\": {{\"{id}\": [{{\"n\": n}}]}}, \"events\": [{{\"type\": \"acme_pr.{id}\", \"payload\": {{\"number\": n}}}}]}}\n")
        };
        emitter(
            svc,
            &format!("{}{}", echo("ping", "pong"), echo("pong", "ping")),
            &[
                ("ping.star", &script("ping")),
                ("pong.star", &script("pong")),
            ],
        )
        .await;
        crate::test_fixtures::activate_with_problems(svc).await;
        let consumer = CollectorTriggers::new(Arc::downgrade(svc));
        svc.event_log_store
            .append(Envelope::new("acme_pr.ping", 1, "test", json!({ "number": 0 })).unwrap())
            .await
            .unwrap();
        // Deliver every ping and pong, the collectors' own included, until
        // a round logs nothing new.
        let mut delivered = 0;
        for _ in 0..10 {
            let events = svc.event_log_store.read_after(0, 10_000).await.unwrap();
            let fresh: Vec<_> = events
                .iter()
                .filter(|e| e.envelope.event_type.starts_with("acme_pr."))
                .skip(delivered)
                .cloned()
                .collect();
            if fresh.is_empty() {
                break;
            }
            delivered += fresh.len();
            for e in &fresh {
                assert!(consumer.handles(&e.envelope.event_type));
                consumer.handle(e).await.unwrap();
            }
        }
        // ping 0 → pong → ping → (pong: its own run led to this) stop.
        assert_eq!(
            rows(svc, "SELECT type, json_extract(payload, '$.number') FROM v_event WHERE type LIKE 'acme_pr.%' ORDER BY seq").await,
            json!([["acme_pr.ping", 0], ["acme_pr.pong", 1], ["acme_pr.ping", 2]])
        );
        let guarded = rows(
            svc,
            "SELECT status, error FROM v_collector_run WHERE id = 'pong'",
        )
        .await;
        assert_eq!(guarded[0][0], json!("skipped"), "{guarded}");
        assert!(
            guarded[0][1].as_str().unwrap().contains("loop guard"),
            "{guarded}"
        );
        assert_eq!(
            rows(
                svc,
                "SELECT count(*) FROM v_contribution_health WHERE consecutive_failures > 0"
            )
            .await,
            json!([[0]])
        );

        // A chain of `MAX_CHAIN` event-triggered runs of other collectors:
        // the next isn't run.
        let mut cause: Option<oxplow_domain::EventId> = None;
        for i in 0..crate::event_lineage::MAX_CHAIN {
            let mut synced = Envelope::typed::<oxplow_domain::events::schema::CollectorSynced>(
                "system",
                &oxplow_domain::events::schema::CollectorSyncedV1 {
                    collector: format!("collector:other/c{i}"),
                    trigger: "on".into(),
                    status: "ok".into(),
                    entities: Default::default(),
                    facts: 0,
                    elapsed_ms: 1,
                    error: None,
                },
            );
            synced.cause = cause.clone();
            cause = Some(synced.id.clone());
            svc.event_log_store.append(synced).await.unwrap();
        }
        let mut chained = Envelope::new(
            "acme_pr.pong",
            1,
            "collector:other/c3",
            json!({ "number": 40 }),
        )
        .unwrap();
        chained.cause = cause;
        let id = chained.id.clone();
        svc.event_log_store.append(chained).await.unwrap();
        let chained = svc.event_log_store.get(id).await.unwrap().unwrap();
        consumer.handle(&chained).await.unwrap();
        assert_eq!(
            rows(
                svc,
                "SELECT status, last_event_id FROM v_collector_run WHERE id = 'ping'"
            )
            .await,
            json!([["skipped", chained.seq]])
        );
        assert_eq!(
            rows(svc, "SELECT count(*) FROM v_event WHERE type = 'acme_pr.ping' AND json_extract(payload, '$.number') = 41").await,
            json!([[0]])
        );
    }
}
