//! Advisories: extension-declared guidance for the coding agent (see
//! `extensions::Advisory`). Core runs each advisory's query with
//! `:effort_id` bound to the thread's open effort, at `post-tool-use` or
//! `prompt`, applies its `once_per` rule, and hands the messages to the
//! hook response. Post-tool-use hits are also recorded as nudges
//! (`v_agent_nudge`). See `.context/extensions.md` → "Advisories".

use std::path::Path;

use oxplow_db::{SqlCell, SqliteAgentNudgeStore};

use crate::extensions::{AdvisoryOn, AdvisoryOncePer, Extension};

/// One advisory that fired: its id (`<extension>/<advisory>`) and the text
/// for the agent (heading, then one message per line).
#[derive(Debug, Clone, PartialEq)]
pub struct AdvisoryHit {
    pub id: String,
    pub text: String,
}

/// Runs advisories and remembers which have fired — durably, in
/// `effort_once_mark` (P3.6), so a restart doesn't repeat one.
pub struct AdvisoryRunner {
    marks: SqliteAgentNudgeStore,
}

impl AdvisoryRunner {
    pub fn new(marks: SqliteAgentNudgeStore) -> Self {
        Self { marks }
    }

    /// Run every `on` advisory of the enabled `extensions` for `effort_id`
    /// and return the ones that fire. A failing query is logged and skipped.
    pub async fn run(
        &self,
        layer: &crate::sql_gateway::SqlGateway,
        extensions: &[Extension],
        on: AdvisoryOn,
        effort_id: i64,
    ) -> Vec<AdvisoryHit> {
        let mut hits = Vec::new();
        // Marks to record once every query has run, so a failure partway
        // can't consume a one-shot the agent never saw.
        let mut marks: Vec<String> = Vec::new();
        for ext in extensions.iter().filter(|e| e.enabled) {
            for a in ext.advisories.iter().filter(|a| a.on == on) {
                let id = format!("{}/{}", ext.name, a.id);
                if a.once_per == AdvisoryOncePer::Effort && self.has_fired(effort_id, &id).await {
                    continue;
                }
                let result = match layer
                    .run(
                        oxplow_db::SqlQuery::new(&a.query)
                            .named(vec![("effort_id".into(), SqlCell::Int(effort_id))])
                            .limit(None),
                    )
                    .await
                {
                    Ok(r) => r,
                    Err(error) => {
                        tracing::warn!(advisory = %id, %error, "advisory query failed");
                        continue;
                    }
                };
                let col = |name: &str| result.columns.iter().position(|c| c == name);
                let Some(msg_i) = col("message") else {
                    tracing::warn!(advisory = %id, "advisory query returned no `message` column");
                    continue;
                };
                let key_i = col("key");
                let mut lines = Vec::new();
                for row in &result.rows {
                    let text = cell_text(&row[msg_i]);
                    if text.is_empty() {
                        continue;
                    }
                    if a.once_per == AdvisoryOncePer::Row {
                        let key = key_i.map(|i| cell_text(&row[i])).unwrap_or_default();
                        let mark = format!("{id}#{key}");
                        if self.has_fired(effort_id, &mark).await || marks.contains(&mark) {
                            continue;
                        }
                        marks.push(mark);
                    }
                    lines.push(text);
                }
                if lines.is_empty() {
                    continue;
                }
                if a.once_per == AdvisoryOncePer::Effort {
                    marks.push(id.clone());
                }
                let mut text = a.heading.clone().map(|h| vec![h]).unwrap_or_default();
                text.extend(lines);
                hits.push(AdvisoryHit {
                    id,
                    text: text.join("\n"),
                });
            }
        }
        for m in marks {
            if let Err(error) = self.marks.claim_once(effort_id, &m).await {
                tracing::warn!(mark = %m, %error, "recording an advisory mark failed");
            }
        }
        hits
    }

    /// A failed read reads as "already fired": suppress rather than nag.
    async fn has_fired(&self, effort_id: i64, mark: &str) -> bool {
        self.marks.has_fired(effort_id, mark).await.unwrap_or(true)
    }
}

/// What running a thread's advisories needs (a pump reactor holds its own
/// copy; `Services::advisory_deps` builds one).
#[derive(Clone)]
pub struct AdvisoryDeps {
    pub advisories: std::sync::Arc<AdvisoryRunner>,
    pub effort_store: std::sync::Arc<oxplow_db::SqliteEffortStore>,
    pub thread_store: std::sync::Arc<oxplow_db::SqliteThreadStore>,
    pub worktrees: std::sync::Arc<crate::worktrees::WorktreeRouter>,
    pub approvals: std::sync::Arc<crate::exec_consent::ApprovalStore>,
    pub extension_catalog: std::sync::Arc<crate::extension_catalog::ExtensionCatalog>,
    pub db: oxplow_db::Database,
    pub sql: crate::sql_gateway::SqlGateway,
    pub collection: crate::collection::CollectionService,
}

/// Run the `on` advisories for `thread`'s open effort (only when exactly one
/// is open: under parallel sub-agents we can't say whose effort it is) —
/// or, for a tool event (`cause`), the effort it was anchored to — reading
/// extensions from the thread's stream worktree. Post-tool-use hits are
/// recorded as nudges, keyed by the cause.
pub async fn for_thread(
    svc: &AdvisoryDeps,
    thread: &oxplow_domain::ThreadId,
    on: AdvisoryOn,
    cause: Option<&crate::collection::RunCause>,
) -> Vec<AdvisoryHit> {
    use oxplow_db::EffortStore as _;
    use oxplow_domain::stores::ThreadStore as _;
    let effort = match cause.and_then(|c| c.anchors.effort_id) {
        Some(id) => svc.effort_store.get_effort(&id).await,
        None => svc.effort_store.find_single_open_for_thread(thread).await,
    };
    let Ok(Some(effort)) = effort else {
        return Vec::new();
    };
    let stream_id = match svc.thread_store.get(thread).await {
        Ok(Some(t)) => Some(t.stream_id.to_string()),
        _ => None,
    };
    let root = svc.worktrees.resolve(stream_id.as_deref()).await;
    let extensions = consented(&svc.approvals, &svc.extension_catalog.get(&root));
    let layer = svc.sql.clone();
    let hits = svc
        .advisories
        .run(&layer, &extensions, on, effort.id.value())
        .await;
    if on == AdvisoryOn::PostToolUse {
        for hit in &hits {
            svc.collection
                .persist_nudge(thread, Some(&effort), &hit.id, &hit.text, "advisory", cause)
                .await;
        }
    }
    hits
}

/// The extensions whose advisories may run: bundled ones, and shared ones
/// a person approved as they are now (tsk352). A teammate's or a
/// git-installed extension can't speak into the agent's context unseen.
pub fn consented(
    approvals: &crate::exec_consent::ApprovalStore,
    extensions: &[Extension],
) -> Vec<Extension> {
    extensions
        .iter()
        .filter(|e| {
            if e.origin == "bundled" || e.advisories.is_empty() {
                return true;
            }
            let p = crate::exec_consent::advisory_program(e);
            p.hash(Path::new(""))
                .is_ok_and(|h| approvals.is_approved(&p.key(), &h))
        })
        .cloned()
        .collect()
}

fn cell_text(c: &SqlCell) -> String {
    match c {
        SqlCell::Null(()) => String::new(),
        SqlCell::Text(t) => t.trim().to_string(),
        SqlCell::Int(i) => i.to_string(),
        SqlCell::Real(r) => r.to_string(),
        SqlCell::Bool(b) => b.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::Advisory;

    fn ext(advisories: Vec<Advisory>) -> Extension {
        let mut e = crate::extensions::load_extensions(tempfile::tempdir().unwrap().path())
            .into_iter()
            .next()
            .unwrap();
        e.name = "x".into();
        e.advisories = advisories;
        e
    }

    fn adv(id: &str, on: AdvisoryOn, once_per: AdvisoryOncePer, query: &str) -> Advisory {
        Advisory {
            id: id.into(),
            on,
            query: query.into(),
            once_per,
            heading: None,
        }
    }

    /// A layer and a runner over one database holding efforts 1–5 (a
    /// once-mark references its effort).
    async fn setup() -> (crate::sql_gateway::SqlGateway, AdvisoryRunner) {
        let db = oxplow_db::Database::in_memory();
        db.transaction(|tx| {
            tx.execute_batch(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                   VALUES (1, 'primary', 'p', 'main', 'r', 'r', '/r', '2026-01-01', '2026-01-01');
                 INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                   VALUES (1, 1, 't', 'active', '2026-01-01', '2026-01-01');
                 INSERT INTO effort (id, work_item, thread_id, started_at, ended_at) VALUES
                   (1, 'work_item:linear:A-1', 1, '2026-01-01', '2026-01-01'),
                   (2, 'work_item:linear:A-2', 1, '2026-01-01', '2026-01-01'),
                   (3, 'work_item:linear:A-3', 1, '2026-01-01', '2026-01-01'),
                   (4, 'work_item:linear:A-4', 1, '2026-01-01', '2026-01-01'),
                   (5, 'work_item:linear:A-5', 1, '2026-01-01', '2026-01-01');",
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();
        (
            crate::sql_gateway::SqlGateway::new(db.clone()),
            AdvisoryRunner::new(SqliteAgentNudgeStore::new(db)),
        )
    }

    #[tokio::test]
    async fn once_per_effort_fires_once_per_effort_and_only_with_rows() {
        let (l, runner) = setup().await;
        let e = vec![ext(vec![adv(
            "low",
            AdvisoryOn::PostToolUse,
            AdvisoryOncePer::Effort,
            "SELECT 'add tests (' || :effort_id || ')' AS message WHERE :effort_id <> 3",
        )])];
        let run = |id: i64| runner.run(&l, &e, AdvisoryOn::PostToolUse, id);
        assert_eq!(
            run(1).await,
            vec![AdvisoryHit {
                id: "x/low".into(),
                text: "add tests (1)".into()
            }]
        );
        assert!(run(1).await.is_empty(), "once per effort");
        assert!(run(3).await.is_empty(), "no rows, no hit");
        assert_eq!(run(2).await.len(), 1, "another effort fires again");
        assert!(
            runner.run(&l, &e, AdvisoryOn::Prompt, 5).await.is_empty(),
            "only advisories for this hook point run"
        );
        // Durable: a runner over the same database after a restart agrees.
        let again = AdvisoryRunner::new(runner.marks.clone());
        assert!(again
            .run(&l, &e, AdvisoryOn::PostToolUse, 1)
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn once_per_row_fires_each_key_once_and_turn_always() {
        let (l, runner) = setup().await;
        let mut rows = adv(
            "cross",
            AdvisoryOn::Prompt,
            AdvisoryOncePer::Row,
            "SELECT 'a crossed' AS message, 'a' AS key UNION ALL SELECT 'b crossed', 'b'",
        );
        rows.heading = Some("# Thresholds".into());
        let every = adv(
            "deltas",
            AdvisoryOn::Prompt,
            AdvisoryOncePer::Turn,
            "SELECT '- x: 1 → 2' AS message",
        );
        let e = vec![ext(vec![rows, every])];
        let first = runner.run(&l, &e, AdvisoryOn::Prompt, 1).await;
        assert_eq!(
            first,
            vec![
                AdvisoryHit {
                    id: "x/cross".into(),
                    text: "# Thresholds\na crossed\nb crossed".into()
                },
                AdvisoryHit {
                    id: "x/deltas".into(),
                    text: "- x: 1 → 2".into()
                },
            ]
        );
        let second = runner.run(&l, &e, AdvisoryOn::Prompt, 1).await;
        assert_eq!(
            second,
            vec![AdvisoryHit {
                id: "x/deltas".into(),
                text: "- x: 1 → 2".into()
            }]
        );
    }

    #[tokio::test]
    async fn disabled_extensions_and_bad_queries_contribute_nothing() {
        let (l, runner) = setup().await;
        let mut off = ext(vec![adv(
            "a",
            AdvisoryOn::Prompt,
            AdvisoryOncePer::Turn,
            "SELECT 'm' AS message",
        )]);
        off.enabled = false;
        let bad = ext(vec![adv(
            "b",
            AdvisoryOn::Prompt,
            AdvisoryOncePer::Turn,
            "SELECT nope FROM nowhere",
        )]);
        assert!(runner
            .run(&l, &[off, bad], AdvisoryOn::Prompt, 1)
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn thread_advisories_use_the_open_effort_and_record_post_tool_use_nudges() {
        let f = crate::test_fixtures::services_with_effort().await;
        let dir = f.svc.layout.project_dir.join("oxplow/extensions/guide");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("extension.yaml"),
            "manifest: 2\nname: guide\nintent:\n  purpose: test\nadvisories:\n  - id: hello\n    on: post-tool-use\n    query: SELECT 'effort ' || :effort_id AS message\n",
        )
        .unwrap();
        // A project extension's advisories are silent until a person
        // approves them (tsk352).
        assert!(for_thread(
            &f.svc.advisory_deps(),
            &f.thread,
            AdvisoryOn::PostToolUse,
            None
        )
        .await
        .is_empty());
        let exts = crate::extensions::load_extensions(&f.svc.layout.project_dir);
        let program =
            crate::exec_consent::advisory_program(exts.iter().find(|e| e.name == "guide").unwrap());
        f.svc
            .approvals
            .approve(&program.key(), &program.hash(Path::new("")).unwrap())
            .unwrap();
        let hits = for_thread(
            &f.svc.advisory_deps(),
            &f.thread,
            AdvisoryOn::PostToolUse,
            None,
        )
        .await;
        assert_eq!(
            hits,
            vec![AdvisoryHit {
                id: "guide/hello".into(),
                text: format!("effort {}", f.effort.value())
            }]
        );
        let out = crate::sql_gateway::SqlGateway::new(f.svc.db.clone())
            .query_sql("SELECT kind, message FROM v_agent_nudge", vec![], None)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            serde_json::json!([["guide/hello", format!("effort {}", f.effort.value())]])
        );
        assert!(
            for_thread(&f.svc.advisory_deps(), &f.thread, AdvisoryOn::Prompt, None)
                .await
                .is_empty()
        );
    }

    fn delta(
        key: &str,
        title: &str,
        baseline: f64,
        current: f64,
        crossing: Option<&str>,
    ) -> oxplow_db::EffortMetricDelta {
        oxplow_db::EffortMetricDelta {
            key: key.into(),
            title: title.into(),
            unit: None,
            direction: "lower-better".into(),
            kind: "gauge".into(),
            category: None,
            language: None,
            agg: "level".into(),
            baseline: Some(baseline),
            current,
            delta: Some(current - baseline),
            changed: baseline != current,
            attributed_files: None,
            sample_count: 2,
            target: None,
            warn_at: Some(5.0),
            fail_at: Some(10.0),
            crossing: crossing.map(str::to_string),
            latest_run_id: None,
        }
    }

    #[tokio::test]
    async fn bundled_metric_advisories_match_the_old_prompt_text() {
        let f = crate::test_fixtures::services_with_effort().await;
        let effort = f.effort.value();
        f.svc
            .effort_evidence_store
            .replace(
                effort,
                vec![
                    delta(
                        "test.unsafe_blocks",
                        "unsafe blocks",
                        3.0,
                        12.0,
                        Some("fail"),
                    ),
                    delta("test.flat", "flat", 7.0, 7.0, None),
                    delta("agent.tokens.total", "tokens", 100.0, 5000.0, None),
                    delta("test.ratio", "ratio", 1.5, 1.0, None),
                ],
                Vec::new(),
                String::new(),
            )
            .await
            .unwrap();
        let first = for_thread(&f.svc.advisory_deps(), &f.thread, AdvisoryOn::Prompt, None).await;
        assert_eq!(
            first,
            vec![
                AdvisoryHit {
                    id: "oxplow-analytics/metric-deltas".into(),
                    text: "# Metric deltas (this effort)\n- ratio: 1.5 → 1 (Δ -0.5)\n- unsafe blocks: 3 → 12 (Δ +9)\n(Advisory — for awareness, not gating.)".into(),
                },
                AdvisoryHit {
                    id: "oxplow-analytics/threshold-crossed".into(),
                    text: "# Metric thresholds\n⚠ unsafe blocks crossed its fail threshold (10)".into(),
                },
            ]
        );
        let second = for_thread(&f.svc.advisory_deps(), &f.thread, AdvisoryOn::Prompt, None).await;
        assert_eq!(
            second.len(),
            1,
            "the crossing is once per metric: {second:?}"
        );
        assert_eq!(second[0].id, "oxplow-analytics/metric-deltas");
    }

    #[tokio::test]
    async fn bundled_coverage_advisory_fires_once_below_target() {
        let f = crate::test_fixtures::services_with_effort().await;
        let obs = |pct: f64| oxplow_db::EffortObservation {
            kind: "diff-coverage".into(),
            provenance: "observed".into(),
            source: "post-tool-bash".into(),
            metric_value: Some(pct),
            payload_json: None,
            local_snapshot_id: None,
            created_at: oxplow_domain::Timestamp::now(),
        };
        f.svc
            .effort_evidence_store
            .replace(f.effort.value(), Vec::new(), vec![obs(91.0)], String::new())
            .await
            .unwrap();
        assert!(for_thread(
            &f.svc.advisory_deps(),
            &f.thread,
            AdvisoryOn::PostToolUse,
            None
        )
        .await
        .is_empty());
        f.svc
            .effort_evidence_store
            .replace(f.effort.value(), Vec::new(), vec![obs(42.4)], String::new())
            .await
            .unwrap();
        let hits = for_thread(
            &f.svc.advisory_deps(),
            &f.thread,
            AdvisoryOn::PostToolUse,
            None,
        )
        .await;
        assert_eq!(
            hits,
            vec![AdvisoryHit {
                id: "oxplow-analytics/coverage-target".into(),
                text: "Diff coverage on this effort's changed lines is 42%, below the 80% target. Add tests for the uncovered changed lines before closing (advisory — oxplow won't block you). See the effort's coverage panel for which lines are uncovered.".into(),
            }]
        );
        assert!(
            for_thread(
                &f.svc.advisory_deps(),
                &f.thread,
                AdvisoryOn::PostToolUse,
                None
            )
            .await
            .is_empty(),
            "once per effort"
        );
    }

    #[test]
    fn a_shared_extensions_advisories_speak_only_once_approved() {
        let dir = tempfile::tempdir().unwrap();
        let approvals = crate::exec_consent::ApprovalStore::for_tests(dir.path());
        let mut shared = ext(vec![adv(
            "nag",
            AdvisoryOn::Prompt,
            AdvisoryOncePer::Effort,
            "SELECT 'x' AS message",
        )]);
        shared.origin = "project".into();
        shared.name = "team".into();
        let mut bundled = ext(vec![adv(
            "b",
            AdvisoryOn::Prompt,
            AdvisoryOncePer::Effort,
            "SELECT 'y' AS message",
        )]);
        bundled.origin = "bundled".into();
        let names = |exts: Vec<Extension>| exts.into_iter().map(|e| e.name).collect::<Vec<_>>();
        let all = vec![shared.clone(), bundled.clone()];

        assert_eq!(
            names(consented(&approvals, &all)),
            vec![bundled.name.clone()]
        );

        let program = crate::exec_consent::advisory_program(&shared);
        let version = program.hash(dir.path()).unwrap();
        approvals.approve(&program.key(), &version).unwrap();
        assert_eq!(
            names(consented(&approvals, &all)),
            vec!["team".to_string(), bundled.name.clone()]
        );

        // Changing what it says needs approving again.
        shared.advisories[0].query = "SELECT 'rm -rf everything' AS message".into();
        assert_eq!(
            names(consented(&approvals, &[shared, bundled.clone()])),
            vec![bundled.name]
        );
    }
}
