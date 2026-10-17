//! Extensions that ship inside oxplow, compiled in from the repo's
//! `extensions/<name>/` folder. They load exactly like project extensions
//! (same format, same loader) but are read-only, and their names are
//! reserved. See `.context/extensions.md`.

/// One bundled extension: its name and its files as
/// `(path inside the extension folder, bytes)` — bytes, so a custom
/// component's assets ship as they are.
pub struct BundledExtension {
    pub name: &'static str,
    pub files: &'static [(&'static str, &'static [u8])],
    /// oxplow itself relies on it (`oxplow-foundation`, its own
    /// commands): it can't be disabled, and its commands are registered
    /// when services are built rather than by the reconciler.
    pub required: bool,
}

impl BundledExtension {
    /// The file at `path` in it, as text; `None` when it isn't there or
    /// isn't UTF-8.
    pub fn file(&self, path: &str) -> Option<&'static str> {
        self.bytes(path).and_then(|b| std::str::from_utf8(b).ok())
    }

    /// The file at `path` in it.
    pub fn bytes(&self, path: &str) -> Option<&'static [u8]> {
        self.files.iter().find(|(p, _)| *p == path).map(|(_, f)| *f)
    }
}

macro_rules! ext_file {
    ($ext:literal, $path:literal) => {
        (
            $path,
            include_bytes!(concat!("../../../extensions/", $ext, "/", $path)) as &[u8],
        )
    };
}

pub const BUNDLED: &[BundledExtension] = &[
    BundledExtension {
        name: "oxplow-foundation",
        files: &[
            ext_file!("oxplow-foundation", "README.md"),
            ext_file!("oxplow-foundation", "icons/claude.svg"),
            ext_file!("oxplow-foundation", "icons/codex.svg"),
            ext_file!("oxplow-foundation", "icons/opencode.svg"),
            ext_file!("oxplow-foundation", "lenses/catalog.yaml"),
            ext_file!("oxplow-foundation", "lenses/closed-threads.yaml"),
            ext_file!("oxplow-foundation", "lenses/done-work.yaml"),
            ext_file!("oxplow-foundation", "lenses/effort-files.yaml"),
            ext_file!("oxplow-foundation", "lenses/effort-summary.yaml"),
            ext_file!("oxplow-foundation", "lenses/file-history.yaml"),
            ext_file!("oxplow-foundation", "lenses/hand-off.yaml"),
            ext_file!("oxplow-foundation", "lenses/handoff-items.yaml"),
            ext_file!("oxplow-foundation", "lenses/handoff-recipients.yaml"),
            ext_file!("oxplow-foundation", "lenses/hook-events.yaml"),
            ext_file!("oxplow-foundation", "lenses/problems.yaml"),
            ext_file!("oxplow-foundation", "lenses/recent-commits.yaml"),
            ext_file!("oxplow-foundation", "lenses/session-transcript.yaml"),
            ext_file!("oxplow-foundation", "lenses/snapshots.yaml"),
            ext_file!("oxplow-foundation", "lenses/symbols.yaml"),
            ext_file!("oxplow-foundation", "lenses/thread-handoffs-in.yaml"),
            ext_file!("oxplow-foundation", "lenses/thread-handoffs-out.yaml"),
            ext_file!("oxplow-foundation", "lenses/thread-notes.yaml"),
            ext_file!("oxplow-foundation", "lenses/thread-sessions.yaml"),
            ext_file!("oxplow-foundation", "lenses/thread-work.yaml"),
            ext_file!("oxplow-foundation", "lenses/wiki-freshness.yaml"),
            ext_file!("oxplow-foundation", "lenses/wiki-index.yaml"),
            ext_file!("oxplow-foundation", "lenses/wiki-pages.yaml"),
            ext_file!("oxplow-foundation", "lenses/wiki-visited.yaml"),
            ext_file!("oxplow-foundation", "lenses/work-next.yaml"),
            ext_file!("oxplow-foundation", "collectors/clojure/defn_count.star"),
            ext_file!("oxplow-foundation", "collectors/code/doc_coverage.star"),
            ext_file!("oxplow-foundation", "collectors/effort/test_outcome.star"),
            ext_file!("oxplow-foundation", "collectors/code/duplicate_lines.star"),
            ext_file!("oxplow-foundation", "collectors/code/fn_count.star"),
            ext_file!(
                "oxplow-foundation",
                "collectors/code/high_complexity_fns.star"
            ),
            ext_file!("oxplow-foundation", "collectors/code/long_functions.star"),
            ext_file!("oxplow-foundation", "collectors/code/todos.star"),
            ext_file!(
                "oxplow-foundation",
                "collectors/csharp/blocking_async_calls.star"
            ),
            ext_file!("oxplow-foundation", "collectors/csharp/empty_catch.star"),
            ext_file!("oxplow-foundation", "collectors/rust/panic_macros.star"),
            ext_file!("oxplow-foundation", "collectors/rust/unsafe_blocks.star"),
            ext_file!(
                "oxplow-foundation",
                "collectors/rust/unwrap_expect_calls.star"
            ),
            ext_file!("oxplow-foundation", "collectors/ts/any_usage.star"),
            ext_file!("oxplow-foundation", "collectors/ts/console_calls.star"),
            ext_file!(
                "oxplow-foundation",
                "collectors/ts/non_null_assertions.star"
            ),
            ext_file!("oxplow-foundation", "collectors/ts/ts_ignore.star"),
            ext_file!("oxplow-foundation", "extension.yaml"),
            ext_file!("oxplow-foundation", "providers/anthropic.star"),
            ext_file!("oxplow-foundation", "providers/openai_compatible.star"),
            ext_file!("oxplow-foundation", "providers/openrouter.star"),
            ext_file!("oxplow-foundation", "providers/typesafe.star"),
        ],
        required: true,
    },
    BundledExtension {
        name: "oxplow-bundled",
        required: false,
        files: &[
            ext_file!("oxplow-bundled", "README.md"),
            ext_file!("oxplow-bundled", "collectors/effort_churn.star"),
            ext_file!("oxplow-bundled", "components/history-graph/app.css"),
            ext_file!("oxplow-bundled", "components/history-graph/app.js"),
            ext_file!("oxplow-bundled", "components/history-graph/index.html"),
            ext_file!("oxplow-bundled", "commands/work-next.md"),
            ext_file!("oxplow-bundled", "effects/verify_unchecked.star"),
            ext_file!("oxplow-bundled", "effects/repair_disabled.star"),
            ext_file!("oxplow-bundled", "effects/infer_reasoning.star"),
            ext_file!("oxplow-bundled", "effects/store_reasoning.star"),
            ext_file!("oxplow-bundled", "event_types/accepted.v1.json"),
            ext_file!("oxplow-bundled", "event_types/changes_requested.v1.json"),
            ext_file!("oxplow-bundled", "event_types/finished_cleared.v1.json"),
            ext_file!("oxplow-bundled", "extension.yaml"),
            ext_file!(
                "oxplow-bundled",
                "fixtures/extension runs over the last week.yaml"
            ),
            ext_file!("oxplow-bundled", "fixtures/reasoning-asked.yaml"),
            ext_file!("oxplow-bundled", "fixtures/reasoning-no-lists.yaml"),
            ext_file!("oxplow-bundled", "fixtures/reasoning-not-json.yaml"),
            ext_file!("oxplow-bundled", "fixtures/reasoning-only-claims.yaml"),
            ext_file!("oxplow-bundled", "fixtures/reasoning-stored.yaml"),
            ext_file!("oxplow-bundled", "fixtures/infer-reasoning-asks.yaml"),
            ext_file!(
                "oxplow-bundled",
                "fixtures/infer-reasoning-nothing-to-read.yaml"
            ),
            ext_file!("oxplow-bundled", "fixtures/infer-reasoning-request.yaml"),
            ext_file!(
                "oxplow-bundled",
                "fixtures/infer-reasoning-skips-unbound.yaml"
            ),
            ext_file!("oxplow-bundled", "fixtures/repair-files-an-item.yaml"),
            ext_file!(
                "oxplow-bundled",
                "fixtures/repair-comments-on-the-open-item.yaml"
            ),
            ext_file!(
                "oxplow-bundled",
                "fixtures/repair-skips-without-a-work-list.yaml"
            ),
            ext_file!("oxplow-bundled", "handlers/accept.star"),
            ext_file!("oxplow-bundled", "handlers/clear_finished.star"),
            ext_file!("oxplow-bundled", "handlers/infer_reasoning.star"),
            ext_file!("oxplow-bundled", "handlers/request_changes.star"),
            ext_file!("oxplow-bundled", "lenses/agent-interface.yaml"),
            ext_file!("oxplow-bundled", "lenses/backlog-tasks.yaml"),
            ext_file!("oxplow-bundled", "lenses/change-co-change.yaml"),
            ext_file!("oxplow-bundled", "lenses/change-cross-zone-imports.yaml"),
            ext_file!("oxplow-bundled", "lenses/change-duplicates.yaml"),
            ext_file!("oxplow-bundled", "lenses/change-functions.yaml"),
            ext_file!("oxplow-bundled", "lenses/change-look-here.yaml"),
            ext_file!("oxplow-bundled", "lenses/change-review.yaml"),
            ext_file!("oxplow-bundled", "lenses/change-summary.yaml"),
            ext_file!("oxplow-bundled", "lenses/change-test-files.yaml"),
            ext_file!("oxplow-bundled", "lenses/change-treemap.yaml"),
            ext_file!("oxplow-bundled", "lenses/comments.yaml"),
            ext_file!("oxplow-bundled", "lenses/context-read.yaml"),
            ext_file!("oxplow-bundled", "lenses/decisions.yaml"),
            ext_file!("oxplow-bundled", "lenses/duplicate-blocks.yaml"),
            ext_file!("oxplow-bundled", "lenses/effort-analysis-findings.yaml"),
            ext_file!("oxplow-bundled", "lenses/effort-coverage.yaml"),
            ext_file!("oxplow-bundled", "lenses/effort-failed-tests.yaml"),
            ext_file!("oxplow-bundled", "lenses/effort-metric-deltas.yaml"),
            ext_file!("oxplow-bundled", "lenses/effort-nudges.yaml"),
            ext_file!("oxplow-bundled", "lenses/effort-plan.yaml"),
            ext_file!("oxplow-bundled", "lenses/effort-test-runs.yaml"),
            ext_file!("oxplow-bundled", "lenses/effort-tests.yaml"),
            ext_file!("oxplow-bundled", "lenses/effort-untested-files.yaml"),
            ext_file!("oxplow-bundled", "lenses/effort-verdict.yaml"),
            ext_file!("oxplow-bundled", "lenses/file-co-change.yaml"),
            ext_file!("oxplow-bundled", "lenses/findings.yaml"),
            ext_file!("oxplow-bundled", "lenses/go-to-bookmarks.yaml"),
            ext_file!("oxplow-bundled", "lenses/go-to.yaml"),
            ext_file!("oxplow-bundled", "lenses/history-graph.yaml"),
            ext_file!("oxplow-bundled", "lenses/inferred-decisions.yaml"),
            ext_file!("oxplow-bundled", "lenses/interface-command-errors.yaml"),
            ext_file!("oxplow-bundled", "lenses/interface-commands.yaml"),
            ext_file!("oxplow-bundled", "lenses/interface-hints.yaml"),
            ext_file!("oxplow-bundled", "lenses/interface-tool-calls.yaml"),
            ext_file!("oxplow-bundled", "lenses/job-run-output.yaml"),
            ext_file!("oxplow-bundled", "lenses/job-runs.yaml"),
            ext_file!("oxplow-bundled", "lenses/page-visits-by-day.yaml"),
            ext_file!("oxplow-bundled", "lenses/planning.yaml"),
            ext_file!("oxplow-bundled", "lenses/quality.yaml"),
            ext_file!("oxplow-bundled", "lenses/ready-tasks.yaml"),
            ext_file!("oxplow-bundled", "lenses/recent-decisions.yaml"),
            ext_file!("oxplow-bundled", "lenses/recent-notes.yaml"),
            ext_file!("oxplow-bundled", "lenses/recent-snapshots.yaml"),
            ext_file!("oxplow-bundled", "lenses/review-prompt.yaml"),
            ext_file!("oxplow-bundled", "lenses/review.yaml"),
            ext_file!("oxplow-bundled", "lenses/extension-runs.yaml"),
            ext_file!("oxplow-bundled", "lenses/runs.yaml"),
            ext_file!("oxplow-bundled", "lenses/session-files.yaml"),
            ext_file!("oxplow-bundled", "lenses/struggled.yaml"),
            ext_file!("oxplow-bundled", "lenses/task-token-summary.yaml"),
            ext_file!("oxplow-bundled", "lenses/task-tokens.yaml"),
            ext_file!("oxplow-bundled", "lenses/task-turns.yaml"),
            ext_file!("oxplow-bundled", "lenses/tests-weakened.yaml"),
            ext_file!("oxplow-bundled", "lenses/item-shared-work.yaml"),
            ext_file!("oxplow-bundled", "lenses/shared-work.yaml"),
            ext_file!("oxplow-bundled", "lenses/thread-activity.yaml"),
            ext_file!("oxplow-bundled", "lenses/thread-shared-work.yaml"),
            ext_file!("oxplow-bundled", "lenses/thread-plan.yaml"),
            ext_file!("oxplow-bundled", "lenses/thread-tokens.yaml"),
            ext_file!("oxplow-bundled", "lenses/token-total.yaml"),
            ext_file!("oxplow-bundled", "lenses/tokens-by-agent.yaml"),
            ext_file!("oxplow-bundled", "lenses/tokens-by-day.yaml"),
            ext_file!("oxplow-bundled", "lenses/top-pages.yaml"),
            ext_file!("oxplow-bundled", "lenses/unbacked-claims.yaml"),
            ext_file!("oxplow-bundled", "lenses/uncommitted-count.yaml"),
            ext_file!("oxplow-bundled", "lenses/uncommitted-line.yaml"),
            ext_file!("oxplow-bundled", "lenses/uncommitted.yaml"),
            ext_file!("oxplow-bundled", "lenses/unverified-claims.yaml"),
            ext_file!("oxplow-bundled", "lenses/usage.yaml"),
            ext_file!("oxplow-bundled", "lenses/verify-claim-with-evidence.yaml"),
            ext_file!("oxplow-bundled", "lenses/waiting-on-me.yaml"),
            ext_file!("oxplow-bundled", "lenses/what-deviated.yaml"),
            ext_file!("oxplow-bundled", "lenses/work-count.yaml"),
            ext_file!("oxplow-bundled", "lenses/work-line.yaml"),
            ext_file!("oxplow-bundled", "lenses/work.yaml"),
            ext_file!("oxplow-bundled", "models/change_co_change.sql"),
            ext_file!("oxplow-bundled", "models/change_interest.sql"),
            ext_file!("oxplow-bundled", "models/claim_status.sql"),
            ext_file!("oxplow-bundled", "models/co_change_pair.sql"),
            ext_file!("oxplow-bundled", "models/deviation.sql"),
            ext_file!("oxplow-bundled", "models/finished_cleared.sql"),
            ext_file!("oxplow-bundled", "models/struggle.sql"),
            ext_file!("oxplow-bundled", "models/thread_work.sql"),
            ext_file!("oxplow-bundled", "models/verdict.sql"),
            ext_file!("oxplow-bundled", "models/verdicts.sql"),
            ext_file!("oxplow-bundled", "questions.yaml"),
            ext_file!("oxplow-bundled", "skills/work-items/SKILL.md"),
        ],
    },
];

pub fn find(name: &str) -> Option<&'static BundledExtension> {
    BUNDLED.iter().find(|b| b.name == name)
}

/// Whether oxplow relies on extension `name` (it can't be disabled).
pub fn is_required(name: &str) -> bool {
    find(name).is_some_and(|b| b.required)
}

pub fn is_reserved(name: &str) -> bool {
    find(name).is_some()
}

#[cfg(test)]
mod tests {
    /// Adding a file to a bundled extension's folder without listing it
    /// above would silently drop it; keep the list and the folder in sync.
    #[test]
    fn bundled_file_lists_match_the_repo_folders() {
        for b in super::BUNDLED {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../extensions")
                .join(b.name);
            let mut on_disk: Vec<String> = walkdir::WalkDir::new(&dir)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_file())
                .map(|e| {
                    e.path()
                        .strip_prefix(&dir)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            on_disk.sort();
            let mut listed: Vec<String> = b.files.iter().map(|(p, _)| p.to_string()).collect();
            listed.sort();
            assert_eq!(
                listed, on_disk,
                "extensions/{} files vs BUNDLED list",
                b.name
            );
        }
    }

    /// A lens that needs a param nobody fills in from the launcher (an
    /// `effort_id` a slot passes) opens empty there, so it's hidden; the
    /// viewer's `stream_id` / `thread_id` are filled in (tsk374).
    #[test]
    fn launcher_lenses_need_no_slot_params() {
        let exts = crate::extensions::load_extensions(tempfile::tempdir().unwrap().path());
        let mut shown = Vec::new();
        for e in exts.iter().filter(|e| e.origin == "bundled") {
            assert!(e.errors.is_empty(), "{}: {:?}", e.name, e.errors);
            for l in e.lenses.iter().filter(|l| !l.hidden) {
                for p in &l.params {
                    assert!(
                        p.default.is_some()
                            || ["stream_id", "thread_id"].contains(&p.name.as_str()),
                        "{} is in the launcher but needs `{}`",
                        l.id,
                        p.name
                    );
                }
                shown.push(l.id.clone());
            }
        }
        // The review starters are there to open.
        for id in [
            "oxplow-bundled/waiting-on-me",
            "oxplow-bundled/recent-decisions",
            "oxplow-bundled/unbacked-claims",
        ] {
            assert!(shown.contains(&id.to_string()), "{id}: {shown:?}");
        }
    }

    /// The analytics effort-review lenses unnest the stored observation
    /// payloads (JUnit cases, uncovered lines) in SQL; run them on real rows.
    #[tokio::test]
    async fn analytics_effort_lenses_read_the_observation_payloads() {
        let f = crate::test_fixtures::services_with_effort().await;
        let obs = |kind: &str, value: Option<f64>, payload: serde_json::Value| {
            oxplow_db::EffortObservation {
                kind: kind.into(),
                provenance: "observed".into(),
                source: "post-tool-bash".into(),
                metric_value: value,
                payload_json: Some(payload.to_string()),
                local_snapshot_id: None,
                created_at: oxplow_domain::Timestamp::now(),
            }
        };
        let run = |failing: &str| {
            serde_json::json!({
                "command": "bun test", "passed": 1, "failed": 1,
                "suites": [{"name": "s", "cases": [
                    {"classname": "a", "name": "red", "status": failing},
                    {"classname": "a", "name": "ok", "status": "passed"}
                ]}]
            })
        };
        f.svc
            .effort_evidence_store
            .replace(
                f.effort.value(),
                Vec::new(),
                vec![
                    obs("test-run", None, run("failed")),
                    obs("test-run", None, run("passed")),
                    obs(
                        "diff-coverage",
                        Some(60.0),
                        serde_json::json!({
                            "summaryPct": 60.0, "changedLines": 10, "coveredLines": 6,
                            "files": [
                                {"path": "src/a.rs", "uncoveredChangedLines": [4, 9, 12]},
                                {"path": "src/b.rs", "uncoveredChangedLines": []},
                                {"path": "src/c.rs", "uncoveredChangedLines": [2]}
                            ]
                        }),
                    ),
                ],
                String::new(),
            )
            .await
            .unwrap();
        let lens = |slug: &'static str| run_analytics_lens(&f, slug, "effort_id", f.effort.value());
        assert_eq!(
            lens("effort-coverage").await,
            serde_json::json!([["**60%** of changed lines covered · 6/10"]])
        );
        assert_eq!(
            lens("effort-untested-files").await,
            serde_json::json!([["src/a.rs", 3, 4, "4, 9, 12"], ["src/c.rs", 1, 2, "2"]])
        );
        assert_eq!(
            lens("effort-failed-tests").await,
            serde_json::json!([["s", "a › red", 1, "passed"]])
        );
        let runs = lens("effort-test-runs").await;
        assert_eq!(runs.as_array().unwrap().len(), 2);
    }

    /// A test run on `effort` at `at` (a `tests` capture with its
    /// `test-detail` payload, what `v_test_run` reads): `failed` of
    /// `total` cases failing.
    async fn record_test_run(
        f: &crate::test_fixtures::EffortFixture,
        effort: i64,
        failed: i64,
        total: i64,
        at: &str,
    ) {
        let (thread, at) = (f.thread.value(), at.to_string());
        f.svc
            .db
            .transaction(move |c| {
                let detail = format!(
                    "{{\"kind\":\"test-detail\",\"payload\":{{\"passed\":{},\"failed\":{failed},\"total\":{total}}}}}",
                    total - failed
                );
                c.execute(
                    "INSERT INTO metric_capture (stream_id, thread_id, effort_id, producer, status, provenance, source, captured_at, detail_json)
                     SELECT stream_id, id, ?2, 'tests', 'done', 'observed', 'junit', ?3, ?4 FROM threads WHERE id = ?1",
                    rusqlite::params![thread, effort, at, detail],
                )
                .map_err(oxplow_db::map_sql_err)?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// Another closed effort on the fixture's thread; its id.
    async fn another_effort(f: &crate::test_fixtures::EffortFixture, day: &str) -> i64 {
        let (thread, day) = (f.thread.value(), day.to_string());
        f.svc
            .db
            .transaction(move |c| {
                c.execute(
                    "INSERT INTO effort (thread_id, started_at, ended_at) VALUES (?1, ?2, ?2)",
                    rusqlite::params![thread, day],
                )
                .map_err(oxplow_db::map_sql_err)?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    /// A claim on `effort`, citing `evidence`.
    async fn record_claim(
        f: &crate::test_fixtures::EffortFixture,
        effort: i64,
        kind: &str,
        evidence: Option<&str>,
    ) {
        let claim = oxplow_db::NewClaim {
            thread_id: f.thread.value(),
            work_item: None,
            effort_id: Some(effort),
            statement: format!("{kind} on effort {effort}"),
            kind: kind.into(),
            evidence_ref: evidence.map(str::to_string),
            agent_session_id: None,
        };
        f.svc
            .db
            .transaction(move |tx| {
                oxplow_db::insert_claim_tx(tx, oxplow_db::ClaimProvenance::Inferred, &claim)
            })
            .await
            .unwrap();
    }

    /// oxplow-bundled's struggle rule: a file edited five times, or three
    /// failed shell commands, in an effort. Four edits and two failures
    /// are not struggle; a shell call with no known outcome is no failure.
    #[tokio::test(flavor = "multi_thread")]
    async fn struggle_is_five_edits_of_a_file_or_three_failed_commands() {
        let f = crate::test_fixtures::services_with_effort().await;
        let call = |kind: &str, path: Option<&str>, ok: Option<bool>| oxplow_db::NewToolCall {
            thread_id: f.thread.value(),
            effort_id: Some(f.effort.value()),
            tool: kind.into(),
            kind: kind.into(),
            path: path.map(str::to_string),
            ok,
            ..Default::default()
        };
        let store = &f.svc.tool_call_store;
        for _ in 0..5 {
            store
                .record(call("edit", Some("src/hot.rs"), Some(true)))
                .await
                .unwrap();
        }
        for _ in 0..4 {
            store
                .record(call("edit", Some("src/calm.rs"), Some(true)))
                .await
                .unwrap();
        }
        for _ in 0..3 {
            store
                .record(call("shell", None, Some(false)))
                .await
                .unwrap();
        }
        store.record(call("shell", None, None)).await.unwrap();
        let rows = |sql: String| {
            let svc = f.svc.clone();
            async move {
                serde_json::to_value(svc.sql.query_sql(&sql, vec![], None).await.unwrap().rows)
                    .unwrap()
            }
        };
        assert_eq!(
            rows("SELECT kind, subject, count FROM v_oxplow_bundled_struggle ORDER BY kind".into())
                .await,
            serde_json::json!([
                ["failed_commands", "shell", 3],
                ["repeated_edits", "src/hot.rs", 5]
            ])
        );
        // One fewer of each, and nothing is struggle.
        let g = crate::test_fixtures::services_with_effort().await;
        let few = |kind: &str, path: Option<&str>, ok| oxplow_db::NewToolCall {
            thread_id: g.thread.value(),
            effort_id: Some(g.effort.value()),
            tool: kind.into(),
            kind: kind.into(),
            path: path.map(str::to_string),
            ok,
            ..Default::default()
        };
        for _ in 0..4 {
            g.svc
                .tool_call_store
                .record(few("edit", Some("a.rs"), Some(true)))
                .await
                .unwrap();
        }
        for _ in 0..2 {
            g.svc
                .tool_call_store
                .record(few("shell", None, Some(false)))
                .await
                .unwrap();
        }
        let none = g
            .svc
            .sql
            .query_sql(
                "SELECT count(*) FROM v_oxplow_bundled_struggle",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(none.rows).unwrap(),
            serde_json::json!([[0]])
        );
    }

    /// oxplow-bundled's rule for a backed claim: it cites evidence, or it
    /// says the tests pass and its effort's LATEST run passed with at
    /// least one test (its own or one claimed through attribution).
    #[tokio::test]
    async fn a_claim_is_backed_by_evidence_or_its_efforts_latest_clean_run() {
        let f = crate::test_fixtures::services_with_effort().await;
        let fixed = f.effort.value();
        let broke = another_effort(&f, "2026-01-01T00:00:00Z").await;
        let empty = another_effort(&f, "2026-01-02T00:00:00Z").await;
        // Passed, then a later failure: not backed.
        record_test_run(&f, broke, 0, 5, "2026-01-01T00:00:00Z").await;
        record_test_run(&f, broke, 1, 5, "2026-01-01T01:00:00Z").await;
        // Failed, then fixed: backed.
        record_test_run(&f, fixed, 2, 5, "2026-01-02T00:00:00Z").await;
        record_test_run(&f, fixed, 0, 5, "2026-01-02T01:00:00Z").await;
        // A run with no tests: not backed.
        record_test_run(&f, empty, 0, 0, "2026-01-03T00:00:00Z").await;
        for effort in [broke, fixed, empty] {
            record_claim(&f, effort, "tests_pass", None).await;
        }
        record_claim(&f, broke, "handles_case", Some("test:empty_input")).await;
        record_claim(&f, fixed, "no_behavior_change", None).await;
        let out = f
            .svc
            .sql
            .query_sql(
                "SELECT c.kind, c.effort_id, s.verified, s.backed_by
                 FROM v_claim c JOIN v_oxplow_bundled_claim_status s ON s.ref = c.ref
                 ORDER BY c.id",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            serde_json::json!([
                ["tests_pass", broke, 0, null],
                ["tests_pass", fixed, 1, "latest run"],
                ["tests_pass", empty, 0, null],
                ["handles_case", broke, 1, "evidence"],
                ["no_behavior_change", fixed, 0, null],
            ])
        );
    }

    /// A reviewer's verdict on a claim moves it in the review policy: a
    /// verified one is backed (even one that cites nothing), and a
    /// dismissed one counts for nothing: neither unbacked nor backed, so it
    /// is not in `claim_status` at all.
    #[tokio::test]
    async fn claim_status_counts_verified_claims_and_leaves_out_dismissed_ones() {
        let f = crate::test_fixtures::services_with_effort().await;
        let effort = f.effort.value();
        for kind in ["handles_case", "no_behavior_change", "other"] {
            record_claim(&f, effort, kind, None).await;
        }
        // The first is verified (no evidence cited), the second dismissed.
        f.svc
            .db
            .transaction(|c| {
                c.execute(
                    "UPDATE claim SET review = CASE kind WHEN 'handles_case' THEN 'verified' \
                     WHEN 'no_behavior_change' THEN 'dismissed' END",
                    (),
                )
                .map_err(oxplow_db::map_sql_err)?;
                Ok(())
            })
            .await
            .unwrap();
        let out = f
            .svc
            .sql
            .query_sql(
                "SELECT c.kind, s.verified, s.backed_by
                 FROM v_claim c LEFT JOIN v_oxplow_bundled_claim_status s ON s.ref = c.ref
                 ORDER BY c.id",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            serde_json::json!([
                ["handles_case", 1, "evidence"],
                ["no_behavior_change", null, null],
                ["other", 0, null],
            ])
        );
        // The verdict counts only the one still unverified.
        let verdict = run_analytics_lens(&f, "effort-verdict", "effort_id", effort).await;
        assert!(
            verdict.to_string().contains("1 unverified claim"),
            "{verdict}"
        );
    }

    /// oxplow-bundled's verdict, the first line of an effort's page: did it
    /// test, how much of the change ran (judged against the coverage
    /// metric's `warnAt`), and what's left to check.
    #[tokio::test]
    async fn the_effort_verdict_says_what_a_reviewer_needs_first() {
        let f = crate::test_fixtures::services_with_effort().await;
        let effort = f.effort.value();
        let verdict = || run_analytics_lens(&f, "effort-verdict", "effort_id", effort);
        assert_eq!(
            verdict().await,
            serde_json::json!([
                ["Tests: none ran", "warning"],
                ["Diff coverage: not measured", null],
                ["Every claim is backed", "success"],
                ["0 decisions made", null],
            ])
        );
        let obs = |kind: &str, value: Option<f64>, payload: serde_json::Value| {
            oxplow_db::EffortObservation {
                kind: kind.into(),
                provenance: "observed".into(),
                source: "post-tool-bash".into(),
                metric_value: value,
                payload_json: Some(payload.to_string()),
                local_snapshot_id: None,
                created_at: oxplow_domain::Timestamp::now(),
            }
        };
        let evidence = |runs: Vec<(i64, i64)>, coverage: f64| {
            let mut rows: Vec<oxplow_db::EffortObservation> = runs
                .into_iter()
                .map(|(passed, failed)| {
                    obs(
                        "test-run",
                        None,
                        serde_json::json!({ "passed": passed, "failed": failed }),
                    )
                })
                .collect();
            rows.push(obs(
                "diff-coverage",
                Some(coverage),
                serde_json::json!({ "summaryPct": coverage }),
            ));
            rows
        };
        let replace = |rows: Vec<oxplow_db::EffortObservation>| {
            f.svc
                .effort_evidence_store
                .replace(effort, Vec::new(), rows, String::new())
        };
        // The last run failed, and coverage is under the threshold (80).
        replace(evidence(vec![(4, 0), (3, 1)], 79.6)).await.unwrap();
        record_claim(&f, effort, "tests_pass", None).await;
        f.svc
            .reasoning_store
            .replace_inferred(
                effort,
                vec![oxplow_db::NewDecision {
                    thread_id: f.thread.value(),
                    work_item: None,
                    effort_id: Some(effort),
                    question: "q".into(),
                    choice: "c".into(),
                    alternatives: vec![],
                    confidence: "medium".into(),
                    why: "w".into(),
                }],
            )
            .await
            .unwrap();
        assert_eq!(
            verdict().await,
            serde_json::json!([
                ["Tests: the last run failed (1 of 4)", "warning"],
                ["Diff coverage: 80%", "warning"],
                ["1 unverified claim", "warning"],
                ["1 decision to confirm", "warning"],
            ])
        );
        // Green, and exactly at the threshold: a threshold is crossed only past it.
        replace(evidence(vec![(3, 1), (4, 0)], 80.0)).await.unwrap();
        assert_eq!(
            verdict().await,
            serde_json::json!([
                ["Tests: the last of 2 runs passed (4)", "success"],
                ["Diff coverage: 80%", "success"],
                ["1 unverified claim", "warning"],
                ["1 decision to confirm", "warning"],
            ])
        );
    }

    /// P4.1 (tsk486): every query a bundled extension ships — lens,
    /// advisory, collector or command input — reads only published views
    /// (its own models' included), never a physical table (what the
    /// authorizer will refuse, P4.3).
    #[tokio::test]
    async fn bundled_queries_read_only_published_views() {
        let f = crate::test_fixtures::services_with_effort().await;
        crate::test_fixtures::activate_with_problems(&f.svc).await;
        let bundled: Vec<&str> = super::BUNDLED.iter().map(|b| b.name).collect();
        let mut checked = 0;
        for ext in f
            .svc
            .extension_catalog
            .unpinned(f._dir.path(), "a test reads declarations")
            .iter()
        {
            if !bundled.contains(&ext.name.as_str()) {
                continue;
            }
            let queries = ext
                .lenses
                .iter()
                // A grid or a form reads nothing of its own.
                .filter(|l| !l.query.trim().is_empty())
                .map(|l| (format!("lens {}", l.id), l.query.clone()))
                .chain(
                    ext.advisories
                        .iter()
                        .map(|a| (format!("advisory {}", a.id), a.query.clone())),
                )
                .chain(ext.collectors.iter().filter_map(|s| {
                    s.input
                        .as_ref()
                        .map(|q| (format!("collector {} input", s.id), q.sql.clone()))
                }));
            for (what, sql) in queries {
                // Its own settings, at their defaults.
                let sql = oxplow_db::models::substitute_config(
                    &sql,
                    ext.default_configuration().as_ref(),
                )
                .unwrap_or_else(|e| panic!("{}/{what}: {e}", ext.name));
                let reads = f
                    .svc
                    .sql
                    .check(&sql)
                    .await
                    .unwrap_or_else(|e| panic!("{}/{what}: {e}", ext.name));
                let physical: Vec<&String> = reads
                    .tables
                    .iter()
                    .filter(|t| !t.starts_with("temp."))
                    .collect();
                assert!(
                    physical.is_empty(),
                    "{}/{what} reads {physical:?}",
                    ext.name
                );
                checked += 1;
            }
        }
        assert!(checked > 30, "only {checked} bundled queries found");
    }

    /// Run an oxplow-bundled lens with one integer param; its rows as JSON.
    async fn run_analytics_lens(
        f: &crate::test_fixtures::EffortFixture,
        slug: &str,
        param: &str,
        value: i64,
    ) -> serde_json::Value {
        run_bundled_lens(f, &format!("oxplow-bundled/{slug}"), &[(param, value)]).await
    }

    /// Run any bundled lens (`<extension>/<slug>`) with integer params; its
    /// rows as JSON.
    async fn run_bundled_lens(
        f: &crate::test_fixtures::EffortFixture,
        id: &str,
        params: &[(&str, i64)],
    ) -> serde_json::Value {
        let params: Vec<(&str, oxplow_db::SqlCell)> = params
            .iter()
            .map(|(k, v)| (*k, oxplow_db::SqlCell::Int(*v)))
            .collect();
        run_bundled_lens_with(f, id, &params).await
    }

    /// `run_bundled_lens`, with any param values.
    async fn run_bundled_lens_with(
        f: &crate::test_fixtures::EffortFixture,
        id: &str,
        params: &[(&str, oxplow_db::SqlCell)],
    ) -> serde_json::Value {
        let layer = crate::sql_gateway::SqlGateway::new(f.svc.db.clone());
        let params = params
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        let run = crate::extensions::run_lens(
            &layer,
            &f.svc.extension_catalog,
            f._dir.path(),
            id,
            params,
            &crate::extensions::LensContext::default(),
        )
        .await
        .unwrap();
        serde_json::to_value(&run.result.rows).unwrap()
    }

    /// The `runs` lens: per extension, the last week's runs, failures,
    /// denials, mean CPU and model calls, the ones needing attention first.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_runs_lens_counts_each_extensions_week() {
        let f = crate::test_fixtures::services_with_effort().await;
        // (extension, outcome, days ago, cpu ms, model calls)
        let runs: [(&str, &str, i64, i64, i64); 6] = [
            ("acme", "ok", 1, 10, 0),
            ("acme", "failed", 2, 30, 2),
            ("acme", "denied", 3, 20, 0),
            ("acme", "ok", 8, 999, 9), // a week and a day ago: out
            ("beta", "ok", 0, 5, 0),
            ("beta", "ok", 4, 7, 0),
        ];
        f.svc
            .db
            .transaction(move |tx| {
                for (i, (ext, outcome, days, cpu, ai)) in runs.iter().enumerate() {
                    tx.execute(
                        "INSERT INTO extension_run
                           (run_id, extension, contribution, kind, actor_kind, actor, mode,
                            started_at, finished_at, outcome, wall_ms, cpu_ms, peak_bytes,
                            ai_calls, scopes_json, decisions_json)
                         VALUES (?1, ?2, ?3, 'collector', 'system', 'system', 'external',
                            strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-' || ?4 || ' days'),
                            strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-' || ?4 || ' days'),
                            ?5, 1, ?6, 0, ?7, '{}', '[]')",
                        rusqlite::params![
                            format!("run{i}"),
                            ext,
                            format!("collector:{ext}/pull"),
                            days,
                            outcome,
                            cpu,
                            ai
                        ],
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                }
                Ok(())
            })
            .await
            .unwrap();
        let rows = run_bundled_lens(&f, "oxplow-bundled/runs", &[]).await;
        // Each extension links to its runs, one by one.
        let runs_of = |ext: &str| format!("lens:oxplow-bundled/extension-runs?extension={ext}");
        assert_eq!(
            rows,
            serde_json::json!([
                ["acme", runs_of("acme"), 3, 1, 1, 20.0, 2],
                ["beta", runs_of("beta"), 2, 0, 0, 6.0, 0]
            ])
        );
        let detail = run_bundled_lens_with(
            &f,
            "oxplow-bundled/extension-runs",
            &[("extension", oxplow_db::SqlCell::Text("acme".into()))],
        )
        .await;
        let outcomes: Vec<_> = detail
            .as_array()
            .unwrap()
            .iter()
            .map(|row| (row[0].clone(), row[6].clone()))
            .collect();
        // Newest first, the week and a day old run out.
        assert_eq!(
            outcomes,
            vec![
                (
                    serde_json::json!("collector:acme/pull"),
                    serde_json::json!("ok")
                ),
                (
                    serde_json::json!("collector:acme/pull"),
                    serde_json::json!("failed")
                ),
                (
                    serde_json::json!("collector:acme/pull"),
                    serde_json::json!("denied")
                ),
            ]
        );
    }

    /// The agent-interface lenses over recorded usage: tool calls by tool
    /// and harness, an agent's command runs with the share refused and the
    /// most common errors, and the hints' counts.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_agent_interface_lenses_count_tools_commands_and_hints() {
        let f = crate::test_fixtures::services_with_effort().await;
        let thread = f.thread.value();
        f.svc
            .db
            .transaction(move |tx| {
                let run = |sql: &str, params: &[&dyn rusqlite::ToSql]| {
                    tx.execute(sql, params).map_err(oxplow_db::map_sql_err)
                };
                for (id, harness) in [(11, "claude"), (12, "codex")] {
                    run(
                        "INSERT INTO agent_session (id, thread_id, kind, harness, opened_at, updated_at)
                         VALUES (?1, ?2, 'terminal', ?3, 'n', 'n')",
                        &[&id, &thread, &harness],
                    )?;
                }
                // (tool, kind, session, ok, days ago)
                let calls: [(&str, &str, Option<i64>, i64, i64); 7] = [
                    ("mcp__oxplow__run_command", "mcp", Some(11), 1, 1),
                    ("mcp__oxplow__run_command", "mcp", Some(11), 0, 2),
                    ("mcp__oxplow__run_command", "mcp", Some(12), 1, 2),
                    ("mcp__oxplow__oxplow_query_sql", "mcp", None, 1, 3),
                    ("mcp__oxplow__oxplow_query_sql", "mcp", Some(11), 1, 40), // out of the window
                    ("Bash", "shell", Some(11), 1, 1),                  // not an oxplow tool
                    ("mcp__other__thing", "mcp", Some(12), 1, 1),
                ];
                for (tool, kind, session, ok, days) in calls {
                    run(
                        "INSERT INTO agent_tool_call (thread_id, tool, kind, agent_session_id, ok, at)
                         VALUES (?1, ?2, ?3, ?4, ?5,
                                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-' || ?6 || ' days'))",
                        &[&thread, &tool, &kind, &session, &ok, &days],
                    )?;
                }
                // (command, actor, outcome, error)
                let audits: [(&str, &str, &str, Option<&str>); 8] = [
                    ("oxplow.work_item.create", "agent", "ok", None),
                    ("oxplow.work_item.transition", "agent", "ok", None),
                    (
                        "oxplow.work_item.transition",
                        "agent",
                        "invalid",
                        Some("invalid input at `/state`: \"ready\" is not valid under any of the schemas listed in the 'anyOf' keyword. Expected one of: \"todo\", \"done\""),
                    ),
                    (
                        "oxplow.work_item.transition",
                        "agent",
                        "invalid",
                        Some("invalid input at `/state`: \"ready\" is not valid under any of the schemas listed in the 'anyOf' keyword"),
                    ),
                    ("oxplow.work_item.transition", "agent", "denied", Some("not allowed")),
                    ("oxplow.work_item.create", "agent", "error", Some("boom")),
                    ("oxplow.work_item.create", "human", "invalid", Some("a person's, not counted")),
                    ("oxplow.lens.keep", "agent", "ok", None),
                ];
                for (command, actor, outcome, error) in audits {
                    run(
                        "INSERT INTO command_audit (at, command, actor_kind, input_json, outcome, error)
                         VALUES (strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-1 days'), ?1, ?2, '{}', ?3, ?4)",
                        &[&command, &actor, &outcome, &error],
                    )?;
                }
                // hint x/never ran 120 times and never spoke; x/spoke spoke once.
                for (hint, evaluated) in [("x/never", 120), ("x/spoke", 40)] {
                    run(
                        "INSERT INTO hint_stat (thread_id, hint, evaluated, last_evaluated_at)
                         VALUES (?1, ?2, ?3, 'n')",
                        &[&thread, &hint, &evaluated],
                    )?;
                }
                run(
                    "INSERT INTO agent_nudge (thread_id, kind, message, created_at, delivered_at)
                     VALUES (?1, 'x/spoke', 'm', 'n', 'n')",
                    &[&thread],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        let tools = run_bundled_lens(&f, "oxplow-bundled/interface-tool-calls", &[]).await;
        assert_eq!(
            tools,
            serde_json::json!([
                ["run_command", "claude", 2, 1],
                ["oxplow_query_sql", "unknown", 1, 0],
                ["run_command", "codex", 1, 0],
            ])
        );

        let commands = run_bundled_lens(&f, "oxplow-bundled/interface-commands", &[]).await;
        let errors_of =
            |c: &str| format!("lens:oxplow-bundled/interface-command-errors?command={c}");
        assert_eq!(
            commands,
            serde_json::json!([
                [
                    "oxplow.work_item.transition",
                    errors_of("oxplow.work_item.transition"),
                    4,
                    2,
                    1,
                    0,
                    75.0
                ],
                [
                    "oxplow.work_item.create",
                    errors_of("oxplow.work_item.create"),
                    2,
                    0,
                    0,
                    1,
                    50.0
                ],
                [
                    "oxplow.lens.keep",
                    errors_of("oxplow.lens.keep"),
                    1,
                    0,
                    0,
                    0,
                    0.0
                ],
            ])
        );

        // The same text counts together however the choices were listed.
        let errors = run_bundled_lens_with(
            &f,
            "oxplow-bundled/interface-command-errors",
            &[(
                "command",
                oxplow_db::SqlCell::Text("oxplow.work_item.transition".into()),
            )],
        )
        .await;
        let top: Vec<_> = errors
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r[0].clone(), r[1].clone(), r[2].clone()))
            .collect();
        assert_eq!(
            top,
            vec![
                (
                    serde_json::json!("invalid"),
                    serde_json::json!("invalid input at `/state`: \"ready\" is not valid under any of the schemas listed in the 'anyOf' keyword"),
                    serde_json::json!(2)
                ),
                (
                    serde_json::json!("denied"),
                    serde_json::json!("not allowed"),
                    serde_json::json!(1)
                ),
            ]
        );

        // A hint that never spoke leads the list.
        let hints = run_bundled_lens(&f, "oxplow-bundled/interface-hints", &[]).await;
        let counts: Vec<_> = hints
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r[0].clone(), r[1].clone(), r[2].clone(), r[3].clone()))
            .collect();
        assert_eq!(
            counts,
            vec![
                (
                    serde_json::json!("x/never"),
                    serde_json::json!(120),
                    serde_json::json!(0),
                    serde_json::json!(0)
                ),
                (
                    serde_json::json!("x/spoke"),
                    serde_json::json!(40),
                    serde_json::json!(1),
                    serde_json::json!(1)
                ),
            ]
        );
    }

    /// Token usage moved from the task/thread widgets to slot lenses.
    #[tokio::test]
    async fn analytics_token_lenses_sum_a_tasks_and_a_threads_turns() {
        let f = crate::test_fixtures::services_with_task_effort().await;
        let turn =
            |effort: Option<String>, prompt: &str, tokens: i64| oxplow_db::NewAgentTokenUsage {
                stream_id: "str1".into(),
                thread_id: f.thread.to_string(),
                effort_id: effort,
                session_id: "s".into(),
                agent_kind: "claude".into(),
                model: Some("m".into()),
                prompt: Some(prompt.into()),
                input_tokens: tokens,
                output_tokens: 1,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
                message_count: 1,
                ..Default::default()
            };
        let store = &f.svc.token_usage_store;
        store
            .record(turn(Some(f.effort.to_string()), "fix it", 1999))
            .await
            .unwrap();
        store
            .record(turn(Some(f.effort.to_string()), "again", 99))
            .await
            .unwrap();
        store.record(turn(None, "chat", 9)).await.unwrap();

        let item = oxplow_tasks::work_item_ref(f.task);
        assert_eq!(
            run_bundled_lens_with(
                &f,
                "oxplow-bundled/task-token-summary",
                &[("ref", oxplow_db::SqlCell::Text(item.clone()))],
            )
            .await,
            serde_json::json!([[
                "**2,100** tokens · 2 turns · in 2,098 · out 2 · cache-write 0 · cache-read 0"
            ]])
        );
        let turns = run_bundled_lens_with(
            &f,
            "oxplow-bundled/task-turns",
            &[("ref", oxplow_db::SqlCell::Text(item.clone()))],
        )
        .await;
        assert_eq!(turns.as_array().unwrap().len(), 2);
        assert_eq!(
            run_analytics_lens(&f, "thread-tokens", "thread_id", f.thread.value()).await,
            serde_json::json!([[2110]])
        );
        assert_eq!(
            run_analytics_lens(&f, "thread-tokens", "thread_id", 999).await,
            serde_json::json!([]),
            "an idle thread shows nothing"
        );
    }

    /// The change-review grid reads a real analyzed commit.
    #[tokio::test]
    async fn analytics_change_lenses_read_an_analyzed_commit() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::write(
            root.join("src/lib.rs"),
            "fn grow(a: u32) -> u32 {\n    a\n}\n",
        )
        .unwrap();
        std::fs::write(root.join("tests/it.rs"), "fn t() {}\n").unwrap();
        crate::test_fixtures::commit_all(&root, "base");
        std::fs::write(
            root.join("src/lib.rs"),
            "fn grow(a: u32, b: u32) -> u32 {\n    if a > b {\n        a\n    } else {\n        b\n    }\n}\n",
        )
        .unwrap();
        std::fs::write(root.join("tests/it.rs"), "fn t() {}\nfn u() {}\n").unwrap();
        let sha = crate::test_fixtures::commit_all(&root, "change");
        let change = crate::change_analysis::ensure_change(
            &f.svc,
            crate::change_analysis::ChangeTarget::Commit {
                sha,
                stream_id: None,
            },
        )
        .await
        .unwrap();

        let summary = run_analytics_lens(&f, "change-summary", "change_id", change.id).await;
        assert_eq!(
            summary,
            serde_json::json!([[
                "**2 files** · +7 −2 · 0 added, 2 modified, 0 deleted · 1 test file · test/code lines 12%"
            ]])
        );
        let functions = run_analytics_lens(&f, "change-functions", "change_id", change.id).await;
        assert_eq!(functions[0][1], serde_json::json!("grow"));
        assert_eq!(functions[0][3], serde_json::json!("signature"));
        assert_eq!(
            functions[0].as_array().unwrap().last().unwrap(),
            "symbol:src/lib.rs/grow"
        );
        let tests = run_analytics_lens(&f, "change-test-files", "change_id", change.id).await;
        assert_eq!(tests[0][0], serde_json::json!("tests/it.rs"));
    }

    /// Waiting on Me lists threads whose agent waits on the person (its
    /// final message asked them something) until a prompt moves it on.
    #[tokio::test]
    async fn waiting_on_me_lists_unanswered_questions() {
        let f = crate::test_fixtures::services_with_effort().await;
        let hook = |kind, payload: serde_json::Value| crate::hook_ingest::HookEnvelope {
            session_title: None,
            turn_run: None,
            kind,
            thread_id: Some(f.thread),
            stream_id: None,
            agent_session_id: None,
            session_id: None,
            payload_json: payload.to_string(),
            prompt: Some("go".into()),
            decision: None,
            tool: None,
            subagent: None,
            peer: None,
        };
        use oxplow_domain::hook::HookKind;
        f.svc
            .hook_ingest
            .ingest(hook(HookKind::UserPromptSubmit, serde_json::json!({})))
            .await
            .unwrap();
        f.svc
            .hook_ingest
            .ingest(hook(
                HookKind::Stop,
                serde_json::json!({ "last_assistant_message": "Pick A or B?" }),
            ))
            .await
            .unwrap();
        let waiting = |rows: serde_json::Value| {
            rows.as_array()
                .unwrap()
                .iter()
                .filter(|r| r[0] == "Waiting for your answer")
                .map(|r| r[1].clone())
                .collect::<Vec<_>>()
        };
        let rows = run_bundled_lens(&f, "oxplow-bundled/waiting-on-me", &[]).await;
        assert_eq!(waiting(rows), vec![serde_json::json!("Pick A or B?")]);

        // The person answers: a new turn starts.
        f.svc
            .hook_ingest
            .ingest(hook(HookKind::UserPromptSubmit, serde_json::json!({})))
            .await
            .unwrap();
        let rows = run_bundled_lens(&f, "oxplow-bundled/waiting-on-me", &[]).await;
        assert!(waiting(rows).is_empty());
    }

    /// Work handed to a thread with no agent session open there is the
    /// person's move: Waiting on Me lists it until an agent is there.
    #[tokio::test]
    async fn waiting_on_me_lists_handoffs_no_agent_is_there_for() {
        let f = crate::test_fixtures::services_with_effort().await;
        let to =
            crate::test_fixtures::new_thread(&f.svc, oxplow_domain::StreamId::new(1), "Docs").await;
        f.svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                crate::commands::handoff::SEND,
                serde_json::json!({
                    "from_thread": oxplow_domain::refs::build::thread_ref(f.thread),
                    "to_thread": oxplow_domain::refs::build::thread_ref(to.id),
                    "body": "the docs half is yours",
                }),
                false,
            )
            .await
            .unwrap();
        let handed = |rows: serde_json::Value| {
            rows.as_array()
                .unwrap()
                .iter()
                .filter(|r| r[0] == "Handed to a thread with no agent")
                .map(|r| (r[1].clone(), r[3].clone()))
                .collect::<Vec<_>>()
        };
        let rows = run_bundled_lens(&f, "oxplow-bundled/waiting-on-me", &[]).await;
        assert_eq!(
            handed(rows),
            vec![(
                serde_json::json!("the docs half is yours"),
                serde_json::json!("Docs")
            )]
        );
        crate::test_fixtures::open_session(&f.svc, to.id).await;
        let rows = run_bundled_lens(&f, "oxplow-bundled/waiting-on-me", &[]).await;
        assert!(handed(rows).is_empty());
    }

    /// What Deviated: the effort's files outside the area its task names.
    #[tokio::test]
    async fn what_deviated_lists_files_outside_the_tasks_stated_area() {
        use oxplow_db::EffortStore as _;
        use oxplow_tasks::TaskStore as _;
        let f = crate::test_fixtures::services_with_task_effort().await;
        crate::test_fixtures::activate_with_problems(&f.svc).await;
        let describe = |text: &'static str| {
            let svc = f.svc.clone();
            let task = f.task;
            async move {
                let mut t = svc.task_store.get(task).await.unwrap().unwrap();
                t.description = text.into();
                svc.task_store.update(&t).await.unwrap();
                crate::test_fixtures::restate_task(&svc, task).await;
            }
        };
        for path in ["src/ui/panel.ts", "src/ui/button.ts", "crates/db/store.rs"] {
            f.svc
                .effort_store
                .record_file(
                    &f.effort,
                    path,
                    oxplow_db::EffortFileChange::Updated,
                    oxplow_db::FileRefVersion {
                        local_snapshot_id: 0,
                        closest_vcs_rev: None,
                        vcs_rev_exact: false,
                    },
                )
                .await
                .unwrap();
        }
        let lens = "oxplow-bundled/what-deviated";
        let effort = [("effort_id", f.effort.value())];

        describe("Fix the hover state in [[src/ui/button.ts]].").await;
        let rows = run_bundled_lens(&f, lens, &effort).await;
        assert_eq!(rows, serde_json::json!([["crates/db/store.rs", "updated"]]));

        // A task that names no paths states no area, so nothing deviates.
        describe("Make the buttons feel snappier.").await;
        let rows = run_bundled_lens(&f, lens, &effort).await;
        assert_eq!(rows, serde_json::json!([]));
    }

    /// Tests Weakened: deleted tests, removed assertions and new skips.
    #[tokio::test]
    async fn tests_weakened_reports_deleted_tests_fewer_assertions_and_skips() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::write(
            root.join("tests/it.rs"),
            "#[test]\nfn a() {\n    assert_eq!(1, 1);\n    assert!(true);\n}\n\n#[test]\nfn b() {\n    assert!(true);\n}\n",
        )
        .unwrap();
        crate::test_fixtures::commit_all(&root, "base");
        std::fs::write(
            root.join("tests/it.rs"),
            "#[test]\n#[ignore]\nfn a() {\n    assert_eq!(1, 1);\n}\n",
        )
        .unwrap();
        let sha = crate::test_fixtures::commit_all(&root, "weaken");
        let change = crate::change_analysis::ensure_change(
            &f.svc,
            crate::change_analysis::ChangeTarget::Commit {
                sha,
                stream_id: None,
            },
        )
        .await
        .unwrap();
        let rows = run_bundled_lens(
            &f,
            "oxplow-bundled/tests-weakened",
            &[("change_id", change.id)],
        )
        .await;
        let got: Vec<(String, String, String)> = rows
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r[0].as_str().unwrap().to_string(),
                    r[1].as_str().unwrap().to_string(),
                    r[2].as_str().unwrap().to_string(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("Test deleted".into(), "tests/it.rs".into(), "b".into()),
                (
                    "Fewer assertions".into(),
                    "tests/it.rs".into(),
                    "3 → 1".into()
                ),
                (
                    "Tests skipped".into(),
                    "tests/it.rs".into(),
                    "0 → 1 skip marker".into()
                ),
            ]
        );
    }

    /// Copy Review Prompt: a prompt for reviewing the effort with another
    /// harness, built from the task, the files and the agent's claims.
    #[tokio::test]
    async fn review_prompt_names_the_task_and_what_changed() {
        use oxplow_db::EffortStore as _;
        use oxplow_tasks::TaskStore as _;
        let f = crate::test_fixtures::services_with_task_effort().await;
        let mut t = f.svc.task_store.get(f.task).await.unwrap().unwrap();
        t.title = "Fix the hover state".into();
        t.description = "Buttons flicker on hover.".into();
        f.svc.task_store.update(&t).await.unwrap();
        crate::test_fixtures::restate_task(&f.svc, f.task).await;
        f.svc
            .effort_store
            .record_file(
                &f.effort,
                "src/ui/button.ts",
                oxplow_db::EffortFileChange::Updated,
                oxplow_db::FileRefVersion {
                    local_snapshot_id: 0,
                    closest_vcs_rev: None,
                    vcs_rev_exact: false,
                },
            )
            .await
            .unwrap();
        let rows = run_bundled_lens(
            &f,
            "oxplow-bundled/review-prompt",
            &[("effort_id", f.effort.value())],
        )
        .await;
        let prompt = rows[0][0].as_str().unwrap();
        for expected in [
            "Fix the hover state",
            "Buttons flicker on hover.",
            "- `src/ui/button.ts` (updated)",
            "No claims were found.",
            "No decisions were recorded.",
        ] {
            assert!(
                prompt.contains(expected),
                "missing {expected:?} in:\n{prompt}"
            );
        }
    }

    /// An effort on another provider's work item has no oxplow task; the
    /// prompt names the work item instead of coming back empty (tsk458).
    #[tokio::test]
    async fn review_prompt_names_a_foreign_work_item() {
        use oxplow_db::EffortStore as _;
        let f = crate::test_fixtures::services_with_effort().await;
        let effort = f
            .svc
            .effort_store
            .start("work_item:issues:ENG-1", &f.thread, None)
            .await
            .unwrap();
        let rows = run_bundled_lens(
            &f,
            "oxplow-bundled/review-prompt",
            &[("effort_id", effort.id.value())],
        )
        .await;
        let prompt = rows[0][0]
            .as_str()
            .unwrap_or_else(|| panic!("no prompt: {rows}"));
        assert!(
            prompt.contains("**work_item:issues:ENG-1**"),
            "missing the work item in:\n{prompt}"
        );
    }

    /// The stream-level review starters list the viewer's stream's
    /// decisions and unbacked claims, newest first (tsk374).
    #[tokio::test]
    async fn review_starters_read_the_viewers_stream() {
        let f = crate::test_fixtures::services_with_task_effort().await;
        let thread = f.thread.value();
        let (task, effort) = (f.task, f.effort);
        let store = &f.svc.db;
        store
            .transaction(move |tx| {
                oxplow_db::record_decision_tx(
                    tx,
                    &oxplow_db::NewDecision {
                        thread_id: thread,
                        work_item: Some(oxplow_tasks::work_item_ref(task)),
                        effort_id: Some(effort.value()),
                        question: "Where does export live?".into(),
                        choice: "src/export".into(),
                        alternatives: vec![],
                        confidence: "high".into(),
                        why: String::new(),
                    },
                )
            })
            .await
            .unwrap();
        for (statement, evidence) in [("tests pass", None), ("handles empty", Some("test:empty"))] {
            store
                .transaction(move |tx| {
                    oxplow_db::insert_claim_tx(
                        tx,
                        oxplow_db::ClaimProvenance::Inferred,
                        &oxplow_db::NewClaim {
                            thread_id: thread,
                            work_item: Some(oxplow_tasks::work_item_ref(task)),
                            effort_id: Some(effort.value()),
                            statement: statement.into(),
                            kind: "tests_pass".into(),
                            evidence_ref: evidence.map(str::to_string),
                            agent_session_id: None,
                        },
                    )
                })
                .await
                .unwrap();
        }
        let layer = crate::sql_gateway::SqlGateway::new(f.svc.db.clone());
        let ctx = crate::extensions::lens_context(&f.svc, None, None).await;
        let first = |id: &'static str, ctx: crate::extensions::LensContext| {
            let layer = layer.clone();
            let root = f._dir.path().to_path_buf();
            let catalog = f.svc.extension_catalog.clone();
            async move {
                let run = crate::extensions::run_lens(
                    &layer,
                    &catalog,
                    &root,
                    id,
                    Default::default(),
                    &ctx,
                )
                .await
                .unwrap();
                serde_json::to_value(&run.result.rows).unwrap()
            }
        };
        let decisions = first("oxplow-bundled/recent-decisions", ctx.clone()).await;
        assert_eq!(decisions[0][0], "Where does export live?");
        let claims = first("oxplow-bundled/unbacked-claims", ctx).await;
        assert_eq!(claims.as_array().unwrap().len(), 1);
        assert_eq!(claims[0][0], "tests pass");
        let elsewhere = crate::extensions::LensContext {
            stream_id: Some(999),
            thread_id: None,
            active: None,
        };
        assert_eq!(
            first("oxplow-bundled/unbacked-claims", elsewhere).await,
            serde_json::json!([])
        );
    }

    /// The rail's Comments panel is a lens: the stream's open
    /// comments for the person and for the agent, each row and the header
    /// opening the inbox.
    #[tokio::test]
    async fn comments_is_a_panel_counting_open_comments_by_who_acts() {
        let f = crate::test_fixtures::services_with_effort().await;
        let bundled = crate::extensions::load_extensions(f._dir.path())
            .into_iter()
            .find(|e| e.name == "oxplow-bundled")
            .unwrap();
        let panel = bundled
            .panels
            .iter()
            .find(|p| p.id == "oxplow-bundled/comments")
            .expect("a comments panel");
        assert_eq!(
            (panel.body.as_str(), panel.open.as_deref()),
            ("oxplow-bundled/comments", Some("page:comments"))
        );
        let run = |name: &'static str, input: serde_json::Value| {
            let svc = f.svc.clone();
            async move {
                svc.commands
                    .run(&oxplow_domain::Actor::Human, name, input, false)
                    .await
                    .unwrap()
            }
        };
        for intent in ["note", "note", "followup", "note"] {
            run(
                crate::commands::comment::ADD,
                serde_json::json!({
                    "stream": "stream:str1",
                    "target": { "kind": "wiki", "id": "some-page" },
                    "intent": intent,
                    "body": "b",
                }),
            )
            .await;
        }
        // The last note is resolved: it no longer counts.
        let last = f
            .svc
            .sql
            .query_sql("SELECT max(id) FROM v_comment", vec![], None)
            .await
            .unwrap();
        let last = serde_json::to_value(&last.rows[0][0]).unwrap();
        run(
            crate::commands::comment::UPDATE,
            serde_json::json!({ "comment": format!("comment:cmt{last}"), "status": "resolved" }),
        )
        .await;
        let rows = run_bundled_lens(&f, "oxplow-bundled/comments", &[("stream_id", 1)]).await;
        assert_eq!(
            rows,
            serde_json::json!([
                ["For me", 2, "page:comments"],
                ["For the agent", 1, "page:comments"]
            ])
        );
        let none = run_bundled_lens(&f, "oxplow-bundled/comments", &[("stream_id", 2)]).await;
        assert_eq!(none, serde_json::json!([]));
    }

    /// The rail's Work panel is a lens: per thread, what's in
    /// progress, what's ready and what finished since the person last
    /// cleared it (an extension command recording an event), grouped
    /// under headings; collapsed, just the active item; counted without
    /// an alert.
    /// The task lenses need no work list: with none they run (and, reading
    /// the interface, show nothing) rather than saying what's missing.
    #[tokio::test]
    async fn the_task_lenses_run_without_a_work_list() {
        let f = crate::test_fixtures::services_with_effort().await;
        let run = |svc: std::sync::Arc<crate::Services>| async move {
            let ctx = crate::extensions::lens_context(&svc, None, None).await;
            crate::extensions::run_lens(
                &svc.sql,
                &svc.extension_catalog,
                &svc.layout.project_dir,
                "oxplow-bundled/ready-tasks",
                Default::default(),
                &ctx,
            )
            .await
            .unwrap()
        };
        assert!(run(f.svc.clone()).await.inactive.is_none());
        f.svc
            .config
            .write()
            .unwrap()
            .personal_active_providers
            .insert("work_items".into(), "none".into());
        assert!(run(f.svc.clone()).await.inactive.is_none());
    }

    #[tokio::test]
    async fn work_is_a_panel_grouping_a_threads_work() {
        let f = crate::test_fixtures::services_with_effort().await;
        let bundled = crate::extensions::load_extensions(f._dir.path())
            .into_iter()
            .find(|e| e.name == "oxplow-bundled")
            .unwrap();
        assert!(bundled.errors.is_empty(), "{:?}", bundled.errors);
        let panel = bundled
            .panels
            .iter()
            .find(|p| p.id == "oxplow-bundled/work")
            .expect("a work panel");
        assert_eq!(
            (
                panel.body.as_str(),
                panel.collapsed.as_deref(),
                panel.count.as_deref(),
                panel.open.as_deref()
            ),
            (
                "oxplow-bundled/work",
                Some("oxplow-bundled/work-line"),
                Some("oxplow-bundled/work-count"),
                Some("page:tasks")
            )
        );
        let run = |name: &'static str, input: serde_json::Value| {
            let svc = f.svc.clone();
            async move {
                svc.commands
                    .run(&oxplow_domain::Actor::Human, name, input, false)
                    .await
                    .unwrap_or_else(|e| panic!("{name}: {e:?}"))
            }
        };
        let thread = format!("thr{}", f.thread.value());
        for (title, state) in [("Next up", "todo"), ("Shipped", "done")] {
            run(
                "oxplow.work_item.create",
                serde_json::json!({ "title": title, "state": state, "thread": thread }),
            )
            .await;
        }
        // As boot does: its models, its event types, its commands.
        crate::test_fixtures::activate_with_problems(&f.svc).await;
        crate::test_fixtures::activate(&f.svc).await;
        let tid = f.thread.value();
        // (group, title) per row, in display order.
        let lines = |rows: serde_json::Value| -> Vec<(String, String)> {
            rows.as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    (
                        r[0].as_str().unwrap().to_string(),
                        r[1].as_str().unwrap().to_string(),
                    )
                })
                .collect()
        };
        assert_eq!(
            lines(run_bundled_lens(&f, "oxplow-bundled/work", &[("thread_id", tid)]).await),
            vec![
                // The fixture's effort, linked to nothing, by its own title.
                ("In progress".into(), "Work in progress".into()),
                ("Ready".into(), "Next up".into()),
                ("Finished".into(), "Shipped".into()),
            ]
        );
        assert_eq!(
            run_bundled_lens(&f, "oxplow-bundled/work-count", &[("thread_id", tid)]).await,
            serde_json::json!([[2]]),
            "the active item and the ready one; finished isn't counted"
        );
        // Clearing hides what finished before it.
        run(
            "oxplow.work.clear_finished",
            serde_json::json!({ "thread_id": f.thread.value() }),
        )
        .await;
        assert_eq!(
            lines(run_bundled_lens(&f, "oxplow-bundled/work", &[("thread_id", tid)]).await)
                .into_iter()
                .map(|(g, _)| g)
                .collect::<Vec<_>>(),
            vec!["In progress".to_string(), "Ready".to_string()]
        );

        // An effort no item names shows too: in progress while open, then
        // finished.
        let opened = run(
            "oxplow.effort.open",
            serde_json::json!({ "thread": format!("thread:{thread}"), "title": "Tidy the shell scripts" }),
        )
        .await;
        let effort = opened.result["effort"].as_str().unwrap().to_string();
        let work = lines(run_bundled_lens(&f, "oxplow-bundled/work", &[("thread_id", tid)]).await);
        assert!(
            work.contains(&("In progress".into(), "Tidy the shell scripts".into())),
            "{work:?}"
        );
        run(
            "oxplow.effort.close",
            serde_json::json!({ "effort": effort }),
        )
        .await;
        let work = lines(run_bundled_lens(&f, "oxplow-bundled/work", &[("thread_id", tid)]).await);
        assert!(
            work.contains(&("Finished".into(), "Tidy the shell scripts".into())),
            "{work:?}"
        );
    }

    /// The Work panel and its sibling lenses read the work-item interface:
    /// with the work list switched to none, oxplow's tasks are gone from
    /// them, whatever the task tables hold.
    #[tokio::test]
    async fn work_lenses_show_only_the_active_work_list() {
        let f = crate::test_fixtures::services_with_effort().await;
        let thread = format!("thr{}", f.thread.value());
        for (title, state) in [("Next up", "todo"), ("Stuck", "blocked")] {
            f.svc
                .commands
                .run(
                    &oxplow_domain::Actor::Human,
                    "oxplow.work_item.create",
                    serde_json::json!({ "title": title, "state": state, "thread": thread }),
                    false,
                )
                .await
                .unwrap();
        }
        f.svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "oxplow.work_item.create",
                serde_json::json!({ "title": "Someday" }),
                false,
            )
            .await
            .unwrap();
        crate::test_fixtures::activate_with_problems(&f.svc).await;
        let tid = f.thread.value();
        let titles = |rows: serde_json::Value, col: usize| -> Vec<String> {
            rows.as_array()
                .unwrap()
                .iter()
                .map(|r| r[col].as_str().unwrap_or_default().to_string())
                .collect()
        };
        assert!(titles(
            run_bundled_lens(&f, "oxplow-bundled/work", &[("thread_id", tid)]).await,
            1
        )
        .contains(&"Next up".to_string()));
        assert_eq!(
            titles(
                run_bundled_lens(&f, "oxplow-bundled/backlog-tasks", &[]).await,
                1
            ),
            vec!["Someday".to_string()]
        );

        f.svc
            .config
            .write()
            .unwrap()
            .active_providers
            .insert("work_items".into(), "none".into());
        let config = crate::config_service::read_config(&f.svc.config);
        f.svc.capabilities.publish_now(&config, &f.svc.db).unwrap();

        let work = titles(
            run_bundled_lens(&f, "oxplow-bundled/work", &[("thread_id", tid)]).await,
            1,
        );
        assert_eq!(
            work,
            vec!["Work in progress".to_string()],
            "only the unlinked effort"
        );
        assert_eq!(
            run_bundled_lens(&f, "oxplow-bundled/work-count", &[("thread_id", tid)]).await,
            serde_json::json!([[1]])
        );
        for lens in ["oxplow-bundled/backlog-tasks", "oxplow-bundled/ready-tasks"] {
            assert_eq!(
                run_bundled_lens(&f, lens, &[]).await,
                serde_json::json!([]),
                "{lens}"
            );
        }
        let waiting = titles(
            run_bundled_lens(&f, "oxplow-bundled/waiting-on-me", &[]).await,
            1,
        );
        assert!(!waiting.contains(&"Stuck".to_string()), "{waiting:?}");
    }

    /// The foundation's typed lenses (offered to the agent as tools): each
    /// reads its params as refs and returns the rows its tool answers with.
    #[tokio::test]
    async fn the_foundations_typed_lenses_answer_their_questions() {
        use oxplow_db::SqlCell::{Int, Text};
        let f = crate::test_fixtures::services_with_effort().await;
        let thread = f.thread.value();
        let effort = f.effort.value();
        f.svc
            .db
            .transaction(move |tx| {
                tx.execute_batch(&format!(
                    "INSERT INTO effort_file (effort_id, path, change_kind, local_snapshot_id, source)
                       VALUES ({effort}, 'b.rs', 'created', 0, 'claimed'), ({effort}, 'a.rs', 'updated', 0, 'claimed');
                     INSERT INTO git_commit (sha, author, email, committed_at, subject)
                       VALUES ('c1', 'ann', 'a@x', '2026-01-01T00:00:00Z', 'first'),
                              ('c2', 'bob', 'b@x', '2026-01-02T00:00:00Z', 'second');
                     INSERT INTO git_commit_file (sha, path, status, additions, deletions)
                       VALUES ('c1', 'a.rs', 'A', 1, 0), ('c1', 'z.rs', 'A', 1, 0), ('c2', 'a.rs', 'M', 2, 1);
                     INSERT INTO thread_note (thread_id, body, author, created_at)
                       VALUES ({thread}, 'older', 'agent', '2026-01-01T00:00:00Z'),
                              ({thread}, 'newer', 'agent', '2026-01-02T00:00:00Z');
                     INSERT INTO snapshot (id, stream_id, created_at, revision, branch, tree_hash)
                       SELECT 501, stream_id, '2026-01-01T00:00:00Z', NULL, NULL, 'h1' FROM threads WHERE id = {thread};
                     INSERT INTO snapshot (id, stream_id, created_at, revision, branch, tree_hash)
                       SELECT 502, stream_id, '2026-01-02T00:00:00Z', NULL, NULL, 'h2' FROM threads WHERE id = {thread};
                     INSERT INTO snapshot_op (stream_id, snapshot_id, parent_snapshot_id, trigger, at, elapsed_ms, over_budget, file_count)
                       SELECT stream_id, 501, NULL, 'startup', '2026-01-01T00:00:00Z', 5, 0, 3 FROM threads WHERE id = {thread};
                     INSERT INTO snapshot_op (stream_id, snapshot_id, parent_snapshot_id, trigger, at, elapsed_ms, over_budget, file_count)
                       SELECT stream_id, 502, 501, 'quiet', '2026-01-02T00:00:00Z', 900, 1, 4 FROM threads WHERE id = {thread};"
                ))
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        crate::test_fixtures::file_item(
            &f.svc,
            serde_json::json!({ "title": "ship it", "thread": f.thread.to_string() }),
        )
        .await;
        let thread_ref = Text(format!("thread:thr{thread}"));
        let effort_ref = Text(format!("effort:eff{effort}"));
        let col = |rows: &serde_json::Value, i: usize| -> Vec<String> {
            rows.as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    r[i].as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| r[i].to_string())
                })
                .collect()
        };

        let rows = run_bundled_lens_with(
            &f,
            "oxplow-foundation/effort-files",
            &[("effort", effort_ref.clone())],
        )
        .await;
        assert_eq!(col(&rows, 0), ["a.rs", "b.rs"]);
        assert_eq!(col(&rows, 1), ["updated", "created"]);

        let rows = run_bundled_lens_with(
            &f,
            "oxplow-foundation/effort-summary",
            &[("effort", effort_ref)],
        )
        .await;
        let parts = col(&rows, 1);
        assert_eq!(parts.first().map(String::as_str), Some("effort"), "{rows}");
        assert_eq!(&parts[parts.len() - 2..], ["file", "file"], "{rows}");

        let rows = run_bundled_lens_with(
            &f,
            "oxplow-foundation/file-history",
            &[("path", Text("a.rs".into())), ("limit", Int(20))],
        )
        .await;
        assert_eq!(col(&rows, 0), ["c2", "c1"]);
        let rows = run_bundled_lens_with(
            &f,
            "oxplow-foundation/file-history",
            &[("path", Text("a.rs".into())), ("limit", Int(1))],
        )
        .await;
        assert_eq!(col(&rows, 0), ["c2"]);

        let rows =
            run_bundled_lens_with(&f, "oxplow-foundation/recent-commits", &[("limit", Int(5))])
                .await;
        assert_eq!(col(&rows, 0), ["c2", "c1"]);

        let rows = run_bundled_lens_with(
            &f,
            "oxplow-foundation/snapshots",
            &[("stream", Text(String::new())), ("limit", Int(5))],
        )
        .await;
        // Newest first: its parent, trigger and whether it ran over budget.
        assert_eq!(col(&rows, 0), ["502", "501"]);
        assert_eq!(col(&rows, 2), ["501", "null"]);
        assert_eq!(col(&rows, 3), ["quiet", "startup"]);
        assert_eq!(col(&rows, 4), ["1", "0"]);

        let rows = run_bundled_lens_with(
            &f,
            "oxplow-foundation/thread-notes",
            &[("thread", thread_ref.clone())],
        )
        .await;
        assert_eq!(col(&rows, 3), ["newer", "older"]);

        let rows = run_bundled_lens_with(
            &f,
            "oxplow-foundation/thread-work",
            &[("thread", thread_ref)],
        )
        .await;
        assert_eq!(col(&rows, 1), ["ship it"], "{rows}");
        assert_eq!(col(&rows, 2), ["todo"]);
    }

    /// The thread-sessions lens lists a thread's sessions with their
    /// status and origin, and beneath each its subagents — a bracketed one
    /// by its state, one only its calls name as `unknown`.
    #[tokio::test]
    async fn thread_sessions_lists_sessions_with_their_subagents_beneath() {
        use oxplow_db::SqlCell::Text;
        let f = crate::test_fixtures::services_with_effort().await;
        let (thread, session) = (f.thread.value(), f.session.value());
        f.svc
            .db
            .transaction(move |tx| {
                tx.execute(
                    "INSERT INTO event_log (id, type, v, at, source, thread_id, agent_session_id, subject, payload, payload_hash)
                     VALUES ('e1', 'agent.subagent.started', 1, '2026-10-01T00:00:00.000000Z', 'test', ?1, ?2, '[]',
                             '{\"thread\":\"thread:thr1\",\"subagent\":{\"id\":\"a1\",\"kind\":\"Explore\"}}', 'h1')",
                    rusqlite::params![thread, session],
                )
                .map_err(oxplow_db::map_sql_err)?;
                tx.execute(
                    "INSERT INTO agent_tool_call (thread_id, tool, kind, at, agent_session_id, subagent_id)
                     VALUES (?1, 'Read', 'read', '2026-10-01T00:01:00.000000Z', ?2, 'child-9')",
                    rusqlite::params![thread, session],
                )
                .map_err(oxplow_db::map_sql_err)?;
                Ok(())
            })
            .await
            .unwrap();
        let rows = run_bundled_lens_with(
            &f,
            "oxplow-foundation/thread-sessions",
            &[(
                "thread",
                Text(oxplow_domain::refs::build::thread_ref(f.thread)),
            )],
        )
        .await;
        let col = |rows: &serde_json::Value, i: usize| -> Vec<String> {
            rows.as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    r[i].as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| r[i].to_string())
                })
                .collect()
        };
        let names = col(&rows, 4);
        assert_eq!(names.len(), 3, "{rows}");
        assert!(
            names[0].starts_with("claude "),
            "the session first: {names:?}"
        );
        assert!(names[1].trim_start().starts_with("Explore a1"), "{names:?}");
        assert!(
            names[2].trim_start().starts_with("subagent child-9"),
            "{names:?}"
        );
        assert_eq!(col(&rows, 7), ["idle", "running", "unknown"], "{rows}");
        assert_eq!(col(&rows, 5), ["claude", "subagent", "subagent"]);
    }

    /// The transcript lens lists a session's turns oldest first with the
    /// thread's notes where they fall, marks a subagent's hand-back, counts
    /// each turn's tool calls, starts at a turn, and pages by 50.
    #[tokio::test]
    async fn session_transcript_lists_turns_notes_and_handbacks() {
        use oxplow_db::SqlCell::{Int, Text};
        let f = crate::test_fixtures::services_with_effort().await;
        let (thread, session) = (f.thread.value(), f.session.value());
        f.svc
            .db
            .transaction(move |tx| {
                for (i, (prompt, answer)) in [("first ask", "one"), ("", "report"), ("third ask", "three")]
                    .into_iter()
                    .enumerate()
                {
                    tx.execute(
                        "INSERT INTO agent_turn (thread_id, agent_session_id, prompt, answer, started_at, ended_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        rusqlite::params![
                            thread,
                            session,
                            prompt,
                            answer,
                            format!("2999-01-0{}T10:00:00.000000Z", i + 1),
                            format!("2999-01-0{}T10:05:00.000000Z", i + 1),
                        ],
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                }
                tx.execute(
                    "INSERT INTO agent_tool_call (thread_id, tool, kind, at, agent_session_id, turn_id)
                     VALUES (?1, 'Read', 'read', '2999-01-01T10:01:00.000000Z', ?2, 1),
                            (?1, 'Read', 'read', '2999-01-01T10:02:00.000000Z', ?2, 1)",
                    rusqlite::params![thread, session],
                )
                .map_err(oxplow_db::map_sql_err)?;
                tx.execute(
                    "INSERT INTO thread_note (thread_id, body, author, created_at)
                     VALUES (?1, 'a finding in between', 'agent', '2999-01-01T12:00:00.000000Z')",
                    [thread],
                )
                .map_err(oxplow_db::map_sql_err)?;
                // The session's window has to cover them.
                tx.execute("UPDATE agent_session SET opened_at = '2998-12-31T00:00:00.000000Z'", [])
                    .map_err(oxplow_db::map_sql_err)?;
                Ok(())
            })
            .await
            .unwrap();
        let session_ref = Text(oxplow_domain::refs::build::agent_session_ref(f.session));
        let col = |rows: &serde_json::Value, i: usize| -> Vec<String> {
            rows.as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    r[i].as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| r[i].to_string())
                })
                .collect()
        };
        let params = |turn: &str, page: i64| {
            vec![
                ("session", session_ref.clone()),
                ("turn", Text(turn.into())),
                ("page", Int(page)),
            ]
        };
        let rows =
            run_bundled_lens_with(&f, "oxplow-foundation/session-transcript", &params("", 1)).await;
        // ref, at, kind, ended_at, prompt, answer, tool_calls, …
        assert_eq!(
            col(&rows, 2),
            ["turn", "note", "hand-back", "turn"],
            "{rows}"
        );
        assert_eq!(col(&rows, 4)[1], "a finding in between");
        assert_eq!(col(&rows, 6), ["2", "0", "0", "0"]);

        // Starting at the hand-back's turn leaves it and what follows.
        let handback = col(&rows, 0)[2].clone();
        let rows = run_bundled_lens_with(
            &f,
            "oxplow-foundation/session-transcript",
            &params(&handback, 1),
        )
        .await;
        assert_eq!(col(&rows, 2), ["hand-back", "turn"], "{rows}");
        // A second page of 50 is past them.
        let rows =
            run_bundled_lens_with(&f, "oxplow-foundation/session-transcript", &params("", 2)).await;
        assert_eq!(col(&rows, 2), Vec::<String>::new(), "{rows}");
    }

    /// A turn opens its session's transcript at it: the page is the lens
    /// ref built in SQL, with no prefix spelled outside the owners.
    #[tokio::test]
    async fn a_turn_opens_its_sessions_transcript_at_it() {
        let f = crate::test_fixtures::services_with_effort().await;
        let (thread, session) = (f.thread.value(), f.session.value());
        f.svc
            .db
            .transaction(move |tx| {
                tx.execute(
                    "INSERT INTO agent_turn (thread_id, agent_session_id, prompt, started_at)
                     VALUES (?1, ?2, 'ask', '2999-01-01T10:00:00.000000Z')",
                    rusqlite::params![thread, session],
                )
                .map_err(oxplow_db::map_sql_err)?;
                tx.execute(
                    "INSERT INTO agent_turn (thread_id, prompt, started_at)
                     VALUES (?1, 'unclaimed', '2999-01-02T10:00:00.000000Z')",
                    [thread],
                )
                .map_err(oxplow_db::map_sql_err)?;
                Ok(())
            })
            .await
            .unwrap();
        let rows = f
            .svc
            .sql
            .query_sql(
                "SELECT ref, transcript_page FROM v_agent_turn ORDER BY id",
                vec![],
                None,
            )
            .await
            .unwrap()
            .rows;
        let page = match &rows[0][1] {
            oxplow_db::SqlCell::Text(t) => t.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            page,
            format!(
                "lens:oxplow-foundation/session-transcript?session=agent_session:ses{session}&turn=turn:trn1"
            )
        );
        assert_eq!(rows[1][1], oxplow_db::SqlCell::Null(()));
    }

    /// The data catalog lists every model with its owner, what it holds
    /// and its columns; naming one gives its columns' docs and key.
    #[tokio::test]
    async fn the_catalog_lists_models_and_one_models_columns() {
        use oxplow_db::SqlCell::Text;
        let f = crate::test_fixtures::services_with_effort().await;
        let col = |rows: &serde_json::Value, i: usize| -> Vec<String> {
            rows.as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    r[i].as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| r[i].to_string())
                })
                .collect()
        };
        let rows = run_bundled_lens_with(
            &f,
            "oxplow-foundation/catalog",
            &[("model", Text(String::new()))],
        )
        .await;
        let views = col(&rows, 0);
        assert!(
            views.iter().any(|v| v == "v_effort") && views.iter().any(|v| v == "v_model"),
            "{views:?}"
        );
        let rows = run_bundled_lens_with(
            &f,
            "oxplow-foundation/catalog",
            &[("model", Text("effort".into()))],
        )
        .await;
        assert_eq!(col(&rows, 0), ["v_effort"]);
        let columns = &col(&rows, 3)[0];
        assert!(
            columns.lines().any(|l| l.starts_with("ref TEXT — ")),
            "{columns}"
        );
        assert!(
            columns.lines().next().unwrap().starts_with("id INTEGER"),
            "{columns}"
        );
    }

    /// The pages the foundation shows as lenses are declared in its
    /// manifest and their lenses exist.
    #[tokio::test]
    async fn the_foundation_declares_its_lens_pages() {
        let f = crate::test_fixtures::services_with_effort().await;
        let foundation = f
            .svc
            .extension_catalog
            .named(f._dir.path(), "oxplow-foundation")
            .expect("the foundation loads");
        let pages: Vec<(&str, &str)> = foundation
            .pages
            .iter()
            .map(|p| (p.id.as_str(), p.lens.as_str()))
            .collect();
        for (page, lens) in [
            ("problems", "oxplow-foundation/problems"),
            ("symbols", "oxplow-foundation/symbols"),
            ("closed-threads", "oxplow-foundation/closed-threads"),
            ("hook-events", "oxplow-foundation/hook-events"),
            ("done-work", "oxplow-foundation/done-work"),
            ("wiki-freshness", "oxplow-foundation/wiki-freshness"),
            ("wiki-index", "oxplow-foundation/wiki-index"),
        ] {
            assert!(pages.contains(&(page, lens)), "{page}: {pages:?}");
        }
    }

    fn texts(rows: &serde_json::Value, i: usize) -> Vec<String> {
        rows.as_array()
            .unwrap()
            .iter()
            .map(|r| r[i].as_str().unwrap_or_default().to_string())
            .collect()
    }

    /// Problems lists the stream's diagnostics by file, files with errors
    /// first, the severest first inside a file; a severity keeps only its own.
    #[tokio::test]
    async fn the_problems_lens_groups_diagnostics_by_file() {
        use oxplow_db::SqlCell::{Int, Text};
        let f = crate::test_fixtures::services_with_effort().await;
        let d = |severity: &str, message: &str, line: i64| oxplow_db::DiagnosticRow {
            severity: severity.into(),
            message: message.into(),
            line,
            col: 1,
            ..Default::default()
        };
        let put = |path: &str, rows| {
            f.svc
                .diagnostic_store
                .replace_file(1, "rust".into(), path.into(), rows)
        };
        put("a.rs", vec![d("warning", "unused", 3)]).await.unwrap();
        put(
            "b.rs",
            vec![d("warning", "late", 9), d("error", "broken", 20)],
        )
        .await
        .unwrap();
        let fixture = &f;
        let run = |severity: &'static str| async move {
            run_bundled_lens_with(
                fixture,
                "oxplow-foundation/problems",
                &[("stream_id", Int(1)), ("severity", Text(severity.into()))],
            )
            .await
        };
        let all = run("all").await;
        assert_eq!(texts(&all, 0), ["b.rs", "b.rs", "a.rs"]);
        assert_eq!(texts(&all, 2), ["broken", "late", "unused"]);
        let errors = run("error").await;
        assert_eq!(texts(&errors, 2), ["broken"]);
    }

    /// Symbols nests each under its container, under its file; a filter
    /// keeps the names that contain it, a path one file.
    #[tokio::test]
    async fn the_symbols_lens_nests_symbols_under_their_container_and_file() {
        use oxplow_db::SqlCell::{Int, Text};
        let f = crate::test_fixtures::services_with_effort().await;
        let sym = |id: &str, name: &str, kind: &str, container: Option<&str>, line: i64| {
            oxplow_db::SymbolRow {
                id: id.into(),
                name: name.into(),
                kind: kind.into(),
                container: container.map(str::to_string),
                signature: None,
                line,
                col: 1,
                start_line: line,
                start_col: 1,
                end_line: line + 1,
                end_col: 1,
            }
        };
        f.svc
            .symbol_store
            .record(
                1,
                1,
                vec![oxplow_db::FileSymbols {
                    path: "w.py".into(),
                    language: "python".into(),
                    rows: Some(vec![
                        sym("Widget", "Widget", "class", None, 1),
                        sym("Widget::spin", "spin", "method", Some("Widget"), 2),
                    ]),
                }],
                oxplow_db::SymbolCapture::default(),
            )
            .await
            .unwrap();
        let fixture = &f;
        let run = |path: &'static str, filter: &'static str| async move {
            run_bundled_lens_with(
                fixture,
                "oxplow-foundation/symbols",
                &[
                    ("stream_id", Int(1)),
                    ("path", Text(path.into())),
                    ("filter", Text(filter.into())),
                ],
            )
            .await
        };
        let all = run("", "").await;
        assert_eq!(texts(&all, 2), ["w.py", "class Widget", "method spin"]);
        // `Widget` is the method's parent; the file is the class's.
        assert_eq!(texts(&all, 1)[2], texts(&all, 0)[1]);
        // A file's node is its path (a tree id, not a ref); the class's
        // parent is that node.
        assert_eq!(texts(&all, 1)[1], "w.py");
        assert_eq!(texts(&all, 0)[0], "w.py");
        let found = run("", "spi").await;
        assert_eq!(texts(&found, 2), ["w.py", "method spin"]);
        let outline = run("w.py", "").await;
        assert_eq!(texts(&outline, 2), ["class Widget", "method spin"]);
    }

    /// Closed threads lists only closed ones, with the work they finished.
    #[tokio::test]
    async fn the_closed_threads_lens_lists_closed_threads_with_their_work() {
        use oxplow_db::SqlCell::Int;
        let f = crate::test_fixtures::services_with_effort().await;
        let thread =
            crate::test_fixtures::new_thread(&f.svc, oxplow_domain::StreamId::new(1), "Old").await;
        crate::test_fixtures::new_thread(&f.svc, oxplow_domain::StreamId::new(1), "Live").await;
        f.svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                crate::commands::thread::CLOSE,
                serde_json::json!({ "thread": oxplow_domain::refs::build::thread_ref(thread.id) }),
                false,
            )
            .await
            .unwrap();
        let rows = run_bundled_lens_with(
            &f,
            "oxplow-foundation/closed-threads",
            &[("stream_id", Int(1))],
        )
        .await;
        assert_eq!(texts(&rows, 1), ["Old"]);
    }

    /// Done work lists the thread's closed items, the latest first.
    #[tokio::test]
    async fn the_done_work_lens_lists_closed_items_latest_first() {
        use oxplow_db::SqlCell::Int;
        let f = crate::test_fixtures::services_with_effort().await;
        let thread = f.thread.to_string();
        let mut refs = Vec::new();
        for title in ["older", "newer", "open"] {
            let item = crate::test_fixtures::file_item(
                &f.svc,
                serde_json::json!({ "title": title, "thread": thread }),
            )
            .await;
            refs.push(oxplow_tasks::work_item_ref(item));
        }
        for (r, to) in [(&refs[0], "done"), (&refs[1], "canceled")] {
            f.svc
                .commands
                .run(
                    &oxplow_domain::Actor::Human,
                    "oxplow.work_item.transition",
                    serde_json::json!({ "ref": r, "to": to }),
                    false,
                )
                .await
                .unwrap();
        }
        let (older, newer) = (refs[0].clone(), refs[1].clone());
        f.svc
            .db
            .transaction(move |tx| {
                tx.execute(
                    "UPDATE work_item SET closed_at = '2026-01-05T09:00:00Z' WHERE ref = ?1",
                    [&older],
                )
                .and_then(|_| {
                    tx.execute(
                        "UPDATE work_item SET closed_at = '2026-03-01T09:00:00Z' WHERE ref = ?1",
                        [&newer],
                    )
                })
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        let rows = run_bundled_lens_with(
            &f,
            "oxplow-foundation/done-work",
            &[("thread_id", Int(f.thread.value()))],
        )
        .await;
        assert_eq!(texts(&rows, 1), ["newer", "older"], "{rows}");
        assert_eq!(texts(&rows, 2), ["canceled", "done"], "{rows}");
    }

    /// Hook events read the stream's `agent.*` events newest first, each
    /// with what it says: a tool, its target and its outcome.
    #[tokio::test]
    async fn the_hook_events_lens_says_what_each_agent_event_did() {
        use oxplow_db::SqlCell::Int;
        let f = crate::test_fixtures::services_with_effort().await;
        let post = |event: &str, payload: serde_json::Value| {
            let env = oxplow_domain::Envelope::new(event, 3, "test", payload)
                .unwrap()
                .with_anchors(oxplow_domain::events::Anchors {
                    stream_id: Some(oxplow_domain::StreamId::new(1)),
                    ..Default::default()
                });
            let log = f.svc.event_log_store.clone();
            async move { log.append(env).await.unwrap() }
        };
        post(
            "agent.tool.requested",
            serde_json::json!({ "tool": "Edit", "kind": "edit", "paths": ["src/a.rs"], "decision": "allowed" }),
        )
        .await;
        post(
            "agent.tool.finished",
            serde_json::json!({ "tool": "Edit", "kind": "edit", "paths": ["src/a.rs"], "ok": false }),
        )
        .await;
        let rows = run_bundled_lens_with(
            &f,
            "oxplow-foundation/hook-events",
            &[("stream_id", Int(1))],
        )
        .await;
        let kinds = texts(&rows, 2);
        assert_eq!(&kinds[..2], ["tool.finished", "tool.requested"], "{rows}");
        assert_eq!(
            &texts(&rows, 3)[..2],
            ["Edit · src/a.rs · failed", "Edit · src/a.rs"],
            "{rows}"
        );
    }

    /// Wiki freshness lists the files a page cites under a heading that
    /// counts the stale ones.
    #[tokio::test]
    async fn the_wiki_freshness_lens_lists_a_pages_cited_files() {
        use oxplow_db::SqlCell::Text;
        let f = crate::test_fixtures::services_with_effort().await;
        std::fs::create_dir_all(f._dir.path().join("src")).unwrap();
        std::fs::write(f._dir.path().join("src/a.rs"), "v1").unwrap();
        f.svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                crate::knowledge::WRITE_PAGE,
                serde_json::json!({ "slug": "lib", "body": "# Lib\n\nsee [[src/a.rs]]\n" }),
                true,
            )
            .await
            .unwrap();
        let rows = run_bundled_lens_with(
            &f,
            "oxplow-foundation/wiki-freshness",
            &[("slug", Text("lib".into()))],
        )
        .await;
        assert_eq!(texts(&rows, 1), ["lib — 0 of 1 stale"], "{rows}");
        assert_eq!(texts(&rows, 2), ["src/a.rs"], "{rows}");
        assert_eq!(texts(&rows, 4), ["✓ fresh"], "{rows}");
    }

    /// The wiki index: pages not visited, newest first; a filter finds the
    /// pages whose title or body holds it, with the text around the match;
    /// the visited ones list apart.
    #[tokio::test]
    async fn the_wiki_index_lists_visited_pages_apart_and_filters_by_title_or_body() {
        use oxplow_db::analytics_stores::PageVisitStore;
        use oxplow_db::SqlCell::Text;
        let f = crate::test_fixtures::services_with_effort().await;
        for (slug, body) in [
            ("alpha", "# Alpha\n\nThe quick brown fox jumps.\n"),
            ("beta", "# Beta\n\nNothing here.\n"),
            ("gamma", "# Gamma\n\nA lazy dog sleeps.\n"),
        ] {
            f.svc
                .commands
                .run(
                    &oxplow_domain::Actor::Human,
                    crate::knowledge::WRITE_PAGE,
                    serde_json::json!({ "slug": slug, "body": body }),
                    true,
                )
                .await
                .unwrap();
        }
        let thread = f.thread.to_string();
        f.svc
            .page_visit_store
            .record("wiki", "wiki:beta", Some("Beta"), None, Some(&thread))
            .await
            .unwrap();
        let filter = |q: &str| {
            vec![
                ("stream_id", oxplow_db::SqlCell::Int(1)),
                ("filter", Text(q.into())),
            ]
        };
        let pages = |q: String| {
            let f = &f;
            async move { run_bundled_lens_with(f, "oxplow-foundation/wiki-pages", &filter(&q)).await }
        };
        let rows = pages(String::new()).await;
        let mut slugs = texts(&rows, 0);
        slugs.sort();
        assert_eq!(slugs, ["alpha", "gamma"], "{rows}");
        let rows = pages("LAZY".into()).await;
        assert_eq!(texts(&rows, 0), ["gamma"], "{rows}");
        assert!(texts(&rows, 3)[0].contains("lazy dog"), "{rows}");
        let rows = pages("beta".into()).await;
        assert_eq!(texts(&rows, 0), ["beta"], "{rows}");
        let rows = run_bundled_lens_with(&f, "oxplow-foundation/wiki-visited", &filter("")).await;
        assert_eq!(texts(&rows, 0), ["beta"], "{rows}");
        let rows = run_bundled_lens_with(&f, "oxplow-foundation/wiki-visited", &filter("x")).await;
        assert!(texts(&rows, 0).is_empty(), "{rows}");
    }

    /// Finished items come newest first by when they closed, even within
    /// one year (`closed_at` is RFC 3339 text), after the open ones.
    #[tokio::test]
    async fn thread_work_lists_finished_items_newest_first() {
        use oxplow_db::SqlCell::Text;
        let f = crate::test_fixtures::services_with_effort().await;
        let thread = f.thread.to_string();
        let mut refs = Vec::new();
        for title in ["older done", "newer done", "still open"] {
            let item = crate::test_fixtures::file_item(
                &f.svc,
                serde_json::json!({ "title": title, "thread": thread }),
            )
            .await;
            refs.push(oxplow_tasks::work_item_ref(item));
        }
        for r in &refs[..2] {
            f.svc
                .commands
                .run(
                    &oxplow_domain::Actor::Human,
                    "oxplow.work_item.transition",
                    serde_json::json!({ "ref": r, "to": "done" }),
                    false,
                )
                .await
                .unwrap();
        }
        let (older, newer) = (refs[0].clone(), refs[1].clone());
        f.svc
            .db
            .transaction(move |tx| {
                tx.execute(
                    "UPDATE work_item SET closed_at = '2026-01-05T09:00:00Z' WHERE ref = ?1",
                    [&older],
                )
                .and_then(|_| {
                    tx.execute(
                        "UPDATE work_item SET closed_at = '2026-03-01T09:00:00Z' WHERE ref = ?1",
                        [&newer],
                    )
                })
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        let rows = run_bundled_lens_with(
            &f,
            "oxplow-foundation/thread-work",
            &[("thread", Text(format!("thread:thr{}", f.thread.value())))],
        )
        .await;
        let titles: Vec<&str> = rows
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r[1].as_str().unwrap())
            .collect();
        assert_eq!(titles, ["still open", "newer done", "older done"], "{rows}");
    }

    /// What to pick up next on a thread's list: ready items in list order,
    /// an open blocker holding one until it closes, and a ready epic first
    /// coming with its ready descendants as one unit.
    #[tokio::test]
    async fn work_next_offers_the_ready_items_or_the_first_epic_with_its_children() {
        use oxplow_domain::Actor;
        let f = crate::test_fixtures::services_with_effort().await;
        let thread = f.thread.to_string();
        let run = |id: &'static str, input: serde_json::Value| {
            let svc = f.svc.clone();
            async move {
                svc.commands
                    .run(&Actor::Human, id, input, false)
                    .await
                    .unwrap()
                    .result
            }
        };
        let file = |title: &'static str, extra: serde_json::Value| {
            let mut input = serde_json::json!({ "title": title, "thread": thread });
            input
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            async move {
                run("oxplow.work_item.create", input).await["ref"]
                    .as_str()
                    .unwrap()
                    .to_string()
            }
        };
        let next = || async {
            let rows = run_bundled_lens_with(
                &f,
                "oxplow-foundation/work-next",
                &[(
                    "thread",
                    oxplow_db::SqlCell::Text(format!("thread:{}", f.thread)),
                )],
            )
            .await;
            rows.as_array()
                .unwrap()
                .iter()
                .map(|r| format!("{} {}", r[1].as_str().unwrap(), r[2].as_str().unwrap()))
                .collect::<Vec<_>>()
        };
        assert!(next().await.is_empty(), "nothing filed, nothing ready");

        let a = file("a", serde_json::json!({})).await;
        file("b", serde_json::json!({ "state": "blocked" })).await;
        file("c", serde_json::json!({})).await;
        assert_eq!(next().await, ["a ready", "c ready"]);

        let z = run(
            "oxplow.work_item.create",
            serde_json::json!({ "title": "z" }),
        )
        .await["ref"]
            .as_str()
            .unwrap()
            .to_string();
        run(
            "oxplow.work_item.link",
            serde_json::json!({ "ref": z, "target": a, "link_type": "blocks" }),
        )
        .await;
        assert_eq!(next().await, ["c ready"], "an open blocker holds a");
        run(
            "oxplow.work_item.transition",
            serde_json::json!({ "ref": z, "to": "done" }),
        )
        .await;
        assert_eq!(next().await, ["a ready", "c ready"], "a closed one doesn't");

        run(
            "oxplow.work_item.transition",
            serde_json::json!({ "ref": a, "to": "done" }),
        )
        .await;
        run(
            "oxplow.work_item.transition",
            serde_json::json!({ "ref": file("x", serde_json::json!({})).await, "to": "in_progress" }),
        )
        .await;
        // c is next, and becomes an epic once it has children.
        let c1 = file(
            "c1",
            serde_json::json!({ "parent_ref": next_ref(&f, "c").await }),
        )
        .await;
        file(
            "c2",
            serde_json::json!({ "parent_ref": next_ref(&f, "c").await, "state": "done" }),
        )
        .await;
        file("g", serde_json::json!({ "parent_ref": c1 })).await;
        file("y", serde_json::json!({})).await;
        assert_eq!(next().await, ["c epic", "c1 child", "g child"]);
    }

    /// The ref of the item titled `title` on `f`'s thread.
    async fn next_ref(f: &crate::test_fixtures::EffortFixture, title: &str) -> String {
        let out = f
            .svc
            .sql
            .query_sql(
                "SELECT ref FROM v_work_item WHERE title = ?1",
                vec![oxplow_db::SqlCell::Text(title.into())],
                None,
            )
            .await
            .unwrap();
        match &out.rows[0][0] {
            oxplow_db::SqlCell::Text(r) => r.clone(),
            other => panic!("{other:?}"),
        }
    }

    /// Shared work: items with open efforts on two threads, files two
    /// threads' efforts changed, an epic whose children sit on different
    /// threads. The thread and item variants keep only their own rows.
    #[tokio::test]
    async fn the_shared_work_lenses_show_what_two_threads_hold() {
        use oxplow_db::EffortStore as _;
        let f = crate::test_fixtures::services_with_task_effort().await;
        let item = oxplow_tasks::work_item_ref(f.task);
        let lens = |id: &'static str| {
            let id = format!("oxplow-bundled/{id}");
            let f = &f;
            async move { run_bundled_lens(f, &id, &[]).await }
        };
        assert_eq!(
            lens("shared-work").await.as_array().unwrap().len(),
            0,
            "nothing shared yet"
        );

        // A second thread on the same item, with its own effort and a file
        // both efforts changed.
        use oxplow_domain::stores::ThreadStore as _;
        let stream = f
            .svc
            .thread_store
            .get(&f.thread)
            .await
            .unwrap()
            .unwrap()
            .stream_id;
        let beta = crate::test_fixtures::new_thread(&f.svc, stream, "Beta").await;
        let opened = f
            .svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                crate::commands::effort::OPEN,
                serde_json::json!({ "thread": oxplow_domain::refs::build::thread_ref(beta.id), "work_item": item }),
                false,
            )
            .await
            .unwrap();
        let beta_effort = opened.result["effort"]
            .as_str()
            .and_then(oxplow_domain::refs::build::effort_of_ref)
            .unwrap();
        for effort in [f.effort, beta_effort] {
            f.svc
                .effort_store
                .record_file(
                    &effort,
                    "src/shared.rs",
                    oxplow_db::effort_store::EffortFileChange::Updated,
                    oxplow_db::effort_store::FileRefVersion {
                        local_snapshot_id: 0,
                        closest_vcs_rev: None,
                        vcs_rev_exact: false,
                    },
                )
                .await
                .unwrap();
        }
        // An epic whose children are on the two threads.
        let epic =
            crate::test_fixtures::file_item(&f.svc, serde_json::json!({ "title": "Epic" })).await;
        let epic_ref = oxplow_tasks::work_item_ref(epic);
        for (title, thread) in [("Child A", f.thread), ("Child B", beta.id)] {
            crate::test_fixtures::file_item(
                &f.svc,
                serde_json::json!({ "title": title, "parent_ref": epic_ref,
                    "thread": thread.to_string() }),
            )
            .await;
        }

        let all = lens("shared-work").await;
        let sections: Vec<(String, String)> = all
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r[0].as_str().unwrap().into(), r[1].as_str().unwrap().into()))
            .collect();
        for want in [
            ("Items on two threads", "t"),
            ("Files two threads changed", "src/shared.rs"),
            ("Epics spread over threads", "Epic"),
        ] {
            assert!(
                sections.contains(&(want.0.to_string(), want.1.to_string())),
                "{want:?} in {sections:?}"
            );
        }
        let threads = all.as_array().unwrap()[0][2].as_str().unwrap();
        assert!(threads.contains("Beta"), "{threads}");

        // The thread variant keeps what involves the thread; a third
        // thread sees none of it.
        let mine = run_bundled_lens(
            &f,
            "oxplow-bundled/thread-shared-work",
            &[("thread_id", beta.id.value())],
        )
        .await;
        assert_eq!(mine.as_array().unwrap().len(), 3, "{mine}");
        let third = crate::test_fixtures::new_thread(&f.svc, stream, "Gamma").await;
        let none = run_bundled_lens(
            &f,
            "oxplow-bundled/thread-shared-work",
            &[("thread_id", third.id.value())],
        )
        .await;
        assert_eq!(none.as_array().unwrap().len(), 0, "{none}");
        // The item variant: the item's own row and the files its efforts changed.
        let item_rows = run_bundled_lens_with(
            &f,
            "oxplow-bundled/item-shared-work",
            &[("ref", oxplow_db::SqlCell::Text(item))],
        )
        .await;
        let kinds: Vec<&str> = item_rows
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r[0].as_str().unwrap())
            .collect();
        assert!(
            kinds.contains(&"Items on two threads") && kinds.contains(&"Files two threads changed"),
            "{kinds:?}"
        );
        assert!(!kinds.contains(&"Epics spread over threads"), "{kinds:?}");
    }

    /// The Plan lenses: the plans an agent proposed, newest first, under
    /// an effort and under a thread; nothing when none was.
    #[tokio::test]
    async fn the_plan_lenses_show_the_proposed_plans() {
        let f = crate::test_fixtures::services_with_effort().await;
        for lens in ["oxplow-bundled/effort-plan", "oxplow-bundled/thread-plan"] {
            let param = if lens.contains("effort") {
                ("effort_id", f.effort.value())
            } else {
                ("thread_id", f.thread.value())
            };
            let rows = run_bundled_lens(&f, lens, &[param]).await;
            assert_eq!(rows.as_array().unwrap().len(), 0, "{lens}: none yet");
        }
        use oxplow_domain::hook::HookKind;
        for (kind, id, plan) in [
            (HookKind::UserPromptSubmit, "", ""),
            (HookKind::PreToolUse, "t1", "1. First"),
            (HookKind::PreToolUse, "t2", "1. Second"),
        ] {
            let body = serde_json::json!({ "tool_name": "ExitPlanMode", "tool_use_id": id,
                "tool_input": { "plan": plan } });
            f.svc
                .hook_ingest
                .ingest(crate::hook_ingest::HookEnvelope {
                    session_title: None,
                    turn_run: None,
                    kind,
                    thread_id: Some(f.thread),
                    stream_id: None,
                    agent_session_id: None,
                    session_id: None,
                    payload_json: body.to_string(),
                    prompt: None,
                    decision: None,
                    tool: crate::test_fixtures::claude_tool(kind, &body),
                    subagent: None,
                    peer: None,
                })
                .await
                .unwrap();
        }
        for lens in ["oxplow-bundled/effort-plan", "oxplow-bundled/thread-plan"] {
            let param = if lens.contains("effort") {
                ("effort_id", f.effort.value())
            } else {
                ("thread_id", f.thread.value())
            };
            let rows = run_bundled_lens(&f, lens, &[param]).await;
            let text = rows[0][0].as_str().unwrap();
            let (first, second) = (text.find("1. First"), text.find("1. Second"));
            assert!(first.is_some() && second.is_some(), "{lens}: {text}");
            assert!(second < first, "{lens}: newest first: {text}");
        }
    }

    /// Thread activity: each turn under the effort it fell in, the turns
    /// outside any effort (questions, talk) under their own heading.
    #[tokio::test]
    async fn thread_activity_lists_turns_by_effort() {
        let f = crate::test_fixtures::services_with_effort().await;
        let turn = |prompt: &'static str| {
            let svc = f.svc.clone();
            let thread = f.thread;
            async move {
                use oxplow_domain::hook::HookKind;
                for kind in [HookKind::UserPromptSubmit, HookKind::Stop] {
                    svc.hook_ingest
                        .ingest(crate::hook_ingest::HookEnvelope {
                            session_title: None,
                            turn_run: None,
                            kind,
                            thread_id: Some(thread),
                            stream_id: None,
                            agent_session_id: None,
                            session_id: None,
                            payload_json: "{}".into(),
                            prompt: Some(prompt.into()),
                            decision: None,
                            tool: None,
                            subagent: None,
                            peer: None,
                        })
                        .await
                        .unwrap();
                }
            }
        };
        turn("Build the parser").await;
        {
            use oxplow_db::EffortStore as _;
            f.svc
                .effort_store
                .finish(&f.effort, None, None)
                .await
                .unwrap();
        }
        turn("What does the lexer do?").await;
        let rows = run_bundled_lens(
            &f,
            "oxplow-bundled/thread-activity",
            &[("thread_id", f.thread.value())],
        )
        .await;
        let got: Vec<(String, String)> = rows
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r[0].as_str().unwrap().to_string(),
                    r[1].as_str().unwrap().to_string(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("Between efforts".into(), "What does the lexer do?".into()),
                // Linked to nothing, the effort is titled by its first prompt.
                ("Build the parser".into(), "Build the parser".into()),
            ]
        );
    }

    /// Thread activity shows how full the context was when a turn ended
    /// (with its window when the harness said it) and how often the turn
    /// compacted it.
    #[tokio::test]
    async fn thread_activity_shows_the_context_fill_and_compactions() {
        use oxplow_domain::hook::{HookKind, TurnRun};
        let f = crate::test_fixtures::services_with_effort().await;
        let thread = f.thread;
        let hook = |kind, prompt: Option<&str>, payload: &str, turn_run| {
            crate::hook_ingest::HookEnvelope {
                session_title: None,
                turn_run,
                kind,
                thread_id: Some(thread),
                stream_id: None,
                agent_session_id: None,
                session_id: None,
                payload_json: payload.into(),
                prompt: prompt.map(str::to_string),
                decision: None,
                tool: None,
                subagent: None,
                peer: None,
            }
        };
        let turn = |prompt: &'static str, window: Option<i64>, compactions: usize| {
            let svc = f.svc.clone();
            async move {
                svc.hook_ingest
                    .ingest(hook(HookKind::UserPromptSubmit, Some(prompt), "{}", None))
                    .await
                    .unwrap();
                for _ in 0..compactions {
                    svc.hook_ingest
                        .ingest(hook(HookKind::PostCompact, None, "{}", None))
                        .await
                        .unwrap();
                }
                let run = TurnRun {
                    context_tokens: Some(52_000),
                    context_window: window,
                    ..TurnRun::default()
                };
                svc.hook_ingest
                    .ingest(hook(HookKind::Stop, None, "{}", Some(run)))
                    .await
                    .unwrap();
            }
        };
        turn("with a window", Some(200_000), 1).await;
        turn("without one", None, 0).await;
        let rows = run_bundled_lens(
            &f,
            "oxplow-bundled/thread-activity",
            &[("thread_id", f.thread.value())],
        )
        .await;
        let got: Vec<(String, serde_json::Value, serde_json::Value)> = rows
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r[1].as_str().unwrap().to_string(),
                    r[4].clone(),
                    r[5].clone(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("without one".into(), "52k".into(), 0.into()),
                ("with a window".into(), "52k of 200k (26%)".into(), 1.into()),
            ]
        );
    }

    /// `session-files` groups an effort's files by the session that changed
    /// them, shared ones apart; the thread's activity names each turn's
    /// session.
    #[tokio::test]
    async fn the_session_lenses_name_who_changed_what() {
        let f = crate::test_fixtures::services_with_effort().await;
        let effort = f.effort.value();
        f.svc
            .db
            .transaction(move |tx| {
                tx.execute_batch(&format!(
                    "INSERT INTO effort_file (effort_id, path, change_kind, local_snapshot_id, source, agent_session_id, shared)
                       VALUES ({effort}, 'a.rs', 'updated', 0, 'claimed', 4, 0),
                              ({effort}, 'b.rs', 'updated', 0, 'observed', NULL, 1);"
                ))
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        let rows = run_analytics_lens(&f, "session-files", "effort_id", effort).await;
        assert_eq!(
            rows,
            serde_json::json!([
                ["Session 4", "a.rs", "updated", "claimed"],
                ["Shared", "b.rs", "updated", "observed"],
            ])
        );
    }

    /// The rail's Go To panel is a lens: the thread's bookmarks,
    /// then its recent (or most visited) pages, each once; collapsed, the
    /// bookmarks alone.
    #[tokio::test]
    async fn go_to_is_a_panel_of_bookmarks_and_history() {
        use oxplow_db::analytics_stores::PageVisitStore;
        use oxplow_db::SqlCell;
        let f = crate::test_fixtures::services_with_effort().await;
        let bundled = crate::extensions::load_extensions(f._dir.path())
            .into_iter()
            .find(|e| e.name == "oxplow-bundled")
            .unwrap();
        assert!(bundled.errors.is_empty(), "{:?}", bundled.errors);
        let panel = bundled
            .panels
            .iter()
            .find(|p| p.id == "oxplow-bundled/go-to")
            .expect("a go-to panel");
        assert_eq!(
            (
                panel.body.as_str(),
                panel.collapsed.as_deref(),
                panel.open.as_deref()
            ),
            (
                "oxplow-bundled/go-to",
                Some("oxplow-bundled/go-to-bookmarks"),
                Some("page:dashboard?variant=visits")
            )
        );
        let thread = f.thread.to_string();
        for (r, kind, label, scope) in [
            ("page:git-dashboard", "git-dashboard", "Git", "thread"),
            ("file:a.rs", "file", "a.rs", "project"),
        ] {
            f.svc
                .commands
                .run(
                    &oxplow_domain::Actor::Human,
                    crate::commands::bookmark::SET,
                    serde_json::json!({ "ref": r, "page_kind": kind, "label": label,
                                        "scope": scope, "thread": format!("thread:{thread}") }),
                    false,
                )
                .await
                .unwrap();
        }
        for (kind, id, label) in [
            ("file", "file:DEV.md", "DEV.md"),
            ("file", "file:DEV.md", "DEV.md"),
            ("file", "file:DEV.md", "DEV.md"),
            ("settings", "page:settings", "Settings"),
            ("agent_session", "agent_session:ses1", "Claude"),
        ] {
            f.svc
                .page_visit_store
                .record(kind, id, Some(label), None, Some(&thread))
                .await
                .unwrap();
        }
        let tid = SqlCell::Int(f.thread.value());
        // (group, title, visits) per row, in display order.
        let rows = |mode: &str| {
            let tid = tid.clone();
            let mode = SqlCell::Text(mode.into());
            let f = &f;
            async move {
                let rows = run_bundled_lens_with(
                    f,
                    "oxplow-bundled/go-to",
                    &[("thread_id", tid), ("mode", mode)],
                )
                .await;
                rows.as_array()
                    .unwrap()
                    .iter()
                    .map(|r| (r[0].to_string(), r[1].to_string(), r[4].to_string()))
                    .collect::<Vec<_>>()
            }
        };
        let row =
            |g: &str, t: &str, n: &str| (format!("\"{g}\""), format!("\"{t}\""), n.to_string());
        assert_eq!(
            rows("recent").await,
            vec![
                row("Bookmarks", "a.rs", "null"),
                row("Bookmarks", "Git", "null"),
                row("History", "Settings", "null"),
                row("History", "DEV.md", "null"),
            ]
        );
        assert_eq!(
            rows("top").await[2..],
            [
                row("Most visited", "DEV.md", "3"),
                row("Most visited", "Settings", "1"),
            ]
        );
        let collapsed = run_bundled_lens_with(
            &f,
            "oxplow-bundled/go-to-bookmarks",
            &[("thread_id", tid.clone())],
        )
        .await;
        assert_eq!(
            collapsed,
            serde_json::json!([
                ["a.rs", "file", "file:a.rs"],
                ["Git", "git-dashboard", "page:git-dashboard"]
            ])
        );
    }

    /// The rail's Uncommitted panel is a lens over the working
    /// tree's file list (change analysis's stage one): a folder tree with
    /// A/M/D letters, collapsed to a one-line summary, counted by files.
    #[tokio::test]
    async fn uncommitted_is_a_panel_over_the_working_files() {
        let f = crate::test_fixtures::services_with_effort().await;
        let bundled = crate::extensions::load_extensions(f._dir.path())
            .into_iter()
            .find(|e| e.name == "oxplow-bundled")
            .unwrap();
        assert!(bundled.errors.is_empty(), "{:?}", bundled.errors);
        let panel = bundled
            .panels
            .iter()
            .find(|p| p.id == "oxplow-bundled/uncommitted")
            .expect("an uncommitted panel");
        assert_eq!(
            (
                panel.body.as_str(),
                panel.collapsed.as_deref(),
                panel.count.as_deref(),
                panel.open.as_deref()
            ),
            (
                "oxplow-bundled/uncommitted",
                Some("oxplow-bundled/uncommitted-line"),
                Some("oxplow-bundled/uncommitted-count"),
                Some("page:uncommitted-changes")
            )
        );
        let root = f.svc.layout.project_dir.clone();
        std::fs::create_dir_all(root.join("src/ui")).unwrap();
        std::fs::write(root.join("src/ui/a.rs"), "fn a() {}\n").unwrap();
        crate::test_fixtures::commit_all(&root, "base");
        std::fs::write(root.join("src/ui/a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        std::fs::write(root.join("src/new.rs"), "fn n() {}\n").unwrap();
        crate::change_analysis::refresh_files(
            &f.svc,
            crate::change_analysis::ChangeTarget::Working {
                stream_id: oxplow_domain::StreamId::new(1).to_string(),
            },
        )
        .await
        .unwrap();
        crate::test_fixtures::activate_with_problems(&f.svc).await;
        let tree = run_bundled_lens(&f, "oxplow-bundled/uncommitted", &[("stream_id", 1)]).await;
        // (id, parent, label) per node.
        let nodes: Vec<(String, Option<String>, String)> = tree
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r[0].as_str().unwrap().to_string(),
                    r[1].as_str().map(str::to_string),
                    r[2].as_str().unwrap().to_string(),
                )
            })
            .collect();
        assert_eq!(
            nodes,
            vec![
                ("d:src".into(), None, "src/  AM".into()),
                (
                    "f:src/new.rs".into(),
                    Some("d:src".into()),
                    "new.rs  A".into()
                ),
                ("d:src/ui".into(), Some("d:src".into()), "ui/  M".into()),
                (
                    "f:src/ui/a.rs".into(),
                    Some("d:src/ui".into()),
                    "a.rs  M".into()
                ),
            ]
        );
        let line =
            run_bundled_lens(&f, "oxplow-bundled/uncommitted-line", &[("stream_id", 1)]).await;
        assert_eq!(line[0][0], "1A 1M  +2 −0");
        let count =
            run_bundled_lens(&f, "oxplow-bundled/uncommitted-count", &[("stream_id", 1)]).await;
        assert_eq!(count, serde_json::json!([[2]]));
    }

    /// Waiting on Me sits in the rail and raises an alert while anything
    /// is waiting on the user.
    #[tokio::test]
    async fn waiting_on_me_is_a_panel_whose_badge_alerts() {
        let f = crate::test_fixtures::services_with_task_effort().await;
        let review = crate::extensions::load_extensions(f._dir.path())
            .into_iter()
            .find(|e| e.name == "oxplow-bundled")
            .unwrap();
        assert!(review
            .panels
            .iter()
            .any(|p| p.id == "oxplow-bundled/waiting"
                && p.badge.as_deref() == Some("oxplow-bundled/waiting-on-me")));
        let layer = crate::sql_gateway::SqlGateway::new(f.svc.db.clone());
        let alert = |run: crate::extensions::LensRun| run.alert.unwrap();
        let run = crate::extensions::run_lens(
            &layer,
            &f.svc.extension_catalog,
            f._dir.path(),
            "oxplow-bundled/waiting-on-me",
            Default::default(),
            &crate::extensions::LensContext::default(),
        )
        .await
        .unwrap();
        assert!(!alert(run).firing);
        f.svc
            .task_store
            .set_status(f.task, oxplow_tasks::TaskStatus::Blocked)
            .await
            .unwrap();
        crate::test_fixtures::restate_task(&f.svc, f.task).await;
        let run = crate::extensions::run_lens(
            &layer,
            &f.svc.extension_catalog,
            f._dir.path(),
            "oxplow-bundled/waiting-on-me",
            Default::default(),
            &crate::extensions::LensContext::default(),
        )
        .await
        .unwrap();
        let a = alert(run);
        assert!(a.firing);
        assert_eq!(a.message, "Waiting on you: 1");
    }

    /// An effort with one unverified claim and one inferred decision, its
    /// task naming `src/ui/` as its area and the effort touching
    /// `crates/db/store.rs` outside it; oxplow-bundled's commands registered.
    async fn review_fixture() -> crate::test_fixtures::TaskEffortFixture {
        use oxplow_db::EffortStore as _;
        use oxplow_tasks::TaskStore as _;
        let f = crate::test_fixtures::services_with_task_effort().await;
        let mut t = f.svc.task_store.get(f.task).await.unwrap().unwrap();
        t.description = "Fix the hover state in [[src/ui/button.ts]].".into();
        f.svc.task_store.update(&t).await.unwrap();
        crate::test_fixtures::restate_task(&f.svc, f.task).await;
        for path in ["src/ui/button.ts", "crates/db/store.rs"] {
            f.svc
                .effort_store
                .record_file(
                    &f.effort,
                    path,
                    oxplow_db::EffortFileChange::Updated,
                    oxplow_db::FileRefVersion {
                        local_snapshot_id: 0,
                        closest_vcs_rev: None,
                        vcs_rev_exact: false,
                    },
                )
                .await
                .unwrap();
        }
        let (thread, task, effort) = (f.thread, f.task, f.effort);
        f.svc
            .db
            .transaction(move |tx| {
                oxplow_db::insert_claim_tx(
                    tx,
                    oxplow_db::ClaimProvenance::Inferred,
                    &oxplow_db::NewClaim {
                        thread_id: thread.value(),
                        work_item: Some(oxplow_tasks::work_item_ref(task)),
                        effort_id: Some(effort.value()),
                        statement: "no behavior change".into(),
                        kind: "no_behavior_change".into(),
                        evidence_ref: None,
                        agent_session_id: None,
                    },
                )
            })
            .await
            .unwrap();
        f.svc
            .reasoning_store
            .replace_inferred(
                f.effort.value(),
                vec![oxplow_db::NewDecision {
                    thread_id: f.thread.value(),
                    work_item: Some(oxplow_tasks::work_item_ref(f.task)),
                    effort_id: Some(f.effort.value()),
                    question: "Which store?".into(),
                    choice: "SQLite".into(),
                    alternatives: vec!["files".into()],
                    confidence: "medium".into(),
                    why: "it's there".into(),
                }],
            )
            .await
            .unwrap();
        // As boot does: its models, its event types, its commands.
        crate::test_fixtures::activate_with_problems(&f.svc).await;
        crate::test_fixtures::activate(&f.svc).await;
        f
    }

    fn effort_ref(f: &crate::test_fixtures::EffortFixture) -> String {
        oxplow_domain::refs::build::effort_ref(f.effort)
    }

    async fn review(
        f: &crate::test_fixtures::EffortFixture,
        actor: &oxplow_domain::Actor,
        command: &str,
        input: serde_json::Value,
    ) -> Result<oxplow_domain::CommandOutcome, oxplow_domain::CommandError> {
        f.svc.commands.run(actor, command, input, true).await
    }

    async fn task_notes(f: &crate::test_fixtures::TaskEffortFixture) -> Vec<String> {
        let task = f.task.value();
        f.svc
            .db
            .read(move |c| {
                let mut st = c
                    .prepare("SELECT body FROM task_note WHERE task_id = ?1 ORDER BY id")
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = st
                    .query_map([task], |r| r.get::<_, String>(0))
                    .map_err(oxplow_db::map_sql_err)?;
                rows.collect::<rusqlite::Result<_>>()
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
    }

    async fn task_status(f: &crate::test_fixtures::TaskEffortFixture) -> String {
        use oxplow_tasks::TaskStore as _;
        let t = f.svc.task_store.get(f.task).await.unwrap().unwrap();
        serde_json::to_value(t.status)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    }

    /// P7.C5: oxplow-bundled loads with no errors, its examples dry-run
    /// clean against the running registry, and its verbs register at boot
    /// as a person's (and a lens's), never an agent's, each confirmed.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_review_extension_registers_its_verbs_and_its_examples_check() {
        let f = review_fixture().await;
        let v = crate::extensions::validate_extension(
            &f.svc.sql,
            &f.svc.extension_catalog,
            f._dir.path(),
            "oxplow-bundled",
            Some(f.svc.commands.as_ref()),
        )
        .await
        .unwrap();
        assert!(v.errors.is_empty(), "{:?}", v.errors);
        assert!(v.warnings.is_empty(), "{:?}", v.warnings);
        for name in ["oxplow.review.accept", "oxplow.review.request_changes"] {
            let spec = f
                .svc
                .commands
                .spec(name)
                .unwrap_or_else(|| panic!("{name}"));
            assert_eq!(spec.confirm, oxplow_domain::Confirm::Always, "{name}");
            assert_eq!(
                (spec.invokers.human, spec.invokers.agent, spec.invokers.lens),
                (true, false, true),
                "{name}"
            );
        }
        let agent = oxplow_domain::Actor::Agent {
            session_id: None,
            thread_id: Some(f.thread),
            stream_id: None,
        };
        // Nor a lens acting for one (P7 review, tsk729): the agent policy
        // closes a command closed to agents to an agent through a lens
        // too — denied, not turned into a proposal for a person.
        let through_lens = oxplow_domain::Actor::Lens {
            lens_id: "oxplow-bundled/packet".into(),
            on_behalf_of: Box::new(agent.clone()),
        };
        for actor in [&agent, &through_lens] {
            let err = review(
                &f,
                actor,
                "oxplow.review.accept",
                serde_json::json!({ "ref": effort_ref(&f), "force": true }),
            )
            .await
            .unwrap_err();
            assert!(
                matches!(err, oxplow_domain::CommandError::Denied { .. }),
                "{actor:?}: {err:?}"
            );
        }
        assert_eq!(task_status(&f).await, "in_progress");
    }

    /// Accept refuses while a claim is unverified or an inferred decision
    /// is unreviewed; forced, it closes the task and leaves one review
    /// comment, in one run.
    #[tokio::test(flavor = "multi_thread")]
    async fn accept_refuses_unreviewed_work_unless_forced() {
        let f = review_fixture().await;
        let human = oxplow_domain::Actor::Human;
        let err = review(
            &f,
            &human,
            "oxplow.review.accept",
            serde_json::json!({ "ref": effort_ref(&f) }),
        )
        .await
        .unwrap_err();
        let oxplow_domain::CommandError::Invalid { message, .. } = &err else {
            panic!("{err:?}");
        };
        assert!(
            message.contains("1 unverified claim") && message.contains("1 inferred decision"),
            "{message}"
        );
        assert_eq!(task_status(&f).await, "in_progress");
        assert!(task_notes(&f).await.is_empty());

        let ran = review(
            &f,
            &human,
            "oxplow.review.accept",
            serde_json::json!({ "ref": effort_ref(&f), "force": true }),
        )
        .await
        .unwrap();
        assert_eq!(task_status(&f).await, "done");
        // The run's audit row says what it used: one read of the models.
        let audit_id = ran.audit_id.unwrap();
        let audit = f
            .svc
            .db
            .read(move |c| oxplow_db::command_audit_store::get_tx(c, audit_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(audit.scopes, [("sql.read".to_string(), 1)].into());
        let notes = task_notes(&f).await;
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].starts_with("Review accepted"), "{}", notes[0]);
        assert!(notes[0].contains("no behavior change"), "{}", notes[0]);
    }

    /// Once its claims are verified and its decisions confirmed, accept
    /// needs no force.
    #[tokio::test(flavor = "multi_thread")]
    async fn accept_after_review_needs_no_force() {
        let f = review_fixture().await;
        let human = oxplow_domain::Actor::Human;
        let ids: (i64, i64) = f
            .svc
            .db
            .read(|c| {
                c.query_row(
                    "SELECT (SELECT max(id) FROM claim), (SELECT max(id) FROM decision)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        review(
            &f,
            &human,
            "oxplow.effort.verify_claim",
            serde_json::json!({ "claim": format!("claim:{}", ids.0) }),
        )
        .await
        .unwrap();
        review(
            &f,
            &human,
            "oxplow.effort.confirm_decision",
            serde_json::json!({ "decision": format!("decision:{}", ids.1) }),
        )
        .await
        .unwrap();
        review(
            &f,
            &human,
            "oxplow.review.accept",
            serde_json::json!({ "ref": effort_ref(&f) }),
        )
        .await
        .unwrap();
        assert_eq!(task_status(&f).await, "done");
        assert_eq!(
            task_notes(&f).await,
            vec![format!("Review accepted ({}).", effort_ref(&f))]
        );
    }

    /// A dismissed claim doesn't count: with it dismissed and the decision
    /// confirmed, accept needs no force, and reopening it brings the
    /// refusal back.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_dismissed_claim_does_not_hold_up_accept_until_reopened() {
        let f = review_fixture().await;
        let human = oxplow_domain::Actor::Human;
        let ids: (i64, i64) = f
            .svc
            .db
            .read(|c| {
                c.query_row(
                    "SELECT (SELECT max(id) FROM claim), (SELECT max(id) FROM decision)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        let claim = serde_json::json!({ "claim": format!("claim:{}", ids.0) });
        review(
            &f,
            &human,
            "oxplow.effort.confirm_decision",
            serde_json::json!({ "decision": format!("decision:{}", ids.1) }),
        )
        .await
        .unwrap();
        review(&f, &human, "oxplow.effort.dismiss_claim", claim.clone())
            .await
            .unwrap();
        // Reopened, it is unverified again.
        review(&f, &human, "oxplow.effort.reopen_claim", claim.clone())
            .await
            .unwrap();
        let accept = serde_json::json!({ "ref": effort_ref(&f) });
        let err = review(&f, &human, "oxplow.review.accept", accept.clone())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("1 unverified claim"), "{err}");
        review(&f, &human, "oxplow.effort.dismiss_claim", claim)
            .await
            .unwrap();
        review(&f, &human, "oxplow.review.accept", accept)
            .await
            .unwrap();
        assert_eq!(task_status(&f).await, "done");
    }

    /// A shipped effect or collector that reacts to a type with more than
    /// one registered version has a fixture at an older `v` — proof it
    /// still works once the event is carried to the newest. (Today none
    /// does: every type they react to has one version registered. This
    /// fails the day one does, until the fixture is written.)
    #[test]
    fn shipped_scripts_that_react_to_a_versioned_type_prove_an_older_version() {
        use std::collections::{BTreeMap, BTreeSet};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut dirs = vec![
            root.join("extensions/oxplow-bundled"),
            root.join("tests-e2e/fixtures/extension"),
        ];
        dirs.extend(
            std::fs::read_dir(root.join("examples/extensions"))
                .unwrap()
                .flatten()
                .map(|e| e.path()),
        );
        let yaml = |p: &std::path::Path| -> Option<serde_yaml::Value> {
            serde_yaml::from_str(&std::fs::read_to_string(p).ok()?).ok()
        };
        let strings = |v: Option<&serde_yaml::Value>| -> Vec<String> {
            v.and_then(|v| v.as_sequence())
                .into_iter()
                .flatten()
                .filter_map(|s| s.as_str().map(str::to_string))
                .collect()
        };
        let mut versions: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
        for (t, v) in oxplow_domain::events::schema::EventSchemaRegistry::core().versions() {
            versions.entry(t).or_default().insert(v);
        }
        let manifests: Vec<(std::path::PathBuf, serde_yaml::Value)> = dirs
            .iter()
            .filter_map(|d| Some((d.clone(), yaml(&d.join("extension.yaml"))?)))
            .collect();
        for (_, m) in &manifests {
            for t in m["event_types"]["types"]
                .as_sequence()
                .into_iter()
                .flatten()
            {
                if let (Some(name), Some(v)) = (t["type"].as_str(), t["v"].as_u64()) {
                    versions.entry(name.into()).or_default().insert(v as u32);
                }
            }
        }
        for (dir, m) in &manifests {
            let mut reacts: Vec<(String, String)> = Vec::new();
            for e in m["effects"].as_sequence().into_iter().flatten() {
                for t in strings(e.get("on")) {
                    reacts.push((format!("effect {}", e["id"].as_str().unwrap_or("?")), t));
                }
            }
            for c in m["collectors"].as_sequence().into_iter().flatten() {
                for t in strings(c["trigger"].get("on")) {
                    reacts.push((format!("collector {}", c["id"].as_str().unwrap_or("?")), t));
                }
            }
            let mut fixtures: Vec<(String, u32)> = Vec::new();
            for f in std::fs::read_dir(dir.join("fixtures"))
                .into_iter()
                .flatten()
                .flatten()
            {
                if let Some(y) = yaml(&f.path()) {
                    let ev = &y["input"]["event"];
                    if let Some(t) = ev["type"].as_str() {
                        fixtures.push((t.into(), ev["v"].as_u64().unwrap_or(0) as u32));
                    }
                }
            }
            for (who, t) in reacts {
                let known = versions.get(&t).cloned().unwrap_or_default();
                let Some(latest) = known.iter().max().copied() else {
                    continue;
                };
                if known.len() > 1 {
                    assert!(
                        fixtures.iter().any(|(ft, fv)| *ft == t && *fv < latest),
                        "{}: {who} reacts to `{t}`, registered at versions {known:?}, with no fixture at an older `v`",
                        dir.display()
                    );
                }
            }
        }
    }

    const VERIFY: &str = "oxplow-bundled/verify-unchecked";

    /// The latest `oxplow_bundled.accepted`, delivered to the effects as
    /// the pump would.
    async fn deliver_acceptance(f: &crate::test_fixtures::EffortFixture) {
        let seq: i64 = f
            .svc
            .db
            .read(|c| {
                c.query_row(
                    "SELECT max(seq) FROM event_log WHERE type = 'oxplow_bundled.accepted'",
                    [],
                    |r| r.get(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        let ev = f
            .svc
            .event_log_store
            .read_after(seq - 1, 1)
            .await
            .unwrap()
            .remove(0);
        use crate::event_pump::AsyncEventConsumer as _;
        crate::effect_triggers::EffectTriggers::new(std::sync::Arc::downgrade(&f.svc))
            .handle(&ev)
            .await
            .unwrap();
    }

    /// The latest acceptance delivered under an id the log never had: what
    /// the effect knows of the event is what its envelope carries.
    async fn deliver_unlogged_acceptance(f: &crate::test_fixtures::EffortFixture) {
        let seq: i64 = f
            .svc
            .db
            .read(|c| {
                c.query_row(
                    "SELECT max(seq) FROM event_log WHERE type = 'oxplow_bundled.accepted'",
                    [],
                    |r| r.get(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        let mut ev = f
            .svc
            .event_log_store
            .read_after(seq - 1, 1)
            .await
            .unwrap()
            .remove(0);
        ev.envelope.id = oxplow_domain::EventId::generate();
        use crate::event_pump::AsyncEventConsumer as _;
        crate::effect_triggers::EffectTriggers::new(std::sync::Arc::downgrade(&f.svc))
            .handle(&ev)
            .await
            .unwrap();
    }

    /// The follow-ups the effect filed: (title, body, author).
    async fn follow_ups(
        f: &crate::test_fixtures::EffortFixture,
    ) -> Vec<(String, String, Option<String>)> {
        f.svc
            .db
            .read(|c| {
                let mut st = c
                    .prepare(
                        "SELECT title, description, author FROM task
                          WHERE title LIKE 'Verify what the review of %' ORDER BY id",
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = st
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .map_err(oxplow_db::map_sql_err)?;
                rows.collect::<rusqlite::Result<_>>()
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
    }

    /// A person approves the review follow-up as it is now: its bundled
    /// files, and events from now on.
    async fn approve_follow_up(f: &crate::test_fixtures::EffortFixture) {
        let (ext, decl) = crate::effect_triggers::find_effect(&f.svc, VERIFY).expect("its effect");
        let program = crate::effects::effect_program(&ext, &decl);
        let root = f.svc.layout.project_dir.clone();
        crate::exec_consent::approve_program(
            &f.svc.approvals,
            &root,
            std::slice::from_ref(&ext),
            crate::exec_consent::ProgramKind::Effect,
            &program.name,
            &program.hash(&root).unwrap(),
        )
        .unwrap();
        crate::effects::approved(&f.svc.db, VERIFY).await.unwrap();
    }

    /// P11 (tsk956): the review follow-up comes with oxplow, and runs only
    /// once a person approves it: it is listed with a version to approve,
    /// and until then a forced acceptance files nothing.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_review_follow_up_waits_for_a_persons_approval() {
        let f = review_fixture().await;
        let root = f.svc.layout.project_dir.clone();
        let programs = crate::exec_consent::list(
            &f.svc.approvals,
            &root,
            f.svc
                .extension_catalog
                .unpinned(&root, "a test reads declarations")
                .as_ref(),
        );
        let follow_up = programs
            .iter()
            .find(|p| p.kind == crate::exec_consent::ProgramKind::Effect && p.name == VERIFY)
            .expect("listed");
        assert!(!follow_up.approved);
        assert!(follow_up.version.is_some(), "it has a version to approve");
        review(
            &f,
            &oxplow_domain::Actor::Human,
            "oxplow.review.accept",
            serde_json::json!({ "ref": effort_ref(&f), "force": true }),
        )
        .await
        .unwrap();
        deliver_acceptance(&f).await;
        assert!(follow_ups(&f).await.is_empty());
    }

    /// P11 (tsk956): approved, a forced acceptance files one item to verify
    /// what it left unchecked — on the reviewed item's provider, a checklist
    /// naming each claim and decision, authored by no person — and a second
    /// forced acceptance of the same effort files nothing.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_forced_acceptance_files_one_verify_follow_up() {
        let f = review_fixture().await;
        approve_follow_up(&f).await;
        let accept = || {
            review(
                &f,
                &oxplow_domain::Actor::Human,
                "oxplow.review.accept",
                serde_json::json!({ "ref": effort_ref(&f), "force": true }),
            )
        };
        accept().await.unwrap();
        deliver_acceptance(&f).await;
        let filed = follow_ups(&f).await;
        assert_eq!(filed.len(), 1, "{filed:?}");
        let (title, body, author) = &filed[0];
        assert_eq!(
            title,
            &format!(
                "Verify what the review of {} accepted unchecked",
                effort_ref(&f)
            )
        );
        assert!(body.contains("no behavior change"), "{body}");
        assert!(
            body.contains("Which store?") && body.contains("SQLite"),
            "{body}"
        );
        assert!(body.contains("- [ ] "), "a checklist: {body}");
        assert_eq!(author, &None, "the effect's, not the person's");
        // Accepted again, forced: it was followed up already — the effect
        // skips (tsk991: a reaction that skipped, not one that failed).
        accept().await.unwrap();
        deliver_acceptance(&f).await;
        assert_eq!(follow_ups(&f).await.len(), 1);
        assert_eq!(last_reaction(&f).await, "skipped");
    }

    const REPAIR: &str = "oxplow-bundled/repair-disabled";

    /// The repair items filed for the `work`/`hot` collector: (ref, title,
    /// body).
    async fn repair_items(svc: &std::sync::Arc<crate::Services>) -> Vec<(String, String, String)> {
        let out = svc
            .sql
            .query_sql(
                "SELECT w.ref, w.title, w.body FROM v_work_item w JOIN v_work_item_link l ON l.from_ref = w.ref WHERE l.to_ref = 'collector:work/hot' AND l.link_type = 'about' ORDER BY w.ref",
                vec![],
                None,
            )
            .await
            .unwrap();
        out.rows
            .into_iter()
            .map(|r| {
                let text = |c: &oxplow_db::SqlCell| match c {
                    oxplow_db::SqlCell::Text(t) => t.clone(),
                    other => format!("{other:?}"),
                };
                (text(&r[0]), text(&r[1]), text(&r[2]))
            })
            .collect()
    }

    async fn repair_item_shown(svc: &std::sync::Arc<crate::Services>) -> oxplow_db::SqlCell {
        svc.sql
            .query_sql(
                "SELECT repair_item FROM v_contribution_health WHERE extension = 'work'",
                vec![],
                None,
            )
            .await
            .unwrap()
            .rows[0][0]
            .clone()
    }

    async fn effect_states(svc: &std::sync::Arc<crate::Services>) -> Vec<String> {
        svc.db
            .read(|c| {
                let mut st = c
                    .prepare("SELECT state FROM effect_run WHERE effect = ?1 ORDER BY id")
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = st
                    .query_map([REPAIR], |r| r.get(0))
                    .map_err(oxplow_db::map_sql_err)?;
                rows.collect::<rusqlite::Result<_>>()
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
    }

    /// A disabled contribution files one repair item whose body names what
    /// failed and what to do; a repeat disable comments on the open item; a
    /// closed one lets the next file a new item. The health view shows the
    /// open one; a redelivery files nothing.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_disabled_contribution_files_a_repair_item_and_a_repeat_comments() {
        let f = crate::test_fixtures::services_with_effort().await;
        crate::test_fixtures::approve_effect(&f.svc, REPAIR).await;

        let first = crate::test_fixtures::disable_collector(
            &f.svc,
            "work",
            "hot",
            "3 failures in a row; the last: boom",
        )
        .await;
        crate::test_fixtures::deliver(&f.svc, &first).await;
        crate::test_fixtures::deliver(&f.svc, &first).await; // a redelivery
        let items = repair_items(&f.svc).await;
        assert_eq!(items.len(), 1, "{items:?}");
        let (item, title, body) = items[0].clone();
        assert_eq!(
            title,
            "Repair work hot: 3 failures in a row; the last: boom"
        );
        assert!(
            body.contains("collector `hot` of the extension `work`")
                && body.contains("oxplow extension check work"),
            "{body}"
        );
        assert_eq!(
            repair_item_shown(&f.svc).await,
            oxplow_db::SqlCell::Text(item.clone())
        );

        let again = crate::test_fixtures::disable_collector(
            &f.svc,
            "work",
            "hot",
            "3 failures in a row; the last: boom again",
        )
        .await;
        crate::test_fixtures::deliver(&f.svc, &again).await;
        assert_eq!(
            repair_items(&f.svc).await.len(),
            1,
            "the open item is reused"
        );
        let comments = f
            .svc
            .sql
            .query_sql(
                "SELECT count(*) FROM v_event WHERE type = 'work_item.commented'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(comments.rows[0][0], oxplow_db::SqlCell::Int(1));

        // Closed: the next disable files a new one.
        f.svc
            .work_items_client()
            .transition(
                &oxplow_domain::Actor::Human,
                &item,
                oxplow_domain::work_items::CanonicalState::Done,
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            repair_item_shown(&f.svc).await,
            oxplow_db::SqlCell::Null(())
        );
        let third = crate::test_fixtures::disable_collector(
            &f.svc,
            "work",
            "hot",
            "3 failures in a row; the last: boom thrice",
        )
        .await;
        crate::test_fixtures::deliver(&f.svc, &third).await;
        assert_eq!(repair_items(&f.svc).await.len(), 2);
    }

    /// Core reports a disabled contribution whether or not the repair
    /// effect runs: until a person approves it, and once they take the
    /// approval back, no item is filed — and health still says disabled.
    #[tokio::test(flavor = "multi_thread")]
    async fn without_the_repair_effect_approved_nothing_is_filed_and_core_still_reports() {
        let f = crate::test_fixtures::services_with_effort().await;
        let before = crate::test_fixtures::disable_collector(&f.svc, "work", "hot", "boom").await;
        crate::test_fixtures::deliver(&f.svc, &before).await;
        assert!(repair_items(&f.svc).await.is_empty(), "never approved");

        crate::test_fixtures::approve_effect(&f.svc, REPAIR).await;
        crate::test_fixtures::revoke_effect(&f.svc, REPAIR);
        let after = crate::test_fixtures::disable_collector(&f.svc, "work", "hot", "boom").await;
        crate::test_fixtures::deliver(&f.svc, &after).await;
        assert!(repair_items(&f.svc).await.is_empty(), "approval taken back");

        let health = f
            .svc
            .sql
            .query_sql(
                "SELECT state, reason FROM v_contribution_health WHERE extension = 'work'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            health.rows[0][0],
            oxplow_db::SqlCell::Text("disabled".into())
        );
    }

    /// No running work list: the effect skips (not a failure that would
    /// count against it) and files nowhere.
    #[tokio::test(flavor = "multi_thread")]
    async fn with_no_running_work_list_the_repair_effect_skips() {
        let f = crate::test_fixtures::services_with_effort().await;
        f.svc
            .config
            .write()
            .unwrap()
            .active_providers
            .insert("work_items".into(), "tracker".into());
        crate::test_fixtures::activate(&f.svc).await;
        crate::test_fixtures::approve_effect(&f.svc, REPAIR).await;
        let ev = crate::test_fixtures::disable_collector(&f.svc, "work", "hot", "boom").await;
        crate::test_fixtures::deliver(&f.svc, &ev).await;
        assert!(repair_items(&f.svc).await.is_empty());
        assert_eq!(effect_states(&f.svc).await, ["skipped"]);
    }

    /// The state of the follow-up effect's latest reaction.
    async fn last_reaction(f: &crate::test_fixtures::EffortFixture) -> String {
        f.svc
            .db
            .read(|c| {
                c.query_row(
                    "SELECT state FROM effect_run WHERE effect = ?1 ORDER BY id DESC LIMIT 1",
                    [VERIFY],
                    |r| r.get(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
    }

    /// The effect takes the effort, its item and what was accepted
    /// unchecked from the event's own subject, not from a second read of the
    /// log: an event the log doesn't hold still files its follow-up.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_follow_up_reads_the_acceptance_from_its_envelope() {
        let f = review_fixture().await;
        approve_follow_up(&f).await;
        review(
            &f,
            &oxplow_domain::Actor::Human,
            "oxplow.review.accept",
            serde_json::json!({ "ref": effort_ref(&f), "force": true }),
        )
        .await
        .unwrap();
        deliver_unlogged_acceptance(&f).await;
        let filed = follow_ups(&f).await;
        assert_eq!(filed.len(), 1, "{filed:?}");
        assert!(filed[0].1.contains("- [ ] "), "{}", filed[0].1);
    }

    /// tsk991: an earlier forced acceptance stops a second follow-up only
    /// when the effect filed one for it — not one from before the effect was
    /// approved (it never reacted), which would leave the second unfollowed.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_acceptance_from_before_approval_doesnt_stop_the_follow_up() {
        let f = review_fixture().await;
        let accept = || {
            review(
                &f,
                &oxplow_domain::Actor::Human,
                "oxplow.review.accept",
                serde_json::json!({ "ref": effort_ref(&f), "force": true }),
            )
        };
        accept().await.unwrap();
        approve_follow_up(&f).await;
        accept().await.unwrap();
        deliver_acceptance(&f).await;
        assert_eq!(follow_ups(&f).await.len(), 1);
    }

    /// tsk991: what the checklist names is text: a claim statement can't
    /// add items or reshape the body, and a long one is cut.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_claim_statement_stays_one_checklist_line() {
        let f = review_fixture().await;
        let (thread, task, effort) = (f.thread, f.task, f.effort);
        f.svc
            .db
            .transaction(move |tx| {
                oxplow_db::insert_claim_tx(
                    tx,
                    oxplow_db::ClaimProvenance::Inferred,
                    &oxplow_db::NewClaim {
                        thread_id: thread.value(),
                        work_item: Some(oxplow_tasks::work_item_ref(task)),
                        effort_id: Some(effort.value()),
                        statement: format!(
                            "first line\n- [x] forged item [link](https://evil.example) {}",
                            "z".repeat(600)
                        ),
                        kind: "tests_pass".into(),
                        evidence_ref: None,
                        agent_session_id: None,
                    },
                )
            })
            .await
            .unwrap();
        approve_follow_up(&f).await;
        review(
            &f,
            &oxplow_domain::Actor::Human,
            "oxplow.review.accept",
            serde_json::json!({ "ref": effort_ref(&f), "force": true }),
        )
        .await
        .unwrap();
        deliver_acceptance(&f).await;
        let filed = follow_ups(&f).await;
        let body = &filed[0].1;
        assert!(!body.contains("\n- [x]"), "{body}");
        assert!(!body.contains("[link](https://evil.example)"), "{body}");
        assert!(body.lines().all(|l| l.chars().count() <= 400), "{body}");
    }

    /// P11 (tsk956): an acceptance that left nothing unchecked files
    /// nothing — the effect skips.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_clean_acceptance_files_nothing() {
        let f = review_fixture().await;
        approve_follow_up(&f).await;
        let human = oxplow_domain::Actor::Human;
        let ids: (i64, i64) = f
            .svc
            .db
            .read(|c| {
                c.query_row(
                    "SELECT (SELECT max(id) FROM claim), (SELECT max(id) FROM decision)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        review(
            &f,
            &human,
            "oxplow.effort.verify_claim",
            serde_json::json!({ "claim": format!("claim:{}", ids.0) }),
        )
        .await
        .unwrap();
        review(
            &f,
            &human,
            "oxplow.effort.confirm_decision",
            serde_json::json!({ "decision": format!("decision:{}", ids.1) }),
        )
        .await
        .unwrap();
        review(
            &f,
            &human,
            "oxplow.review.accept",
            serde_json::json!({ "ref": effort_ref(&f) }),
        )
        .await
        .unwrap();
        deliver_acceptance(&f).await;
        assert!(follow_ups(&f).await.is_empty());
        let state: String = f
            .svc
            .db
            .read(|c| {
                c.query_row("SELECT state FROM effect_run", [], |r| r.get(0))
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(state, "skipped");
    }

    /// Request Changes comments a checklist — each unverified claim, each
    /// inferred decision, each file outside the task's area, and the
    /// note — and moves the task back to ready.
    #[tokio::test(flavor = "multi_thread")]
    async fn request_changes_lists_what_to_fix_and_reopens_the_task() {
        let f = review_fixture().await;
        review(
            &f,
            &oxplow_domain::Actor::Human,
            "oxplow.review.request_changes",
            serde_json::json!({ "ref": effort_ref(&f), "note": "Keep it to the UI." }),
        )
        .await
        .unwrap();
        assert_eq!(task_status(&f).await, "ready");
        let notes = task_notes(&f).await;
        assert_eq!(notes.len(), 1, "{notes:?}");
        let body = &notes[0];
        for line in [
            "Changes requested",
            "Keep it to the UI.",
            "- [ ] Back up the claim: no behavior change",
            "- [ ] Confirm or rework the decision: Which store? → SQLite",
            "- [ ] Explain or revert the change outside the task's area: crates/db/store.rs",
        ] {
            assert!(body.contains(line), "{line}\n---\n{body}");
        }
        assert!(!body.contains("src/ui/button.ts"), "{body}");
    }

    /// A claim is agent-written text: the review's comment holds it as one
    /// inert line, so it can't add headings, links or items to the
    /// reviewer's comment — on Accept and on Request Changes alike.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_review_comments_hold_a_claim_as_inert_text() {
        for (command, force) in [
            ("oxplow.review.accept", true),
            ("oxplow.review.request_changes", false),
        ] {
            let f = review_fixture().await;
            let (thread, task, effort) = (f.thread, f.task, f.effort);
            f.svc
                .db
                .transaction(move |tx| {
                    oxplow_db::insert_claim_tx(
                        tx,
                        oxplow_db::ClaimProvenance::Inferred,
                        &oxplow_db::NewClaim {
                            thread_id: thread.value(),
                            work_item: Some(oxplow_tasks::work_item_ref(task)),
                            effort_id: Some(effort.value()),
                            statement: "tests pass\n# Approved\n- [x] [ship it](https://x.test)"
                                .into(),
                            kind: "tests_pass".into(),
                            evidence_ref: None,
                            agent_session_id: None,
                        },
                    )
                })
                .await
                .unwrap();
            let mut input = serde_json::json!({ "ref": effort_ref(&f) });
            if force {
                input["force"] = serde_json::json!(true);
            }
            review(&f, &oxplow_domain::Actor::Human, command, input)
                .await
                .unwrap();
            let notes = task_notes(&f).await;
            let body = notes.last().unwrap();
            assert!(
                body.contains(r"tests pass \# Approved - \[x\] \[ship it\](https://x.test)"),
                "{command}\n---\n{body}"
            );
            assert!(
                !body
                    .lines()
                    .any(|l| l.starts_with('#') || l.starts_with("- [x]")),
                "{command}\n---\n{body}"
            );
        }
    }

    /// The packet's rows carry their reviews: Mark Verified on an
    /// unverified claim, with Dismiss and Reopen beside it; Confirm and
    /// Dismiss on an inferred decision.
    #[test]
    fn the_review_lenses_declare_their_row_actions() {
        let exts = crate::extensions::load_extensions(std::path::Path::new("/nonexistent"));
        let ext = exts.iter().find(|e| e.name == "oxplow-bundled").unwrap();
        let actions = |slug: &str| {
            ext.lenses
                .iter()
                .find(|l| l.id == format!("oxplow-bundled/{slug}"))
                .unwrap_or_else(|| panic!("{slug}"))
                .actions
                .iter()
                .filter(|a| a.row)
                .map(|a| (a.label.clone(), a.command.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            actions("unverified-claims"),
            vec![
                (
                    "Mark Verified".to_string(),
                    "oxplow.effort.verify_claim".to_string()
                ),
                (
                    "Dismiss".to_string(),
                    "oxplow.effort.dismiss_claim".to_string()
                ),
                (
                    "Reopen".to_string(),
                    "oxplow.effort.reopen_claim".to_string()
                ),
            ]
        );
        assert_eq!(
            actions("inferred-decisions"),
            vec![
                (
                    "Confirm".to_string(),
                    "oxplow.effort.confirm_decision".to_string()
                ),
                (
                    "Dismiss".to_string(),
                    "oxplow.effort.dismiss_decision".to_string()
                ),
            ]
        );
        assert!(ext
            .lenses
            .iter()
            .any(|l| l.id == "oxplow-bundled/verify-claim-with-evidence"));
        assert!(ext.commands.iter().any(|c| c.name == "oxplow.review.accept"
            && c.ui.as_ref().is_some_and(
                |u| u.label == "Accept Review" && u.about.as_deref() == Some("effort")
            )));
    }

    /// P7.B5: the "look here first" score is oxplow-bundled's model over
    /// core's change rows, the same formula core used to store — size,
    /// complexity spikes, parameter growth and long new functions,
    /// multiplied — and it has left `v_change_file`.
    #[tokio::test]
    async fn the_interest_model_scores_a_change_like_the_old_formula() {
        let f = crate::test_fixtures::services_with_effort().await;
        crate::test_fixtures::activate_with_problems(&f.svc).await;
        f.svc
            .db
            .transaction(|tx| {
                tx.execute_batch(
                    "INSERT INTO change (id, stream_id, kind, target, status) VALUES (90, 1, 'commit', 'abc', 'done');
                     INSERT INTO change_file (change_id, path, status, additions, deletions, zone, is_test)
                       VALUES (90, 'src/hot.rs', 'modified', 10, 2, 'other', 0),
                              (90, 'src/calm.rs', 'modified', 3, 1, 'other', 0),
                              (90, 'src/big.rs', 'modified', 40, 20, 'other', 0);
                     INSERT INTO change_function (change_id, path, ref, container, name, status, signature_changed,
                         body_changed, start_line, visibility, is_test, length, params_before, params_after,
                         complexity_delta)
                       VALUES (90, 'src/hot.rs', 'symbol:src/hot.rs/grow', '', 'grow', 'modified', 0, 1, 1, 'public', 0, 20, 1, 1, 3),
                              (90, 'src/hot.rs', 'symbol:src/hot.rs/widen', '', 'widen', 'modified', 1, 0, 30, 'public', 0, 10, 1, 3, NULL),
                              (90, 'src/hot.rs', 'symbol:src/hot.rs/fresh', '', 'fresh', 'added', 0, 0, 50, 'public', 0, 70, NULL, 0, NULL);",
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        let rows = f
            .svc
            .sql
            .query_sql(
                "SELECT path, interest, reasons FROM v_oxplow_bundled_change_interest
                 WHERE change_id = 90 ORDER BY path",
                vec![],
                None,
            )
            .await
            .unwrap();
        let row = |path: &str| {
            rows.rows
                .iter()
                .find(|r| r[0] == oxplow_db::SqlCell::Text(path.into()))
                .unwrap_or_else(|| panic!("{path}"))
                .clone()
        };
        let score = |r: &Vec<oxplow_db::SqlCell>| match r[1] {
            oxplow_db::SqlCell::Real(x) => x,
            ref other => panic!("{other:?}"),
        };
        let reasons = |r: &Vec<oxplow_db::SqlCell>| match &r[2] {
            oxplow_db::SqlCell::Text(t) => t.clone(),
            other => panic!("{other:?}"),
        };
        // (1 + log2(13)) × (1 + 0.6·3) × (1 + 0.4·2) × (1 + (70 − 60)/40)
        let hot = row("src/hot.rs");
        assert!((score(&hot) - 29.6126).abs() < 0.001, "{hot:?}");
        assert_eq!(
            reasons(&hot),
            "complexity +3 across 1 fn; +2 params across 1 fn; added 70-line function"
        );
        let calm = row("src/calm.rs");
        assert!(score(&calm) < 4.0, "{calm:?}");
        assert_eq!(reasons(&calm), "");
        assert_eq!(reasons(&row("src/big.rs")), "60 lines touched");

        let gone = f
            .svc
            .sql
            .query_sql("SELECT interest FROM v_change_file", vec![], None)
            .await;
        assert!(gone.is_err(), "v_change_file still has interest");
    }

    /// P7.B5: co-change surprises are oxplow-bundled's models over the
    /// commit index — `co_change_pair` (materialized: files committed
    /// together at least 3 times in 180 days, commits of 50 files or
    /// fewer) and `change_co_change` (a change's files whose usual
    /// partners are missing, or that were dormant 90 days or more) — and
    /// core no longer computes them.
    #[tokio::test]
    async fn co_change_surprises_come_from_the_analytics_models() {
        let f = crate::test_fixtures::services_with_effort().await;
        f.svc
            .db
            .transaction(|tx| {
                let commit = |sha: &str, days_ago: i64, paths: &[&str]| -> rusqlite::Result<()> {
                    tx.execute(
                        "INSERT INTO git_commit (sha, author, email, committed_at, subject)
                         VALUES (?1, 'a', 'a@x', strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2), 's')",
                        rusqlite::params![sha, format!("-{days_ago} days")],
                    )?;
                    for p in paths {
                        tx.execute(
                            "INSERT INTO git_commit_file (sha, path, status) VALUES (?1, ?2, 'modified')",
                            rusqlite::params![sha, p],
                        )?;
                    }
                    Ok(())
                };
                (|| {
                    for (i, days) in [5, 10, 20].iter().enumerate() {
                        commit(&format!("ab{i}"), *days, &["src/a.rs", "src/b.rs"])?;
                        commit(&format!("ef{i}"), *days, &["src/e.rs", "src/f.rs"])?;
                    }
                    commit("old", 200, &["src/c.rs"])?;
                    tx.execute_batch(
                        "INSERT INTO change (id, stream_id, kind, target, status) VALUES (91, 1, 'working', '', 'done');
                         INSERT INTO change_file (change_id, path, status, additions, deletions, zone, is_test)
                           VALUES (91, 'src/a.rs', 'modified', 1, 0, 'other', 0),
                                  (91, 'src/c.rs', 'modified', 1, 0, 'other', 0),
                                  (91, 'src/d.rs', 'added', 1, 0, 'other', 0),
                                  (91, 'src/e.rs', 'modified', 1, 0, 'other', 0),
                                  (91, 'src/f.rs', 'modified', 1, 0, 'other', 0);",
                    )
                })()
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        crate::test_fixtures::activate_with_problems(&f.svc).await;
        f.svc.assets.sync_models().await.unwrap();
        let read = || async {
            f.svc
                .sql
                .query_sql(
                    "SELECT path, reason, expected, dormant_days FROM v_oxplow_bundled_change_co_change
                     WHERE change_id = 91 ORDER BY path",
                    vec![],
                    None,
                )
                .await
                .map(|r| serde_json::to_value(&r.rows).unwrap())
        };
        // The pairs fill on the materialized model's first recompute.
        let rows = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Ok(rows) = read().await {
                    if rows.as_array().is_some_and(|r| r.len() == 3) {
                        return rows;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the co-change models filled");
        assert_eq!(rows[0][0], "src/a.rs");
        assert_eq!(rows[0][1], "usual-co-changers-absent");
        assert_eq!(rows[0][2], "src/b.rs");
        assert_eq!(rows[1][0], "src/c.rs");
        assert_eq!(rows[1][1], "dormant");
        assert!(rows[1][3].as_i64().unwrap() >= 199, "{rows}");
        assert_eq!(rows[2][0], "src/d.rs", "never touched is dormant");
        assert_eq!(rows[2][3], 90);

        let gone = f
            .svc
            .sql
            .query_sql("SELECT * FROM v_change_co_change", vec![], None)
            .await;
        assert!(gone.is_err(), "core still has v_change_co_change");
    }

    /// P7.B5: an effort's churn is a fact oxplow-bundled records when the
    /// effort finishes, from the effort's change rows (its collector
    /// `effort_churn`, `on: effort.finished`, after `change.analyze`).
    #[tokio::test]
    async fn effort_churn_is_a_fact_from_the_efforts_change() {
        let f = crate::test_fixtures::services_with_effort().await;
        let effort = f.effort.value();
        f.svc
            .db
            .transaction(move |tx| {
                tx.execute(
                    "INSERT INTO change (id, stream_id, kind, target, status) VALUES (92, 1, 'effort', ?1, 'done')",
                    [effort.to_string()],
                )
                .and_then(|_| {
                    tx.execute_batch(
                        "INSERT INTO change_file (change_id, path, status, additions, deletions, zone, is_test)
                           VALUES (92, 'src/a.rs', 'modified', 10, 4, 'other', 0),
                                  (92, 'src/b.rs', 'added', 6, 0, 'other', 0);",
                    )
                })
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        f.svc
            .metrics
            .run_effort_collectors(&f.thread, &f.effort, None, &|_| true)
            .await;
        let rows = f
            .svc
            .sql
            .query_sql(
                "SELECT value FROM v_fact WHERE measure_key = 'oxplow_bundled.effort_churn_lines'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&rows.rows).unwrap(),
            serde_json::json!([[20.0]])
        );
    }

    /// oxplow-foundation's `oxplow.effort_test_outcome` collector writes, when
    /// an effort finishes, the very facts core's lifecycle producer wrote
    /// (pinned as values in `the_producer_writes_these_outcome_facts_for_seeded_runs`
    /// when that producer existed): same measures, subject, dims and values,
    /// a record-only run ignored, asserted counts anonymous. Its capture is the
    /// effort's and carries the branch, so `oxplow.tests.*` slice by it.
    #[tokio::test]
    async fn the_test_outcome_collector_writes_what_the_producer_did() {
        use crate::foundation_collectors_tests::{
            asserted_runs, expected_outcome, outcome_facts, red_then_green_runs, seed_test_runs,
        };
        for (runs, stats, ttg) in [
            (red_then_green_runs(), [0, 2, 2, 2], Some(60_250)),
            (asserted_runs(), [0, 3, 3, 1], Some(4_000)),
        ] {
            let f = crate::test_fixtures::services_with_effort().await;
            let facts = &f.svc.fact_store;
            seed_test_runs(facts, 1, f.thread.value(), f.effort.value(), "main", &runs).await;
            f.svc
                .metrics
                .run_effort_collectors(&f.thread, &f.effort, None, &|c| {
                    c.key == "oxplow.effort_test_outcome"
                })
                .await;
            assert_eq!(
                outcome_facts(facts).await,
                expected_outcome(
                    &oxplow_domain::refs::build::effort_ref(f.effort),
                    stats,
                    ttg
                )
            );
            let captures = facts.captures_for_effort(f.effort.value()).await.unwrap();
            let ours = captures
                .iter()
                .find(|c| c.producer == "oxplow.effort_test_outcome")
                .expect("the collector's capture is the effort's");
            assert!(ours.branch.is_some(), "it carries the branch: {ours:?}");
        }
    }

    /// P9.D6: a verdict is a typed event, not only a comment's text. Each
    /// review verb logs its own type (`oxplow_bundled.accepted@1`,
    /// `.changes_requested@1`) with its run — caused by its
    /// `command.executed`, about the effort and its work item — so the
    /// effort's timeline carries who decided what, and other extensions
    /// can react to it. The verdict is in the envelope, which retention
    /// keeps (tsk886): an acceptance's subject also names each claim and
    /// decision it accepted unchecked.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_review_verbs_log_a_typed_verdict() {
        let f = review_fixture().await;
        let review_ext = f
            .svc
            .listed_extensions(f._dir.path())
            .await
            .into_iter()
            .find(|e| e.name == "oxplow-bundled")
            .unwrap();
        assert_eq!(review_ext.errors, Vec::<String>::new());
        assert_eq!(
            review_ext
                .event_types
                .types
                .iter()
                .map(|t| (t.event_type.as_str(), t.v))
                .collect::<Vec<_>>(),
            vec![
                ("oxplow_bundled.accepted", 1),
                ("oxplow_bundled.changes_requested", 1),
                ("oxplow_bundled.finished_cleared", 1)
            ],
            "a shared extension's event types load"
        );
        let human = oxplow_domain::Actor::Human;
        let verdicts = || async {
            let out = f
                .svc
                .sql
                .query_sql(
                    "SELECT v.payload, v.source, v.subject, \
                            (SELECT c.type FROM v_event c WHERE c.id = v.cause), v.type \
                       FROM v_event v WHERE v.type LIKE 'oxplow_bundled.%' ORDER BY v.seq",
                    vec![],
                    None,
                )
                .await
                .unwrap();
            serde_json::to_value(out.rows).unwrap()
        };
        let item = oxplow_tasks::work_item_ref(f.task);

        review(
            &f,
            &human,
            "oxplow.review.request_changes",
            serde_json::json!({ "ref": effort_ref(&f), "note": "Keep it to the UI." }),
        )
        .await
        .unwrap();
        let logged = verdicts().await;
        assert_eq!(logged.as_array().unwrap().len(), 1, "{logged}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(logged[0][0].as_str().unwrap()).unwrap(),
            serde_json::json!({
                "effort": effort_ref(&f), "work_item": item,
                "unverified": 1, "inferred": 1, "deviated": 1,
                "note": "Keep it to the UI."
            })
        );
        assert_eq!(logged[0][4], "oxplow_bundled.changes_requested");
        assert_eq!(logged[0][1], "command:oxplow.review.request_changes");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(logged[0][2].as_str().unwrap()).unwrap(),
            serde_json::json!([effort_ref(&f), item])
        );
        assert_eq!(logged[0][3], "command.executed");

        review(
            &f,
            &human,
            "oxplow.review.accept",
            serde_json::json!({ "ref": effort_ref(&f), "force": true }),
        )
        .await
        .unwrap();
        let logged = verdicts().await;
        assert_eq!(logged.as_array().unwrap().len(), 2, "{logged}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(logged[1][0].as_str().unwrap()).unwrap(),
            serde_json::json!({ "effort": effort_ref(&f), "work_item": item, "unverified": 1, "inferred": 1, "deviated": 1 })
        );
        assert_eq!(logged[1][1], "command:oxplow.review.accept");
        assert_eq!(logged[1][4], "oxplow_bundled.accepted");
        let subject: Vec<String> = serde_json::from_str(logged[1][2].as_str().unwrap()).unwrap();
        assert_eq!(subject[..2], [effort_ref(&f), item.clone()]);
        assert!(
            subject[2..].iter().any(|r| r.starts_with("claim:"))
                && subject[2..].iter().any(|r| r.starts_with("decision:")),
            "a forced acceptance names what it accepted unchecked: {subject:?}"
        );
        // A refused review decides nothing: no verdict.
        let again = review(
            &f,
            &human,
            "oxplow.review.accept",
            serde_json::json!({ "ref": "effort:eff999" }),
        )
        .await;
        assert!(again.is_err());
        assert_eq!(verdicts().await.as_array().unwrap().len(), 2);
    }

    /// P10 (K1): oxplow-bundled shows the verdict where the effort is shown
    /// — the first bundled use of `ui.decorators`, which made it stable.
    /// Its `verdict` model is each effort's latest verdict (over
    /// `verdicts`, appended as each lands), as a label and a color the
    /// effort's chip and rows show. It shows; it never acts.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_verdict_decorates_its_effort() {
        let f = review_fixture().await;
        let review_ext = f
            .svc
            .listed_extensions(f._dir.path())
            .await
            .into_iter()
            .find(|e| e.name == "oxplow-bundled")
            .unwrap();
        assert_eq!(review_ext.errors, Vec::<String>::new());
        use crate::extensions::decorators::DecoratorPlacement;
        let decorators: Vec<(&str, &str, DecoratorPlacement)> = review_ext
            .ui
            .decorators
            .iter()
            .map(|d| (d.view.as_str(), d.kind.as_str(), d.placement))
            .collect();
        assert_eq!(
            decorators,
            vec![
                (
                    "v_oxplow_bundled_verdict",
                    "effort",
                    DecoratorPlacement::RefChip
                ),
                (
                    "v_oxplow_bundled_verdict",
                    "effort",
                    DecoratorPlacement::RowBadge
                )
            ]
        );
        // What keeps `verdicts` current in the app (boot starts it): each
        // verdict appended to the log lands in the model.
        crate::models_changed::spawn(
            f.svc.db.clone(),
            std::sync::Arc::new(crate::models_changed::ModelWatermarks::default()),
            f.svc.events.clone(),
            f.svc.assets.clone(),
            f.svc.event_pump.clone(),
        );
        let shown = || async {
            f.svc
                .sql
                .query_sql(
                    "SELECT ref, label, color FROM v_oxplow_bundled_verdict",
                    vec![],
                    None,
                )
                .await
                .map(|r| serde_json::to_value(&r.rows).unwrap())
                .unwrap_or_default()
        };
        let until = |want: serde_json::Value| async move {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    let rows = shown().await;
                    if rows == want {
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            })
            .await
            .unwrap_or_else(|_| panic!("never showed {want}"))
        };
        let human = oxplow_domain::Actor::Human;
        review(
            &f,
            &human,
            "oxplow.review.request_changes",
            serde_json::json!({ "ref": effort_ref(&f), "note": "Keep it to the UI." }),
        )
        .await
        .unwrap();
        until(serde_json::json!([[
            effort_ref(&f),
            "Changes requested",
            "red"
        ]]))
        .await;
        review(
            &f,
            &human,
            "oxplow.review.accept",
            serde_json::json!({ "ref": effort_ref(&f), "force": true }),
        )
        .await
        .unwrap();
        until(serde_json::json!([[
            effort_ref(&f),
            "Accepted (forced)",
            "orange"
        ]]))
        .await;
        // tsk886: the verdict is state. Its payload expires with the
        // extension default's 30 days, but what the chip shows is in the envelope
        // (type and subject), which is kept.
        let later = oxplow_domain::Timestamp::from_unix_ms(
            oxplow_domain::Timestamp::now().unix_ms() + 31 * oxplow_db::event_retention::DAY_MS,
        );
        let swept = oxplow_db::event_retention::sweep(&f.svc.db, later, &Default::default())
            .await
            .unwrap();
        assert!(swept.payloads_expired >= 2, "{swept:?}");
        // Refilled whole, as a restart or a rewrite of the log does.
        let computed = || async {
            f.svc
                .db
                .transaction(|tx| {
                    tx.query_row(
                        "SELECT computed_at FROM asset_state \
                         WHERE asset = 'v_oxplow_bundled_verdicts'",
                        [],
                        |r| r.get::<_, String>(0),
                    )
                    .map_err(oxplow_db::map_sql_err)
                })
                .await
                .unwrap()
        };
        let before = computed().await;
        f.svc.assets.all_changed();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while computed().await == before {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the verdicts refilled");
        assert_eq!(
            shown().await,
            serde_json::json!([[effort_ref(&f), "Accepted (forced)", "orange"]])
        );
    }
}
