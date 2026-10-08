//! `metric.*`: metrics as commands (`.context/commands.md`, P4.7/P4.8).
//! Metric *reads* are SQL (`v_metric_spec`, `v_fact`, `metric_grid()`);
//! everything that changes metrics or computes them is here.
//!
//! - `oxplow.metric.record` asserts a number oxplow didn't compute (a CI import,
//!   an agent's report) as a fact on the metric's measure, in the bus's
//!   transaction with its audit.
//! - `oxplow.metric.rebuild` runs every fact collector's whole-tree baseline. It
//!   drives snapshot captures and collector scripts — systems the bus
//!   doesn't own — so it is `External`. One collector runs now through
//!   `oxplow.collector.sync`.
//! - `oxplow.metric.scaffold` returns a starter collector and the config entries
//!   for a new metric; it writes nothing.
//!
//! `metric.enable { keys, enabled }` switches metrics on or off in this
//! project's `.oxplow/project.yaml`. Which edit that is depends on the
//! metric — a bundled code metric is off until a `use:` names it, a producer or
//! extension metric is on until an `enabled: false` marker turns it off — so
//! the command computes the new `metrics:` list with the metrics service's
//! rule and hands it to `oxplow.config.set`'s core: validated, written after
//! commit, logged as `config.changed`, undone by restoring the old list. The
//! reseed (and `v_metric_catalog`) follows `ConfigChanged` as for any
//! config edit.

use crate::commands::ops::Op;
use std::collections::BTreeMap;
use std::sync::Arc;

use oxplow_db::fact_store::{get_measure_tx, get_spec_tx, record_facts_tx};
use oxplow_db::{NewFact, NewMetricCapture, SqliteFactStore};
use oxplow_domain::{Actor, CommandError, StreamId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::config_commands::{change, ConfigTarget};
use super::util::{invalid, parse, ref_id, schema, sql};
use super::{Handler, HandlerOutput, Invocation, TxCtx};
use crate::metric_engine::FactFilter;
use crate::metrics_service::MetricsService;

pub const ENABLE: &str = "oxplow.metric.enable";
pub const RECORD: &str = "oxplow.metric.record";
pub const REBUILD: &str = "oxplow.metric.rebuild";
pub const SCAFFOLD: &str = "oxplow.metric.scaffold";

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnableInput {
    /// Metric keys (`v_metric_catalog.key`).
    pub keys: Vec<String>,
    /// On (`true`) or off.
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordInput {
    /// A metric with a source measure (`v_metric_spec.key`).
    pub key: String,
    /// The asserted value.
    pub value: f64,
    /// What it's about, as `kind:ref` (`file:src/a.rs`, `model:opus`).
    #[serde(default)]
    pub subject: Option<String>,
    /// Extra dimensions, recorded on the fact.
    #[serde(default)]
    pub dims: Option<BTreeMap<String, String>>,
    /// The stream (`stream:str1`); defaults to the caller's, else the primary.
    #[serde(default)]
    pub stream: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RebuildInput {
    /// Rebuild even when no gauge looks out of date.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScaffoldInput {
    /// Namespaced key (`acme.todo_density`); `oxplow.` is reserved.
    pub key: String,
    /// Display title (defaults from the key).
    #[serde(default)]
    pub title: Option<String>,
    /// Language tag for the starter gauge and the metric's slice.
    #[serde(default)]
    pub language: Option<String>,
    /// Glob the starter gauge sweeps (default `**/*`).
    #[serde(default)]
    pub glob: Option<String>,
}

/// What `metric.*` runs against.
#[derive(Clone)]
pub struct MetricTarget {
    pub config: ConfigTarget,
    pub metrics: MetricsService,
    pub facts: Arc<SqliteFactStore>,
    /// Where a run with no stream and no agent stream lands.
    pub primary_stream: StreamId,
}

/// The stream a run acts on: the one named, else the caller's, else the
/// primary. An agent acts only within its own stream.
fn stream_for(
    actor: &Actor,
    named: Option<&str>,
    primary: StreamId,
) -> Result<StreamId, CommandError> {
    let own = actor.stream_id();
    // The bus gives an agent with a thread its stream: one without either
    // acts in no stream.
    if own.is_none() && actor.is_agent_driven() {
        return Err(invalid(
            "/stream",
            "an agent with no thread acts in no stream",
        ));
    }
    let Some(named) = named else {
        return Ok(own.unwrap_or(primary));
    };
    let stream: StreamId = ref_id(named, "stream", "/stream")?;
    match own {
        Some(own) if actor.is_agent_driven() && own != stream => Err(invalid(
            "/stream",
            format!("`{stream}` is not the caller's stream `{own}`"),
        )),
        _ => Ok(stream),
    }
}

/// Assert `input` as a fact on its metric's measure. Returns the capture,
/// the measure and the stream. Pure over `ctx.conn` (the bus may retry).
/// Record the asserted fact: its capture's id and the measure it's on.
fn record_tx(
    ctx: &TxCtx<'_>,
    input: &RecordInput,
    primary: StreamId,
) -> Result<(i64, i64), CommandError> {
    let stream = stream_for(ctx.actor, input.stream.as_deref(), primary)?;
    let spec = get_spec_tx(ctx.conn, &input.key)
        .map_err(sql)?
        .ok_or_else(|| invalid("/key", crate::metric_engine::missing_metric(&input.key)))?;
    let measure_key = spec.source_measure.as_deref().ok_or_else(|| {
        invalid(
            "/key",
            "a formula metric has no source measure to assert a fact on",
        )
    })?;
    let measure = get_measure_tx(ctx.conn, measure_key)
        .map_err(sql)?
        .ok_or_else(|| {
            invalid(
                "/key",
                format!("the metric's measure `{measure_key}` is not defined (see v_measure)"),
            )
        })?;
    // A count metric aggregates fact ROWS: one asserted fact would read as
    // 1 whatever its value.
    if matches!(spec.aggregation.as_str(), "count" | "count_distinct") {
        return Err(invalid(
            "/key",
            "a `count` metric counts fact rows, so one asserted value can't represent it; \
             run its collector instead (`oxplow.collector.sync`, or `oxplow.test.record_run` for a test run)",
        ));
    }
    // Stamp the fact so the spec's own filter matches it (severity /
    // dim_eq), or the metric's reads would leave the asserted number out.
    // `oxplow.rule` is a fact column (`dim_value` reads it there).
    let filter = match spec.filter_json.as_deref() {
        Some(j) => FactFilter::from_json(j)?,
        None => FactFilter::default(),
    };
    let mut dims = input.dims.clone().unwrap_or_default();
    let mut rule = None;
    if let Some((k, v)) = &filter.dim_eq {
        if k == "oxplow.rule" {
            rule = Some(v.clone());
        } else {
            dims.insert(k.clone(), v.clone());
        }
    }
    // A ratio re-derives Σn/Σd, so the fact carries components; a
    // percent reads ×100, so den = 100 makes the asserted percent exact.
    let (numerator, denominator) = match spec.aggregation.as_str() {
        "ratio" if spec.unit.as_deref() == Some("%") => (Some(input.value), Some(100.0)),
        "ratio" => (Some(input.value), Some(1.0)),
        _ => (None, None),
    };
    let (subject_kind, subject_ref) = match input.subject.as_deref().and_then(|s| s.split_once(':'))
    {
        Some((k, r)) => (Some(k.to_string()), Some(r.to_string())),
        None => (None, input.subject.clone()),
    };
    let mut capture = NewMetricCapture::done(stream.value(), input.key.clone(), "agent-reported");
    capture.provenance = "asserted".into();
    // The assertion restates only what it emits, but it is anchored to
    // the stream's latest snapshot so the value has a tree state
    // (tsk71/tsk72); `scan_kind` keeps that from reading as a scanned set.
    capture.scan_kind = "asserted".into();
    capture.snapshot_id =
        oxplow_db::analytics_stores::latest_snapshot_id_for_stream_tx(ctx.conn, stream)
            .map_err(sql)?;
    // An agent's assertion is its thread's, made in its open turn
    // (tsk923); anyone else's is no turn's.
    if let Actor::Agent {
        thread_id: Some(thread),
        ..
    } = ctx.actor
    {
        capture.thread_id = Some(thread.value());
        capture.turn_id = oxplow_db::agent_stores::open_turn_ids_tx(ctx.conn, *thread)
            .map_err(CommandError::from)?
            .first()
            .map(|t| t.value());
    }
    let fact = NewFact {
        subject_kind,
        subject_ref,
        dims_json: (!dims.is_empty())
            .then(|| serde_json::to_string(&dims).ok())
            .flatten(),
        severity: filter.severity.clone(),
        rule,
        numerator,
        denominator,
        ..NewFact::new(measure.id, input.value)
    };
    let capture_id = record_facts_tx(ctx.conn, &capture, &[fact], None)?;
    Ok((capture_id, measure.id))
}

/// The `metric.*` commands.
pub fn ops(target: MetricTarget) -> Vec<Op> {
    let MetricTarget {
        config: config_target,
        metrics,
        facts,
        primary_stream,
    } = target;
    let record = {
        let facts = facts.clone();
        Op::new(
            "metrics.write",
            "record",
            schema::<RecordInput>(),
            false,
            Handler::Tx(Arc::new(move |ctx: &TxCtx<'_>, input| {
                let input: RecordInput = parse(input)?;
                let (capture_id, measure_id) = record_tx(ctx, &input, primary_stream)?;
                let facts = facts.clone();
                Ok(HandlerOutput {
                    result: json!({
                        "capture_id": capture_id,
                        "key": input.key,
                        "provenance": "asserted",
                    }),
                    after_commit: Some(Box::new(move || facts.facts_committed([measure_id]))),
                    ..HandlerOutput::default()
                })
            })),
        )
    };
    let rebuild = {
        let metrics = metrics.clone();
        Op::new(
            "metrics.write",
            "rebuild",
            schema::<RebuildInput>(),
            false,
            Handler::External(Arc::new(move |_: Invocation, input| {
                let metrics = metrics.clone();
                Box::pin(async move {
                    let input: RebuildInput = parse(input)?;
                    let report = metrics
                        .rebuild_baseline(input.force)
                        .await
                        .map_err(|message| CommandError::Failed { message })?;
                    Ok(HandlerOutput {
                        result: serde_json::to_value(report).expect("report serializes"),
                        ..HandlerOutput::default()
                    })
                })
            })),
        )
    };
    let scaffold = {
        let metrics = metrics.clone();
        Op::new(
            "metrics.read",
            "scaffold",
            schema::<ScaffoldInput>(),
            false,
            Handler::Tx(Arc::new(move |_ctx: &TxCtx<'_>, input| {
                let input: ScaffoldInput = parse(input)?;
                let scaffold = metrics
                    .metric_scaffold(&input.key, input.title, input.language, input.glob)
                    .map_err(|message| invalid("/key", message))?;
                Ok(HandlerOutput {
                    result: serde_json::to_value(scaffold).expect("scaffold serializes"),
                    ..HandlerOutput::default()
                })
            })),
        )
    };
    let target = config_target;
    let enable = Op::new(
        "metrics.write",
        "enable",
        schema::<EnableInput>(),
        true,
        Handler::Tx(Arc::new(move |ctx: &super::TxCtx<'_>, input| {
            let input: EnableInput = parse(input)?;
            for key in &input.keys {
                let known: bool = ctx
                    .conn
                    .query_row(
                        "SELECT EXISTS (SELECT 1 FROM metric_catalog WHERE key = ?1)",
                        [key],
                        |r| r.get(0),
                    )
                    .map_err(sql)?;
                if !known {
                    return Err(CommandError::Invalid {
                        field: Some("/keys".into()),
                        message: format!("no metric `{key}` (see v_metric_catalog)"),
                    });
                }
            }
            let mut list = target
                .config
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .metrics
                .clone();
            for key in &input.keys {
                metrics.apply_metric_enabled(&mut list, key, input.enabled);
            }
            let value = serde_json::to_value(&list).map_err(|e| CommandError::Failed {
                message: e.to_string(),
            })?;
            change(
                &target,
                ctx.actor,
                "metrics",
                Some(value),
                super::config_commands::Layer::Project,
            )
        })),
    );
    vec![enable, record, rebuild, scaffold]
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::ThreadId;
    use serde_json::Value;

    fn agent() -> Actor {
        Actor::Agent {
            thread_id: Some(ThreadId::new(1)),
            stream_id: Some(StreamId::new(1)),
        }
    }

    async fn services() -> (Arc<crate::Services>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let svc = Arc::new(crate::Services::in_memory(dir.path()).unwrap());
        svc.streams.ensure_primary().await.unwrap();
        svc.metrics.seed_catalog().await;
        (svc, dir)
    }

    async fn run(svc: &crate::Services, name: &str, input: Value) -> Result<Value, CommandError> {
        svc.commands
            .run(&agent(), name, input, false)
            .await
            .map(|out| out.result)
    }

    async fn headline(svc: &crate::Services, key: &str) -> Option<f64> {
        let spec = svc.fact_store.get_spec(key).await.unwrap().unwrap();
        svc.metric_engine.headline_for_spec(&spec).await.unwrap()
    }

    /// P4.8 (tsk493): an asserted fact lands on the metric's measure with
    /// its subject, in the same transaction as the run's audit row.
    #[tokio::test]
    async fn record_stores_an_asserted_fact_and_audits_the_run() {
        let (svc, _dir) = services().await;
        svc.fact_store
            .upsert_spec(oxplow_db::NewMetricSpec::base(
                "ci.flaky_rate",
                "Flaky rate",
                "oxplow.ast_hit",
                "last",
            ))
            .await
            .unwrap();
        let out = run(
            &svc,
            RECORD,
            json!({ "key": "ci.flaky_rate", "value": 0.12, "subject": "suite:unit" }),
        )
        .await
        .unwrap();
        assert_eq!(out["provenance"], "asserted");
        let measure = svc
            .fact_store
            .get_measure("oxplow.ast_hit")
            .await
            .unwrap()
            .unwrap();
        let facts = svc.fact_store.facts_for_measure(measure.id).await.unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].value, 0.12);
        assert_eq!(facts[0].provenance, "asserted");
        assert_eq!(facts[0].source, "agent-reported");
        assert_eq!(facts[0].subject_kind.as_deref(), Some("suite"));
        assert_eq!(facts[0].subject_ref.as_deref(), Some("unit"));
        let audited: i64 = svc
            .db
            .read(|tx| {
                tx.query_row(
                    "SELECT count(*) FROM command_audit
                     WHERE command = 'oxplow.metric.record' AND actor_kind = 'agent' AND outcome = 'ok'",
                    [],
                    |r| r.get(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(audited, 1);
    }

    /// tsk923: an agent's assertion is its thread's, made in its open
    /// turn — the capture carries both; a person's carries neither.
    #[tokio::test]
    async fn an_agents_assertion_carries_its_thread_and_turn() {
        let fx = crate::test_fixtures::services_with_effort().await;
        fx.svc.metrics.seed_catalog().await;
        fx.svc
            .fact_store
            .upsert_spec(oxplow_db::NewMetricSpec::base(
                "ci.flaky_rate",
                "Flaky rate",
                "oxplow.ast_hit",
                "last",
            ))
            .await
            .unwrap();
        fx.svc
            .hook_ingest
            .ingest(crate::hook_ingest::HookEnvelope {
                kind: oxplow_domain::HookKind::UserPromptSubmit,
                thread_id: Some(fx.thread),
                stream_id: None,
                agent_session_id: None,
                session_id: Some("s".into()),
                payload_json: "{}".into(),
                prompt: Some("go".into()),
                decision: None,
            })
            .await
            .unwrap();
        let agent = Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        for actor in [&agent, &Actor::Human] {
            fx.svc
                .commands
                .run(
                    actor,
                    RECORD,
                    json!({ "key": "ci.flaky_rate", "value": 0.12 }),
                    false,
                )
                .await
                .unwrap();
        }
        type Row = (Option<i64>, Option<i64>);
        let (open, rows): (i64, Vec<Row>) = fx
            .svc
            .db
            .read(|c| {
                let open = c
                    .query_row(
                        "SELECT id FROM agent_turn WHERE ended_at IS NULL",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                let mut st = c
                    .prepare(
                        "SELECT thread_id, turn_id FROM metric_capture
                         WHERE producer = 'ci.flaky_rate' ORDER BY id",
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = st
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(oxplow_db::map_sql_err)?
                    .collect::<rusqlite::Result<_>>()
                    .map_err(oxplow_db::map_sql_err)?;
                Ok((open, rows))
            })
            .await
            .unwrap();
        assert_eq!(
            rows,
            vec![(Some(fx.thread.value()), Some(open)), (None, None)]
        );
    }

    /// The fact is stamped to match the metric's own filter (a rule-filtered
    /// metric) and carries ratio components (a percent ratio), so the
    /// metric reads back exactly the asserted number.
    #[tokio::test]
    async fn a_recorded_value_reads_back_through_its_metric() {
        let (svc, _dir) = services().await;
        // A built-in gauge has a spec only while it's on (tsk1046).
        let off = run(
            &svc,
            RECORD,
            json!({ "key": "oxplow.rust.unsafe_blocks", "value": 42.0 }),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(off.contains("is off in this project"), "{off}");
        svc.config.write().unwrap().metrics = vec![oxplow_config::MetricEntry {
            use_key: Some("oxplow.rust.unsafe_blocks".into()),
            ..Default::default()
        }];
        svc.metrics.seed_catalog().await;
        run(
            &svc,
            RECORD,
            json!({ "key": "oxplow.rust.unsafe_blocks", "value": 42.0 }),
        )
        .await
        .unwrap();
        assert_eq!(
            headline(&svc, "oxplow.rust.unsafe_blocks").await,
            Some(42.0)
        );
        run(
            &svc,
            RECORD,
            json!({ "key": "oxplow.coverage.abs_pct", "value": 85.0 }),
        )
        .await
        .unwrap();
        assert_eq!(headline(&svc, "oxplow.coverage.abs_pct").await, Some(85.0));
    }

    /// Refused, writing nothing: an unknown metric, a `count` metric (it
    /// counts rows), a formula metric (no measure), another stream.
    #[tokio::test]
    async fn record_refuses_what_it_cannot_represent() {
        let (svc, _dir) = services().await;
        let mut formula =
            oxplow_db::NewMetricSpec::base("ci.derived", "Derived", "oxplow.ast_hit", "last");
        formula.source_measure = None;
        formula.formula = Some("a / b".into());
        svc.fact_store.upsert_spec(formula).await.unwrap();
        for (input, says) in [
            (json!({ "key": "nope.unknown", "value": 1.0 }), "no metric"),
            (
                json!({ "key": "oxplow.todos", "value": 42.0 }),
                "counts fact rows",
            ),
            (json!({ "key": "ci.derived", "value": 1.0 }), "formula"),
            (
                json!({ "key": "oxplow.rust.unsafe_blocks", "value": 1.0, "stream": "stream:str2" }),
                "not the caller's stream",
            ),
        ] {
            let err = run(&svc, RECORD, input).await.unwrap_err();
            assert!(err.to_string().contains(says), "{err}");
        }
        let facts: i64 = svc
            .db
            .read(|tx| {
                tx.query_row(
                    "SELECT count(*) FROM fact f JOIN metric_capture c ON c.id = f.capture_id
                     WHERE c.provenance = 'asserted'",
                    [],
                    |r| r.get(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(facts, 0);
    }

    /// `oxplow.collector.sync` runs one of the project's fact collectors now and
    /// records its facts and its run.
    #[tokio::test]
    async fn collector_sync_runs_a_project_fact_collector() {
        let (svc, dir) = services().await;
        std::fs::write(
            dir.path().join("count.star"),
            "def transform(input):\n    return {\"facts\": [{\"measure\": \"oxplow.ast_hit\", \"value\": 42, \"rule\": \"answer\", \"subject\": \"tree:.\"}]}\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join(".oxplow")).unwrap();
        std::fs::write(
            dir.path().join(".oxplow/project.yaml"),
            "collectors:\n  - { id: repo.answer, doc: answer, runtime: starlark, entry: count.star, facts: [oxplow.ast_hit] }\n",
        )
        .unwrap();
        svc.reload_config_from_disk().unwrap();
        let out = run(
            &svc,
            crate::collector_runner::SYNC,
            json!({ "owner": "project", "id": "repo.answer" }),
        )
        .await
        .unwrap();
        assert_eq!(out["facts"], 1);
        let measure = svc
            .fact_store
            .get_measure("oxplow.ast_hit")
            .await
            .unwrap()
            .unwrap();
        let facts = svc.fact_store.facts_for_measure(measure.id).await.unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].value, 42.0);
        assert_eq!(facts[0].provenance, "observed");
        let synced = svc
            .sql
            .query_sql(
                "SELECT json_extract(payload, '$.trigger'), json_extract(payload, '$.facts') FROM v_event WHERE type = 'collector.synced'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(synced.rows).unwrap(),
            json!([["manual", 1]])
        );
    }

    /// P7.C2: a fact collector failing three runs in a row is disabled;
    /// `oxplow.collector.sync` then refuses it, naming why.
    #[tokio::test]
    async fn a_failing_fact_collector_is_disabled() {
        let (svc, dir) = services().await;
        std::fs::write(
            dir.path().join("bad.star"),
            "def transform(input):\n    return 1 // 0\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join(".oxplow")).unwrap();
        std::fs::write(
            dir.path().join(".oxplow/project.yaml"),
            "collectors:\n  - { id: repo.bad, runtime: starlark, entry: bad.star, facts: [oxplow.ast_hit] }\n",
        )
        .unwrap();
        svc.reload_config_from_disk().unwrap();
        let sync = || {
            run(
                &svc,
                crate::collector_runner::SYNC,
                json!({ "owner": "project", "id": "repo.bad" }),
            )
        };
        for _ in 0..3 {
            let err = sync().await.unwrap_err();
            assert!(matches!(err, CommandError::Failed { .. }), "{err:?}");
        }
        let err = sync().await.unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { message, .. } if message.contains("is disabled")),
            "{err:?}"
        );
    }

    /// `oxplow.metric.scaffold` is a read: a template, and nothing written; the
    /// reserved namespace is refused.
    #[tokio::test]
    async fn scaffold_returns_a_template_and_writes_nothing() {
        let (svc, dir) = services().await;
        let out = run(
            &svc,
            SCAFFOLD,
            json!({ "key": "acme.todo_density", "title": "TODO density" }),
        )
        .await
        .unwrap();
        assert_eq!(
            out["scriptPath"],
            "oxplow/collectors/acme_todo_density.star"
        );
        assert!(out["script"].as_str().unwrap().contains("def transform"));
        assert!(out["projectYaml"].as_str().unwrap().contains("collectors:"));
        assert!(!dir.path().join("oxplow/collectors").exists());
        let err = run(&svc, SCAFFOLD, json!({ "key": "oxplow.nope" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("reserved"), "{err}");
    }
}
