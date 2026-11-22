//! The `collector.triggers` pump consumer (P7.B3): runs each enabled
//! collector whose `on: [...]` names a logged event.
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
//! the others. One whose extension lacks a permission it needs is
//! recorded as an `error` and doesn't run.
//!
//! `after_for(event)` is the union of the `after:` lists of the
//! collectors that event triggers (limited to consumers the pump has): an
//! event reaches the collectors once each of those has handled it, so a
//! collector's `after:` holds up only its own events. `after`, the
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
use crate::extension::extensions::descriptors::{gates, placement};
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
    spec.trigger
        .fires(&event.envelope.event_type, &event.envelope.payload)
}

/// Hand `event` to the fact engine, which runs the fact collectors it
/// triggers over one corpus: a snapshot's files (its whole tree for a
/// whole-tree collector, which alone runs on a take that recorded none),
/// or else the stream's latest snapshot.
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
        // A stream's HEAD moved: its whole-tree collectors restate the tree.
        "vcs.head.moved" => {
            if let Some(stream) = anchors.stream_id {
                if let Ok(Some(latest)) = svc
                    .snapshot_store
                    .latest_snapshot_id_for_stream(stream)
                    .await
                {
                    svc.metrics
                        .run_snapshot_collectors(
                            stream,
                            latest,
                            false,
                            Some(event.clone()),
                            &now,
                            None,
                        )
                        .await;
                }
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
            .unpinned(
                &svc.layout.project_dir,
                "a trigger is read from declarations; the run it starts pins",
            )
            .iter()
            .flat_map(|ext| {
                ext.collectors
                    .iter()
                    .filter(|c| c.facts.is_empty() && matches!(c.trigger, Trigger::On { .. }))
                    // Its owner is the namespace it is named in.
                    .map(|c| (ext.namespace.clone(), c.clone()))
            })
            .collect()
    }

    /// Every `on:` collector's trigger, level and what it waits on, fact
    /// and entity alike, by `(owner, id)`.
    fn triggers(svc: &Services) -> Vec<Triggered> {
        let load = svc.extension_catalog.unpinned(
            &svc.layout.project_dir,
            "a trigger is read from declarations; the run it starts pins",
        );
        let place = |owner: &str, id: &str| {
            load.iter()
                .find(|e| e.namespace == owner && e.collectors.iter().any(|c| c.id == id))
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

    /// Only the collectors `event_type` triggers wait on their `after:`:
    /// a `snapshot.taken` collector doesn't wait on the
    /// `change.analyze` a `thread.closed` collector names.
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
            .filter(|c| c.runs_on(event_type) && c.trigger.where_fits(&event.envelope.payload))
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
                    EventRun::Ran(Ok(_)) | EventRun::Skipped | EventRun::Denied => {}
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::lifecycle::log_thread_closed as log;
    use oxplow_domain::Envelope;
    use serde_json::json;

    const HEAD: &str =
        "manifest: 2\nname: work\nintent: { purpose: t, origin: null, examples: [] }\npermissions: [process.run]\n";

    async fn extension(
        svc: &std::sync::Arc<Services>,
        collectors: &str,
        files: &[(&str, &str)],
    ) -> crate::vocabulary_reactor::VocabularyErrors {
        let dir = svc.layout.project_dir.join(".oxplow/extensions/work");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("extension.yaml"),
            format!("{HEAD}collectors:\n{collectors}"),
        )
        .unwrap();
        for (name, body) in files {
            std::fs::write(dir.join(name), body).unwrap();
        }
        crate::test_fixtures::activate_with_problems(svc).await
    }

    const SEEN: &str = "def transform(input):\n    return {\"entities\": {\"seen\": [{\"ev\": r[\"ev\"], \"id\": r[\"id\"], \"thread\": r[\"thread\"], \"kind\": input[\"event\"][\"type\"]} for r in input[\"rows\"]]}}\n";

    fn on_thread_closed(filter: &str) -> String {
        format!(
            "  - id: seen\n    runtime: starlark\n    entry: seen.star\n    on: [thread.closed]{filter}\n    input: \"SELECT :thread_id AS thread, :event_seq AS ev, :event_id AS id\"\n    sync: upsert\n    entities:\n      - {{ name: seen, key: ev, columns: {{ ev: int, id: text, thread: int, kind: text }} }}\n"
        )
    }

    /// A `where` value names the payload's field as written in YAML (always
    /// a string); a number matches by value whatever its JSON spelling, and a
    /// field the payload lacks never matches.
    #[test]
    fn a_where_filter_compares_by_value_and_a_missing_field_never_matches() {
        let filter = |pairs: &[(&str, &str)]| -> oxplow_config::collectors::WhereFilter {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), vec![v.to_string()]))
                .collect()
        };
        let payload = json!({ "file_count": 1.0, "unchanged": false, "trigger": "git_refs" });
        assert!(oxplow_config::collectors::payload_matches(
            &filter(&[("file_count", "1")]),
            &payload
        ));
        assert!(oxplow_config::collectors::payload_matches(
            &filter(&[("file_count", "1.0")]),
            &payload
        ));
        assert!(!oxplow_config::collectors::payload_matches(
            &filter(&[("file_count", "2")]),
            &payload
        ));
        assert!(oxplow_config::collectors::payload_matches(
            &filter(&[("unchanged", "false")]),
            &payload
        ));
        assert!(oxplow_config::collectors::payload_matches(
            &filter(&[("trigger", "git_refs")]),
            &payload
        ));
        assert!(!oxplow_config::collectors::payload_matches(
            &filter(&[("missing", "x")]),
            &payload
        ));
        assert!(
            oxplow_config::collectors::payload_matches(&filter(&[]), &payload),
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
        let fx = crate::test_fixtures::services_with_thread().await;
        extension(&fx.svc, &on_thread_closed(""), &[("seen.star", SEEN)]).await;
        let consumer = CollectorTriggers::new(Arc::downgrade(&fx.svc));
        let ev = log(&fx.svc, fx.thread, "reviewer").await;
        assert!(consumer.handles(&ev.envelope.event_type));
        consumer.handle(&ev).await.unwrap();
        consumer.handle(&ev).await.unwrap();
        // `:event_id` is the event's id and `:event_seq` its seq,
        // as an effect's `input` binds them.
        assert_eq!(
            rows(&fx.svc, "SELECT ev, id, thread, kind FROM v_work_seen").await,
            json!([[
                ev.seq,
                ev.envelope.id.to_string(),
                fx.thread.value(),
                "thread.closed"
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
            rows(&fx.svc, "SELECT last_event_id FROM v_collector_checkpoint").await,
            json!([[ev.seq]])
        );
    }

    /// `change.analyzed` is a core type a collector can follow: its
    /// `where` picks the phase, its input binds the stream the event is
    /// anchored to, and its script reads the payload.
    #[tokio::test]
    async fn a_collector_follows_change_analyzed_by_phase() {
        use oxplow_domain::events::schema::{ChangeAnalyzed, ChangeAnalyzedV1};
        let fx = crate::test_fixtures::services_with_thread().await;
        let collectors = "  - id: seen\n    runtime: starlark\n    entry: seen.star\n    on: [change.analyzed]\n    where: { phase: files }\n    input: \"SELECT :stream_id AS sid, :event_seq AS ev\"\n    sync: upsert\n    entities:\n      - { name: seen, key: ev, columns: { ev: int, sid: int, change: int, phase: text } }\n";
        let script = "def transform(input):\n    p = input[\"event\"][\"payload\"]\n    return {\"entities\": {\"seen\": [{\"ev\": r[\"ev\"], \"sid\": r[\"sid\"], \"change\": p[\"change_id\"], \"phase\": p[\"phase\"]} for r in input[\"rows\"]]}}\n";
        let problems = extension(&fx.svc, collectors, &[("seen.star", script)]).await;
        assert!(problems.is_empty(), "{problems:?}");
        let consumer = CollectorTriggers::new(Arc::downgrade(&fx.svc));
        assert_eq!(
            rows(
                &fx.svc,
                "SELECT event_type, v FROM v_event_type WHERE event_type = 'change.analyzed'"
            )
            .await,
            json!([["change.analyzed", 1]])
        );
        let mut seqs = Vec::new();
        for phase in ["files", "duplicates"] {
            let env = oxplow_domain::Envelope::typed::<ChangeAnalyzed>(
                "system",
                &ChangeAnalyzedV1 {
                    change_id: 9,
                    kind: "commit".into(),
                    target: "abc".into(),
                    stream_id: 1,
                    base_revision: None,
                    head_revision: Some("git:abc".into()),
                    phase: phase.into(),
                },
            )
            .with_anchors(oxplow_domain::events::Anchors {
                stream_id: Some(oxplow_domain::StreamId::new(1)),
                ..Default::default()
            });
            let id = env.id.clone();
            fx.svc.event_log_store.append(env).await.unwrap();
            let ev = fx.svc.event_log_store.get(id).await.unwrap().unwrap();
            consumer.handle(&ev).await.unwrap();
            seqs.push(ev.seq);
        }
        assert_eq!(
            rows(&fx.svc, "SELECT ev, sid, change, phase FROM v_work_seen").await,
            json!([[seqs[0], 1, 9, "files"]]),
            "the duplicates phase doesn't match the `where`"
        );
    }

    /// A collector on `change.analyzed` reads the change's changed lines
    /// with `change_diff` (`tree.read`, which needs `files.read`): the
    /// added and removed lines, numbered. Without the permission or the
    /// `needs` it is refused at the script.
    #[tokio::test]
    async fn a_collector_reads_a_changes_lines_with_change_diff() {
        use oxplow_domain::events::schema::{ChangeAnalyzed, ChangeAnalyzedV1};
        let fx = crate::test_fixtures::services_with_thread().await;
        let root = fx.svc.layout.project_dir.clone();
        std::fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        crate::test_fixtures::commit_all(&root, "base");
        std::fs::write(root.join("a.txt"), "one\nTWO\nthree\nfour\n").unwrap();
        let sha = crate::test_fixtures::commit_all(&root, "change");
        let change = crate::change_analysis::ensure_change(
            &fx.svc,
            crate::change_analysis::ChangeTarget::Commit {
                sha,
                stream_id: None,
            },
        )
        .await
        .unwrap();
        let script = "def transform(input):\n    out = []\n    for f in change_diff(input[\"event\"][\"payload\"][\"change_id\"], paths = [\"a.txt\"]):\n        for h in f[\"hunks\"]:\n            for l in h[\"added\"]:\n                out.append({\"ev\": l[\"line\"], \"what\": \"+\" + l[\"text\"]})\n            for l in h[\"removed\"]:\n                out.append({\"ev\": 100 + l[\"line\"], \"what\": \"-\" + l[\"text\"]})\n    return {\"entities\": {\"line\": out}}\n";
        let collector = |needs: &str| {
            format!(
                "  - id: lines\n    runtime: starlark\n    entry: lines.star\n    on: [change.analyzed]\n    sync: upsert\n{needs}    entities:\n      - {{ name: line, key: ev, columns: {{ ev: int, what: text }} }}\n"
            )
        };
        let install = |permissions: &str, needs: &str| {
            let dir = fx.svc.layout.project_dir.join(".oxplow/extensions/work");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("extension.yaml"),
                format!(
                    "manifest: 2\nname: work\nintent: {{ purpose: t, origin: null, examples: [] }}\npermissions: {permissions}\ncollectors:\n{}",
                    collector(needs)
                ),
            )
            .unwrap();
            std::fs::write(dir.join("lines.star"), script).unwrap();
        };
        let event = || {
            let env = oxplow_domain::Envelope::typed::<ChangeAnalyzed>(
                "system",
                &ChangeAnalyzedV1 {
                    change_id: change.id,
                    kind: change.kind.clone(),
                    target: change.target.clone(),
                    stream_id: change.stream_id,
                    base_revision: None,
                    head_revision: None,
                    phase: "files".into(),
                },
            )
            .with_anchors(oxplow_domain::events::Anchors {
                stream_id: Some(oxplow_domain::StreamId::new(change.stream_id)),
                ..Default::default()
            });
            let svc = fx.svc.clone();
            async move {
                let id = env.id.clone();
                svc.event_log_store.append(env).await.unwrap();
                svc.event_log_store.get(id).await.unwrap().unwrap()
            }
        };
        let consumer = CollectorTriggers::new(Arc::downgrade(&fx.svc));

        install("[files.read]", "    needs: [tree.read]\n");
        let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
        assert!(problems.is_empty(), "{problems:?}");
        consumer.handle(&event().await).await.unwrap();
        assert_eq!(
            rows(&fx.svc, "SELECT ev, what FROM v_work_line ORDER BY ev").await,
            json!([[2, "+TWO"], [4, "+four"], [102, "-two"]])
        );

        // A re-run replaces this change's rows without a scoped replace: the
        // script reads the entity's old keys with `sql.read` and tombstones
        // the ones it no longer produces.
        let again = "def transform(input):\n    old = scope(\"sql.read\", {\"sql\": \"SELECT ev FROM v_work_line\"})\n    out = [{\"ev\": l[\"line\"], \"what\": l[\"text\"]} for f in change_diff(input[\"event\"][\"payload\"][\"change_id\"], paths = [\"a.txt\"]) for h in f[\"hunks\"] for l in h[\"added\"] if l[\"line\"] != 2]\n    keep = [r[\"ev\"] for r in out]\n    return {\"entities\": {\"line\": out}, \"deleted\": {\"line\": [r[\"ev\"] for r in old if r[\"ev\"] not in keep]}}\n";
        std::fs::write(
            fx.svc
                .layout
                .project_dir
                .join(".oxplow/extensions/work/lines.star"),
            again,
        )
        .unwrap();
        install(
            "[files.read, sql.read]",
            "    needs: [tree.read, sql.read]\n",
        );
        std::fs::write(
            fx.svc
                .layout
                .project_dir
                .join(".oxplow/extensions/work/lines.star"),
            again,
        )
        .unwrap();
        let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
        assert!(problems.is_empty(), "{problems:?}");
        consumer.handle(&event().await).await.unwrap();
        assert_eq!(
            rows(&fx.svc, "SELECT ev, what FROM v_work_line ORDER BY ev").await,
            json!([[4, "four"]])
        );

        // Declared, but the extension holds no `files.read`: refused.
        install("[process.run]", "    needs: [tree.read]\n");
        let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
        assert!(
            format!("{problems:?}").contains("needs `files.read` in `permissions:`"),
            "refused at load: {problems:?}"
        );
        // Permitted, but the collector doesn't declare `tree.read`.
        install("[files.read]", "");
        let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
        assert!(
            format!("{problems:?}").contains("change_diff()"),
            "refused at load: {problems:?}"
        );
    }

    /// A paced collector doesn't run as its event arrives: it's
    /// recorded pending (the latest event wins) and runs once its pacing
    /// allows, for that event, then clears.
    #[tokio::test]
    async fn a_settled_collector_waits_then_runs_once_for_the_latest_event() {
        let fx = crate::test_fixtures::services_with_thread().await;
        extension(
            &fx.svc,
            &on_thread_closed("\n    settle: 30s"),
            &[("seen.star", SEEN)],
        )
        .await;
        let consumer = CollectorTriggers::new(Arc::downgrade(&fx.svc));
        let first = log(&fx.svc, fx.thread, "reviewer").await;
        consumer.handle(&first).await.unwrap();
        let latest = log(&fx.svc, fx.thread, "reviewer").await;
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
        let fx = crate::test_fixtures::services_with_thread().await;
        extension(
            &fx.svc,
            &on_thread_closed("\n    idle: 2m"),
            &[("seen.star", SEEN)],
        )
        .await;
        let consumer = CollectorTriggers::new(Arc::downgrade(&fx.svc));
        consumer
            .handle(&log(&fx.svc, fx.thread, "reviewer").await)
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
        let fx = crate::test_fixtures::services_with_thread().await;
        extension(
            &fx.svc,
            &on_thread_closed("\n    where: { thread: \"reviewer\" }"),
            &[("seen.star", SEEN)],
        )
        .await;
        let consumer = CollectorTriggers::new(Arc::downgrade(&fx.svc));
        let other = log(&fx.svc, fx.thread, "other").await;
        consumer.handle(&other).await.unwrap();
        assert_eq!(
            rows(&fx.svc, "SELECT count(*) FROM v_collector_run").await,
            json!([[0]])
        );
        let mine = log(&fx.svc, fx.thread, "reviewer").await;
        consumer.handle(&mine).await.unwrap();
        assert_eq!(
            rows(&fx.svc, "SELECT ev FROM v_work_seen").await,
            json!([[mine.seq]])
        );
    }

    /// A `where` list matches any of its values; an event it excludes
    /// starts no run (no run row) — the filter runs before anything is queued.
    #[tokio::test]
    async fn a_where_list_runs_for_any_value_and_a_filtered_event_costs_no_run() {
        let fx = crate::test_fixtures::services_with_thread().await;
        extension(
            &fx.svc,
            &on_thread_closed("\n    where: { thread: [\"a\", \"reviewer\"] }"),
            &[("seen.star", SEEN)],
        )
        .await;
        let consumer = CollectorTriggers::new(Arc::downgrade(&fx.svc));
        let other = log(&fx.svc, fx.thread, "other").await;
        consumer.handle(&other).await.unwrap();
        assert_eq!(
            rows(&fx.svc, "SELECT count(*) FROM v_collector_run").await,
            json!([[0]])
        );
        let mine = log(&fx.svc, fx.thread, "reviewer").await;
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
        let fx = crate::test_fixtures::services_with_thread().await;
        let broken = on_thread_closed("")
            .replace("id: seen", "id: broken")
            .replace("entry: seen.star", "entry: broken.star")
            .replace("name: seen", "name: broken");
        extension(
            &fx.svc,
            &format!("{broken}{}", on_thread_closed("")),
            &[
                ("seen.star", SEEN),
                ("broken.star", "def transform(input):\n    return 1 // 0\n"),
            ],
        )
        .await;
        let consumer = CollectorTriggers::new(Arc::downgrade(&fx.svc));
        let ev = log(&fx.svc, fx.thread, "reviewer").await;
        consumer.handle(&ev).await.unwrap();
        assert_eq!(
            rows(
                &fx.svc,
                "SELECT id, outcome FROM v_collector_run ORDER BY id"
            )
            .await,
            json!([["broken", "failed"], ["seen", "ok"]])
        );
        assert_eq!(
            rows(&fx.svc, "SELECT json_extract(payload, '$.status') FROM v_event WHERE type = 'collector.synced' ORDER BY seq").await,
            json!([["error"], ["ok"]])
        );
    }

    /// `after()` is the core consumers the collectors' `after:` lists
    /// name; a collector's own type never triggers one.
    #[tokio::test]
    async fn after_and_handles_follow_the_declarations() {
        let fx = crate::test_fixtures::services_with_thread().await;
        crate::change_reactor::register(&fx.svc);
        crate::test_fixtures::install_code_metrics(&fx.svc.layout.project_dir);
        extension(
            &fx.svc,
            &on_thread_closed("").replace("    input:", "    after: [change.analyze]\n    input:"),
            &[("seen.star", SEEN)],
        )
        .await;
        let consumer = CollectorTriggers::new(Arc::downgrade(&fx.svc));
        assert_eq!(
            consumer.after_for("thread.closed"),
            vec!["change.analyze".to_string()]
        );
        // predecessors follow the event's own collectors — an event
        // no collector waiting on `change.analyze` handles doesn't wait.
        assert_eq!(consumer.after_for("snapshot.taken"), Vec::<String>::new());
        assert_eq!(
            consumer.after(),
            vec![
                "change.analyze".to_string(),
                "token_usage.turns".to_string()
            ],
            "every predecessor, for ordering"
        );
        assert!(consumer.handles("thread.closed"));
        // The default-on code gauges (the test extension's) handle `snapshot.taken`; an
        // event no collector names isn't handled.
        assert!(consumer.handles("snapshot.taken"));
        assert!(!consumer.handles("thread.checkpoint"));
        assert!(!consumer.handles("collector.synced"));
    }

    /// On one event, entity collectors run in their compiled order: each
    /// after the collectors its `after:` names.
    #[tokio::test]
    async fn collectors_run_after_the_collectors_they_name() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let one = |id: &str, after: &str| {
            on_thread_closed("")
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
        let ev = log(&fx.svc, fx.thread, "e").await;
        CollectorTriggers::new(Arc::downgrade(&fx.svc))
            .handle(&ev)
            .await
            .unwrap();
        assert_eq!(
            rows(
                &fx.svc,
                "SELECT json_extract(payload, '$.collector') FROM v_event WHERE type = 'collector.synced' AND json_extract(payload, '$.collector') LIKE 'work/%' ORDER BY seq",
            )
            .await,
            json!([["work/sooner"], ["work/later"]])
        );
    }

    /// A collector naming nothing in `after:` isn't loaded: the order
    /// can't be honored.
    #[tokio::test]
    async fn a_collector_after_nothing_is_a_load_error() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let held = extension(
            &fx.svc,
            &on_thread_closed("")
                .replace("    input:", "    after: [no.such_consumer]\n    input:"),
            &[("seen.star", SEEN)],
        )
        .await;
        // Held back from the active catalog; the activation says why.
        let why = crate::test_fixtures::shown_held(&fx.svc, &held, "work");
        assert!(why.contains("`no.such_consumer` is none of"), "{held:?}");
        let load = fx
            .svc
            .extension_catalog
            .snapshot(&fx.svc.layout.project_dir);
        assert!(load
            .extensions()
            .iter()
            .all(|e| e.name != "work" || e.collectors.is_empty()));
    }

    /// `acme-pr`: it declares `acme_pr.merged`, `.ping` and `.pong`
    /// (payload `{ number }`), and the given collectors.
    async fn emitter(svc: &std::sync::Arc<Services>, collectors: &str, files: &[(&str, &str)]) {
        let dir = svc.layout.project_dir.join(".oxplow/extensions/acme-pr");
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
            format!("manifest: 2\nname: acme-pr\nintent: {{ purpose: t, origin: null, examples: [] }}\nevent_types:\n  types:\n{types}collectors:\n{collectors}"),
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
            "acme_pr",
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
    /// `oxplow:oxplow/collector?collector=<owner>/<id>`, caused by that `collector.synced`.
    #[tokio::test]
    async fn a_collector_emits_its_extensions_events_with_its_run() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let svc = &fx.svc;
        emitter(svc,
            &prs(""),
            &[(
                "prs.star",
                &prs_script("[{\"type\": \"acme_pr.merged\", \"payload\": {\"number\": 12}, \"subject\": [\"oxplow:oxplow/collector?collector=acme_pr/prs\"]}]"),
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
            json!([["oxplow:oxplow/collector?collector=acme_pr/prs", synced[0][0], 12, 1]])
        );
        // What a preview (and `oxplow extension test`) shows: the events it
        // would emit, stored nowhere.
        let root = svc.layout.project_dir.clone();
        let preview = collector_runner::preview_collector(
            &Collectors::of(svc, &root),
            "acme_pr",
            "prs",
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_value(&preview.events).unwrap(),
            json!([{ "type": "acme_pr.merged", "v": 1, "payload": { "number": 12 }, "subject": ["oxplow:oxplow/collector?collector=acme_pr/prs"] }])
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
                "[{\"type\": \"thread.checkpoint\", \"payload\": {}}]".to_string(),
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
                "    on: [acme_pr.merged]\n",
                "[{\"type\": \"acme_pr.merged\", \"payload\": {\"number\": 1}}]".to_string(),
                "a type it runs on",
            ),
            ("", many, "at most"),
        ] {
            // Each on its own project: three failures in a row would
            // disable the collector, which is another rule's business.
            let fx = crate::test_fixtures::services_with_thread().await;
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
                rows(
                    svc,
                    "SELECT outcome FROM v_collector_run WHERE id = 'prs' AND latest = 1"
                )
                .await,
                json!([["failed"]]),
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
        let fx = crate::test_fixtures::services_with_thread().await;
        let svc = &fx.svc;
        let echo = |id: &str, on: &str| {
            format!("  - id: {id}\n    runtime: starlark\n    entry: {id}.star\n    on: [acme_pr.{on}]\n    sync: upsert\n    entities:\n      - {{ name: {id}, key: n, columns: {{ n: int }} }}\n")
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
            "SELECT outcome, error FROM v_collector_run WHERE id = 'pong' AND latest = 1",
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
                &oxplow_domain::events::schema::CollectorSyncedV2 {
                    collector: format!("other/c{i}").to_string(),
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
            "oxplow:oxplow/collector?collector=other/c3",
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
                "SELECT r.outcome, p.last_event_id FROM v_collector_checkpoint p JOIN v_collector_run r ON r.run_id = p.last_run_id WHERE p.id = 'ping'"
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
