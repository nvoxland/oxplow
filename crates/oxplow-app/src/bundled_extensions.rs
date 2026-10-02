//! Extensions that ship inside oxplow, compiled in from the repo's
//! `extensions/<name>/` folder. They load exactly like project extensions
//! (same format, same loader) but are read-only, and their names are
//! reserved. See `.context/extensions.md`.

/// One bundled extension: its name and its files as
/// `(path inside the extension folder, contents)`.
pub struct BundledExtension {
    pub name: &'static str,
    pub files: &'static [(&'static str, &'static str)],
}

macro_rules! ext_file {
    ($ext:literal, $path:literal) => {
        (
            $path,
            include_str!(concat!("../../../extensions/", $ext, "/", $path)),
        )
    };
}

pub const BUNDLED: &[BundledExtension] = &[
    BundledExtension {
        name: "oxplow-analytics",
        files: &[
            ext_file!("oxplow-analytics", "extension.yaml"),
            ext_file!("oxplow-analytics", "lenses/backlog-tasks.yaml"),
            ext_file!("oxplow-analytics", "lenses/change-co-change.yaml"),
            ext_file!("oxplow-analytics", "lenses/change-cross-zone-imports.yaml"),
            ext_file!("oxplow-analytics", "lenses/change-duplicates.yaml"),
            ext_file!("oxplow-analytics", "lenses/change-functions.yaml"),
            ext_file!("oxplow-analytics", "lenses/change-look-here.yaml"),
            ext_file!("oxplow-analytics", "lenses/change-review.yaml"),
            ext_file!("oxplow-analytics", "lenses/change-summary.yaml"),
            ext_file!("oxplow-analytics", "lenses/change-test-files.yaml"),
            ext_file!("oxplow-analytics", "lenses/change-treemap.yaml"),
            ext_file!("oxplow-analytics", "lenses/duplicate-blocks.yaml"),
            ext_file!("oxplow-analytics", "lenses/effort-analysis-findings.yaml"),
            ext_file!("oxplow-analytics", "lenses/effort-coverage.yaml"),
            ext_file!("oxplow-analytics", "lenses/effort-failed-tests.yaml"),
            ext_file!("oxplow-analytics", "lenses/effort-metric-deltas.yaml"),
            ext_file!("oxplow-analytics", "lenses/effort-nudges.yaml"),
            ext_file!("oxplow-analytics", "lenses/effort-test-runs.yaml"),
            ext_file!("oxplow-analytics", "lenses/effort-tests.yaml"),
            ext_file!("oxplow-analytics", "lenses/effort-untested-files.yaml"),
            ext_file!("oxplow-analytics", "lenses/findings.yaml"),
            ext_file!("oxplow-analytics", "lenses/page-visits-by-day.yaml"),
            ext_file!("oxplow-analytics", "lenses/planning.yaml"),
            ext_file!("oxplow-analytics", "lenses/quality.yaml"),
            ext_file!("oxplow-analytics", "lenses/ready-tasks.yaml"),
            ext_file!("oxplow-analytics", "lenses/recent-notes.yaml"),
            ext_file!("oxplow-analytics", "lenses/recent-snapshots.yaml"),
            ext_file!("oxplow-analytics", "lenses/review.yaml"),
            ext_file!("oxplow-analytics", "lenses/task-token-summary.yaml"),
            ext_file!("oxplow-analytics", "lenses/task-tokens.yaml"),
            ext_file!("oxplow-analytics", "lenses/task-turns.yaml"),
            ext_file!("oxplow-analytics", "lenses/thread-tokens.yaml"),
            ext_file!("oxplow-analytics", "lenses/token-total.yaml"),
            ext_file!("oxplow-analytics", "lenses/tokens-by-agent.yaml"),
            ext_file!("oxplow-analytics", "lenses/tokens-by-day.yaml"),
            ext_file!("oxplow-analytics", "lenses/top-pages.yaml"),
            ext_file!("oxplow-analytics", "lenses/usage.yaml"),
        ],
    },
    BundledExtension {
        name: "oxplow-review",
        files: &[
            ext_file!("oxplow-review", "README.md"),
            ext_file!("oxplow-review", "extension.yaml"),
            ext_file!("oxplow-review", "handlers/accept.star"),
            ext_file!("oxplow-review", "handlers/request_changes.star"),
            ext_file!("oxplow-review", "models/deviation.sql"),
            ext_file!("oxplow-review", "lenses/context-read.yaml"),
            ext_file!("oxplow-review", "lenses/decisions.yaml"),
            ext_file!("oxplow-review", "lenses/inferred-decisions.yaml"),
            ext_file!("oxplow-review", "lenses/recent-decisions.yaml"),
            ext_file!("oxplow-review", "lenses/review-prompt.yaml"),
            ext_file!("oxplow-review", "lenses/struggled.yaml"),
            ext_file!("oxplow-review", "lenses/tests-weakened.yaml"),
            ext_file!("oxplow-review", "lenses/unbacked-claims.yaml"),
            ext_file!("oxplow-review", "lenses/unverified-claims.yaml"),
            ext_file!("oxplow-review", "lenses/verify-claim-with-evidence.yaml"),
            ext_file!("oxplow-review", "lenses/waiting-on-me.yaml"),
            ext_file!("oxplow-review", "lenses/what-deviated.yaml"),
            ext_file!("oxplow-review", "questions.yaml"),
        ],
    },
];

pub fn find(name: &str) -> Option<&'static BundledExtension> {
    BUNDLED.iter().find(|b| b.name == name)
}

pub fn is_reserved(name: &str) -> bool {
    find(name).is_some()
        // A collector's owner names its extension, the project or oxplow.
        || [oxplow_config::collectors::PROJECT, oxplow_config::collectors::BUILT_IN].contains(&name)
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
        assert!(shown.contains(&"oxplow-review/waiting-on-me".to_string()));
        assert!(
            shown
                .iter()
                .filter(|id| id.starts_with("oxplow-review/"))
                .count()
                >= 2,
            "the review category has something to open: {shown:?}"
        );
    }

    /// The analytics effort-review lenses unnest the stored observation
    /// payloads (JUnit cases, uncovered lines) in SQL; run them on real rows.
    #[tokio::test]
    async fn analytics_effort_lenses_read_the_observation_payloads() {
        let f = crate::test_fixtures::services_with_effort().await;
        let obs = |seq: i64, kind: &str, value: Option<f64>, payload: serde_json::Value| {
            oxplow_db::EffortObservation {
                id: seq,
                stream_id: "1".into(),
                effort_id: f.effort.to_string(),
                kind: kind.into(),
                provenance: "observed".into(),
                source: "post-tool-bash".into(),
                metric_value: value,
                payload_json: Some(payload.to_string()),
                local_snapshot_id: None,
                closest_vcs_rev: None,
                vcs_rev_exact: false,
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
            .replace_observations(
                f.effort.value(),
                vec![
                    obs(1, "test-run", None, run("failed")),
                    obs(2, "test-run", None, run("passed")),
                    obs(
                        3,
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

    /// P4.1 (tsk486): every query a bundled extension ships — lens,
    /// advisory, collector or command input — reads only published views
    /// (its own models' included), never a physical table (what the
    /// authorizer will refuse, P4.3).
    #[tokio::test]
    async fn bundled_queries_read_only_published_views() {
        let f = crate::test_fixtures::services_with_effort().await;
        f.svc.extension_models.sync().await.unwrap();
        let bundled: Vec<&str> = super::BUNDLED.iter().map(|b| b.name).collect();
        let mut checked = 0;
        for ext in f.svc.extension_catalog.get(f._dir.path()).iter() {
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
                        .clone()
                        .map(|q| (format!("collector {} input", s.id), q))
                }))
                .chain(ext.commands.iter().filter_map(|c| {
                    c.input
                        .clone()
                        .map(|q| (format!("command {} input", c.name), q))
                }));
            for (what, sql) in queries {
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

    /// Run an oxplow-analytics lens with one integer param; its rows as JSON.
    async fn run_analytics_lens(
        f: &crate::test_fixtures::EffortFixture,
        slug: &str,
        param: &str,
        value: i64,
    ) -> serde_json::Value {
        run_bundled_lens(f, &format!("oxplow-analytics/{slug}"), &[(param, value)]).await
    }

    /// Run any bundled lens (`<extension>/<slug>`) with integer params; its
    /// rows as JSON.
    async fn run_bundled_lens(
        f: &crate::test_fixtures::EffortFixture,
        id: &str,
        params: &[(&str, i64)],
    ) -> serde_json::Value {
        let layer = crate::sql_gateway::SqlGateway::new(f.svc.db.clone());
        let params = params
            .iter()
            .map(|(k, v)| (k.to_string(), oxplow_db::SqlCell::Int(*v)))
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

    /// Token usage moved from the task/thread widgets to slot lenses.
    #[tokio::test]
    async fn analytics_token_lenses_sum_a_tasks_and_a_threads_turns() {
        let f = crate::test_fixtures::services_with_effort().await;
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

        let task = f.task.value();
        assert_eq!(
            run_analytics_lens(&f, "task-token-summary", "task_id", task).await,
            serde_json::json!([[
                "**2,100** tokens · 2 turns · in 2,098 · out 2 · cache-write 0 · cache-read 0"
            ]])
        );
        let turns = run_analytics_lens(&f, "task-turns", "task_id", task).await;
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
        let tests = run_analytics_lens(&f, "change-test-files", "change_id", change.id).await;
        assert_eq!(tests[0][0], serde_json::json!("tests/it.rs"));
    }

    /// Waiting on Me lists threads whose agent asked the user a question
    /// (`await_user`) that no later prompt has answered.
    #[tokio::test]
    async fn waiting_on_me_lists_unanswered_questions() {
        use oxplow_domain::stores::AgentTurnStore as _;
        let f = crate::test_fixtures::services_with_effort().await;
        let turn = |at: oxplow_domain::Timestamp| oxplow_domain::AgentTurn {
            id: oxplow_domain::AgentTurnId::placeholder(),
            thread_id: f.thread,
            prompt: "do it".into(),
            answer: None,
            session_id: None,
            started_at: at,
            ended_at: None,
            start_snapshot_id: None,
            snapshot_id: None,
        };
        let now = oxplow_domain::Timestamp::now();
        f.svc
            .agent_turn_store
            .open(&turn(oxplow_domain::Timestamp::from_unix_ms(
                now.unix_ms() - 60_000,
            )))
            .await
            .unwrap();
        f.svc
            .tool_call_store
            .record(oxplow_db::NewToolCall {
                thread_id: f.thread.value(),
                effort_id: None,
                tool: "mcp__oxplow__await_user".into(),
                path: None,
                detail: Some("Pick A or B?".into()),
                ok: Some(true),
                ..Default::default()
            })
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
        let rows = run_bundled_lens(&f, "oxplow-review/waiting-on-me", &[]).await;
        assert_eq!(waiting(rows), vec![serde_json::json!("Pick A or B?")]);

        // The user answers: a new turn starts after the question.
        f.svc
            .agent_turn_store
            .open(&turn(oxplow_domain::Timestamp::from_unix_ms(
                now.unix_ms() + 60_000,
            )))
            .await
            .unwrap();
        let rows = run_bundled_lens(&f, "oxplow-review/waiting-on-me", &[]).await;
        assert!(waiting(rows).is_empty());
    }

    /// What Deviated: the effort's files outside the area its task names.
    #[tokio::test]
    async fn what_deviated_lists_files_outside_the_tasks_stated_area() {
        use oxplow_db::EffortStore as _;
        use oxplow_domain::stores::TaskStore as _;
        let f = crate::test_fixtures::services_with_effort().await;
        f.svc.extension_models.sync().await.unwrap();
        let describe = |text: &'static str| {
            let svc = f.svc.clone();
            let task = f.task;
            async move {
                let mut t = svc.task_store.get(task).await.unwrap().unwrap();
                t.description = text.into();
                svc.task_store.update(&t).await.unwrap();
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
        let lens = "oxplow-review/what-deviated";
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
            "oxplow-review/tests-weakened",
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
        use oxplow_domain::stores::TaskStore as _;
        let f = crate::test_fixtures::services_with_effort().await;
        let mut t = f.svc.task_store.get(f.task).await.unwrap().unwrap();
        t.title = "Fix the hover state".into();
        t.description = "Buttons flicker on hover.".into();
        f.svc.task_store.update(&t).await.unwrap();
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
            "oxplow-review/review-prompt",
            &[("effort_id", f.effort.value())],
        )
        .await;
        let prompt = rows[0][0].as_str().unwrap();
        for expected in [
            "Fix the hover state",
            "Buttons flicker on hover.",
            "- `src/ui/button.ts` (updated)",
            "No claims were recorded.",
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
            .start("work_item:linear:ENG-1", &f.thread, None)
            .await
            .unwrap();
        let rows = run_bundled_lens(
            &f,
            "oxplow-review/review-prompt",
            &[("effort_id", effort.id.value())],
        )
        .await;
        let prompt = rows[0][0]
            .as_str()
            .unwrap_or_else(|| panic!("no prompt: {rows}"));
        assert!(
            prompt.contains("**work_item:linear:ENG-1**"),
            "missing the work item in:\n{prompt}"
        );
    }

    /// The stream-level review starters list the viewer's stream's
    /// decisions and unbacked claims, newest first (tsk374).
    #[tokio::test]
    async fn review_starters_read_the_viewers_stream() {
        let f = crate::test_fixtures::services_with_effort().await;
        let thread = f.thread.value();
        let store = &f.svc.reasoning_store;
        store
            .record_decision(oxplow_db::NewDecision {
                thread_id: thread,
                task_id: Some(f.task.value()),
                effort_id: Some(f.effort.value()),
                question: "Where does export live?".into(),
                choice: "src/export".into(),
                alternatives: vec![],
                confidence: "high".into(),
                why: String::new(),
            })
            .await
            .unwrap();
        for (statement, evidence) in [("tests pass", None), ("handles empty", Some("test:empty"))] {
            store
                .record_claim(oxplow_db::NewClaim {
                    thread_id: thread,
                    task_id: Some(f.task.value()),
                    effort_id: Some(f.effort.value()),
                    statement: statement.into(),
                    kind: "tests_pass".into(),
                    evidence_ref: evidence.map(str::to_string),
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
        let decisions = first("oxplow-review/recent-decisions", ctx).await;
        assert_eq!(decisions[0][0], "Where does export live?");
        let claims = first("oxplow-review/unbacked-claims", ctx).await;
        assert_eq!(claims.as_array().unwrap().len(), 1);
        assert_eq!(claims[0][0], "tests pass");
        let elsewhere = crate::extensions::LensContext {
            stream_id: Some(999),
            thread_id: None,
        };
        assert_eq!(
            first("oxplow-review/unbacked-claims", elsewhere).await,
            serde_json::json!([])
        );
    }

    /// Waiting on Me sits in the rail and raises an alert while anything
    /// is waiting on the user.
    #[tokio::test]
    async fn waiting_on_me_is_a_panel_whose_badge_alerts() {
        let f = crate::test_fixtures::services_with_effort().await;
        let review = crate::extensions::load_extensions(f._dir.path())
            .into_iter()
            .find(|e| e.name == "oxplow-review")
            .unwrap();
        assert!(review.panels.iter().any(|p| p.id == "oxplow-review/waiting"
            && p.badge.as_deref() == Some("oxplow-review/waiting-on-me")));
        let layer = crate::sql_gateway::SqlGateway::new(f.svc.db.clone());
        let alert = |run: crate::extensions::LensRun| run.alert.unwrap();
        let run = crate::extensions::run_lens(
            &layer,
            &f.svc.extension_catalog,
            f._dir.path(),
            "oxplow-review/waiting-on-me",
            Default::default(),
            &crate::extensions::LensContext::default(),
        )
        .await
        .unwrap();
        assert!(!alert(run).firing);
        f.svc
            .task_store
            .set_status(f.task, oxplow_domain::TaskStatus::Blocked)
            .await
            .unwrap();
        let run = crate::extensions::run_lens(
            &layer,
            &f.svc.extension_catalog,
            f._dir.path(),
            "oxplow-review/waiting-on-me",
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
    /// `crates/db/store.rs` outside it; oxplow-review's commands registered.
    async fn review_fixture() -> crate::test_fixtures::EffortFixture {
        use oxplow_db::EffortStore as _;
        use oxplow_domain::stores::TaskStore as _;
        let f = crate::test_fixtures::services_with_effort().await;
        let mut t = f.svc.task_store.get(f.task).await.unwrap().unwrap();
        t.description = "Fix the hover state in [[src/ui/button.ts]].".into();
        f.svc.task_store.update(&t).await.unwrap();
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
        f.svc
            .reasoning_store
            .record_claim(oxplow_db::NewClaim {
                thread_id: f.thread.value(),
                task_id: Some(f.task.value()),
                effort_id: Some(f.effort.value()),
                statement: "no behavior change".into(),
                kind: "no_behavior_change".into(),
                evidence_ref: None,
            })
            .await
            .unwrap();
        f.svc
            .reasoning_store
            .replace_inferred(
                f.effort.value(),
                vec![oxplow_db::NewDecision {
                    thread_id: f.thread.value(),
                    task_id: Some(f.task.value()),
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
        f.svc.extension_models.sync().await.unwrap();
        f.svc.extension_commands.reconcile().await;
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

    async fn task_notes(f: &crate::test_fixtures::EffortFixture) -> Vec<String> {
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

    async fn task_status(f: &crate::test_fixtures::EffortFixture) -> String {
        use oxplow_domain::stores::TaskStore as _;
        let t = f.svc.task_store.get(f.task).await.unwrap().unwrap();
        serde_json::to_value(t.status)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    }

    /// P7.C5: oxplow-review loads with no errors, its examples dry-run
    /// clean against the running registry, and its verbs register at boot
    /// as a person's (and a lens's), never an agent's, each confirmed.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_review_extension_registers_its_verbs_and_its_examples_check() {
        let f = review_fixture().await;
        let v = crate::extensions::validate_extension(
            &f.svc.sql,
            &f.svc.extension_catalog,
            f._dir.path(),
            "oxplow-review",
            Some(f.svc.commands.as_ref()),
        )
        .await
        .unwrap();
        assert!(v.errors.is_empty(), "{:?}", v.errors);
        assert!(v.warnings.is_empty(), "{:?}", v.warnings);
        for name in ["oxplow_review.accept", "oxplow_review.request_changes"] {
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
            thread_id: Some(f.thread),
            stream_id: None,
        };
        let err = review(
            &f,
            &agent,
            "oxplow_review.accept",
            serde_json::json!({ "ref": effort_ref(&f), "force": true }),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, oxplow_domain::CommandError::Denied { .. }),
            "{err:?}"
        );
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
            "oxplow_review.accept",
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

        review(
            &f,
            &human,
            "oxplow_review.accept",
            serde_json::json!({ "ref": effort_ref(&f), "force": true }),
        )
        .await
        .unwrap();
        assert_eq!(task_status(&f).await, "done");
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
            "effort.verify_claim",
            serde_json::json!({ "claim": format!("claim:{}", ids.0) }),
        )
        .await
        .unwrap();
        review(
            &f,
            &human,
            "effort.confirm_decision",
            serde_json::json!({ "decision": format!("decision:{}", ids.1) }),
        )
        .await
        .unwrap();
        review(
            &f,
            &human,
            "oxplow_review.accept",
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

    /// Request Changes comments a checklist — each unverified claim, each
    /// inferred decision, each file outside the task's area, and the
    /// note — and moves the task back to ready.
    #[tokio::test(flavor = "multi_thread")]
    async fn request_changes_lists_what_to_fix_and_reopens_the_task() {
        let f = review_fixture().await;
        review(
            &f,
            &oxplow_domain::Actor::Human,
            "oxplow_review.request_changes",
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

    /// The packet's rows carry their reviews: Mark Verified on an
    /// unverified claim; Confirm and Dismiss on an inferred decision.
    #[test]
    fn the_review_lenses_declare_their_row_actions() {
        let exts = crate::extensions::load_extensions(std::path::Path::new("/nonexistent"));
        let ext = exts.iter().find(|e| e.name == "oxplow-review").unwrap();
        let actions = |slug: &str| {
            ext.lenses
                .iter()
                .find(|l| l.id == format!("oxplow-review/{slug}"))
                .unwrap_or_else(|| panic!("{slug}"))
                .actions
                .iter()
                .filter(|a| a.row)
                .map(|a| (a.label.clone(), a.command.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            actions("unverified-claims"),
            vec![(
                "Mark Verified".to_string(),
                "effort.verify_claim".to_string()
            )]
        );
        assert_eq!(
            actions("inferred-decisions"),
            vec![
                ("Confirm".to_string(), "effort.confirm_decision".to_string()),
                ("Dismiss".to_string(), "effort.dismiss_decision".to_string()),
            ]
        );
        assert!(ext
            .lenses
            .iter()
            .any(|l| l.id == "oxplow-review/verify-claim-with-evidence"));
        assert!(ext
            .ui
            .commands
            .iter()
            .any(|c| c.command == "oxplow_review.accept" && c.label == "Accept Review"));
    }
}
