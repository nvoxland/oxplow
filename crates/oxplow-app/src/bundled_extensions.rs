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
            ext_file!("oxplow-review", "extension.yaml"),
            ext_file!("oxplow-review", "lenses/context-read.yaml"),
            ext_file!("oxplow-review", "lenses/decisions.yaml"),
            ext_file!("oxplow-review", "lenses/inferred-decisions.yaml"),
            ext_file!("oxplow-review", "lenses/struggled.yaml"),
            ext_file!("oxplow-review", "lenses/unverified-claims.yaml"),
            ext_file!("oxplow-review", "lenses/waiting-on-me.yaml"),
        ],
    },
];

pub fn find(name: &str) -> Option<&'static BundledExtension> {
    BUNDLED.iter().find(|b| b.name == name)
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

    /// The analytics effort-review lenses unnest the stored observation
    /// payloads (JUnit cases, uncovered lines) in SQL; run them on real rows.
    #[tokio::test]
    async fn analytics_effort_lenses_read_the_observation_payloads() {
        use oxplow_db::SqlCell;
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
                closest_git_version: None,
                git_version_exact: false,
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
        let layer = oxplow_db::SemanticLayer::new(f.svc.db.clone());
        let lens = |slug: &'static str| {
            let layer = &layer;
            let root = f._dir.path().to_path_buf();
            let mut params = std::collections::BTreeMap::new();
            params.insert("effort_id".to_string(), SqlCell::Int(f.effort.value()));
            async move {
                let run = crate::extensions::run_lens(
                    layer,
                    &root,
                    &format!("oxplow-analytics/{slug}"),
                    params,
                )
                .await
                .unwrap();
                serde_json::to_value(&run.result.rows).unwrap()
            }
        };
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
}
