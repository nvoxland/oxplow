//! Recording a duplicate-block scan. The change analyzer runs one for each
//! change it analyzes (scoped to the changed files, against the whole tree
//! at the change's head); this is where the scan leaves its record:
//!
//! - the scan and its findings in the code-quality store, read through
//!   `v_code_quality_scan` / `v_code_quality_finding`;
//! - for a full-tree scan only, `oxplow.duplicate_lines` facts under one
//!   `complete` capture, an empty one when nothing was found, so the
//!   metric's current state clears after a refactor removes every
//!   duplicate (tsk44). A scoped scan anchors only its paths, so its
//!   capture would restate the whole tree from a slice of it (tsk365);
//!   its findings still land in the code-quality store;
//! - a status-bar background task. Its rows' commits announce it
//!   (`ModelsChanged` on `v_code_quality_scan`).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use oxplow_db::CodeQualityScanStatus;
use oxplow_domain::vcs::Revision;
use oxplow_domain::DomainError;

use crate::code_quality_runner::{scan_duplicates, CodeQualityFinding, RunOptions};
use crate::trees::Trees;
use crate::{BackgroundTaskKind, BackgroundTaskStore, Services, StartInput};

/// What a scan needs to leave its record, owned so it can run on a
/// background task.
#[derive(Clone)]
pub struct DuplicationRecorder {
    code_quality_store: Arc<oxplow_db::SqliteCodeQualityStore>,
    fact_store: Arc<oxplow_db::SqliteFactStore>,
    stream_store: Arc<oxplow_db::SqliteStreamStore>,
    trees: Arc<Trees>,
    background_tasks: BackgroundTaskStore,
}

impl DuplicationRecorder {
    pub fn new(svc: &Services) -> Self {
        Self {
            code_quality_store: svc.code_quality_store.clone(),
            fact_store: svc.fact_store.clone(),
            stream_store: svc.stream_store.clone(),
            trees: svc.trees.clone(),
            background_tasks: svc.background_tasks.clone(),
        }
    }

    /// Scan workspace `ws` at `revision` for duplicated blocks anchored
    /// in `paths` (every file when `None`), with the whole tree as the
    /// corpus so a copy of an unchanged file is still found. Records the
    /// scan as the module docs describe and returns its findings.
    pub async fn record(
        &self,
        ws: PathBuf,
        revision: Revision,
        paths: Option<Vec<String>>,
        scope: String,
    ) -> Result<Vec<CodeQualityFinding>, DomainError> {
        let svc = self;
        let revision_str = revision.to_string();
        // Only a full-tree scan speaks for the whole tree (tsk365).
        let full_tree = paths.is_none();
        let fingerprint = match &paths {
            None => "all".to_string(),
            Some(paths) => paths_fingerprint(paths),
        };
        let scope_paths = paths.map(|p| p.into_iter().collect::<BTreeSet<String>>());

        let scan_id = svc
            .code_quality_store
            .create_scan("duplication", &scope, &revision_str, &fingerprint)
            .await?;
        let label = match &revision {
            Revision::Working => "Scanning duplicates (working tree)".to_string(),
            Revision::Vcs { rev, .. } => format!("Scanning duplicates @{}", short_ref(rev)),
            Revision::Snapshot(id) => format!("Scanning duplicates @snapshot {id}"),
        };
        let task = svc.background_tasks.start(StartInput {
            kind: BackgroundTaskKind::CodeQuality,
            label,
            detail: Some(format!("scope: {scope}")),
            progress: None,
        });
        let scanned_corpus = match svc.trees.corpus(&ws, &revision, |_| true).await {
            Ok(corpus) => scan_duplicates(corpus, scope_paths, RunOptions::default())
                .await
                .map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        let findings = match scanned_corpus {
            Ok(findings) => findings,
            Err(e) => {
                svc.code_quality_store
                    .finish_scan(scan_id, CodeQualityScanStatus::Failed, Some(e.clone()))
                    .await?;
                svc.background_tasks.fail(&task.id, e.clone(), None);
                return Err(DomainError::Invalid(e));
            }
        };

        // Storing can fail too; the scan and its task must not be left
        // "running" when it does (tsk364).
        let stored: Result<(), DomainError> = async {
            for f in &findings {
                svc.code_quality_store
                    .append_finding(
                        scan_id,
                        oxplow_db::CodeQualityFinding {
                            id: 0,
                            scan_id,
                            path: f.path.clone(),
                            start_line: f.start_line as i32,
                            end_line: f.end_line as i32,
                            kind: f.kind.clone(),
                            metric_value: f.metric_value,
                            extra_json: f.extra_json.clone(),
                        },
                    )
                    .await?;
            }
            svc.code_quality_store
                .finish_scan(scan_id, CodeQualityScanStatus::Done, None)
                .await
        }
        .await;
        if let Err(e) = stored {
            let _ = svc
                .code_quality_store
                .finish_scan(scan_id, CodeQualityScanStatus::Failed, Some(e.to_string()))
                .await;
            svc.background_tasks.fail(&task.id, e.to_string(), None);
            return Err(e);
        }
        if full_tree {
            if let Err(error) = write_facts(svc, &findings, &revision).await {
                tracing::warn!(%error, scan_id, "duplication: writing facts failed");
            }
        }
        svc.background_tasks.complete(&task.id, None);
        Ok(findings)
    }
}

/// Order-independent fingerprint of an explicit path list, stored as the
/// scan's `file_filter`.
fn paths_fingerprint(paths: &[String]) -> String {
    use std::hash::{Hash, Hasher};
    let mut sorted: Vec<&String> = paths.iter().collect();
    sorted.sort();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for p in &sorted {
        p.hash(&mut hasher);
    }
    format!("explicit:{:016x}", hasher.finish())
}

/// Abbreviate a ref for the status-bar label: long refs show their first 7
/// characters. By characters, not bytes, so a non-ASCII branch name can't
/// split mid-character (tsk178).
fn short_ref(r#ref: &str) -> String {
    if r#ref.chars().count() > 12 {
        r#ref.chars().take(7).collect()
    } else {
        r#ref.to_string()
    }
}

/// One `oxplow.duplicate_lines` fact per duplicate block of a full-tree
/// scan, under a single capture stamped with the primary stream (the scan has no stream of its
/// own) and the scanned revision. Written even when empty.
async fn write_facts(
    svc: &DuplicationRecorder,
    findings: &[CodeQualityFinding],
    revision: &Revision,
) -> Result<(), DomainError> {
    let Some(measure) = svc.fact_store.get_measure("oxplow.duplicate_lines").await? else {
        return Ok(());
    };
    use oxplow_domain::stores::StreamStore as _;
    let streams = svc.stream_store.list().await?;
    let Some(primary) = streams
        .iter()
        .find(|s| matches!(s.kind, oxplow_domain::StreamKind::Primary))
    else {
        return Ok(());
    };
    let facts = findings
        .iter()
        .filter(|f| f.kind == "duplicate-block")
        .map(|f| oxplow_db::NewFact {
            subject_kind: Some("block".into()),
            subject_ref: Some(format!("{}:{}-{}", f.path, f.start_line, f.end_line)),
            path: Some(f.path.clone()),
            line: Some(f.start_line as i64),
            detail: f.extra_json.clone(),
            ..oxplow_db::NewFact::new(measure.id, f.metric_value)
        })
        .collect();
    let capture = oxplow_db::NewMetricCapture {
        basis_ref: Some(revision.to_string()),
        closest_vcs_rev: match revision {
            Revision::Vcs { rev, .. } => Some(rev.clone()),
            _ => None,
        },
        trigger: Some("change-analysis".into()),
        ..oxplow_db::NewMetricCapture::done(primary.id.value(), "duplication", "duplication")
    };
    svc.fact_store.record_facts(capture, facts).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The current duplicate-lines facts: what a scan leaves standing.
    async fn current(svc: &Services) -> Vec<(Option<String>, f64)> {
        let measure = svc
            .fact_store
            .get_measure("oxplow.duplicate_lines")
            .await
            .unwrap()
            .unwrap();
        let mut facts: Vec<(Option<String>, f64)> = svc
            .metric_engine
            .current_facts(&measure)
            .await
            .unwrap()
            .into_iter()
            .map(|f| (f.path, f.value))
            .collect();
        facts.sort_by(|a, b| a.0.cmp(&b.0));
        facts
    }

    /// Two files sharing a >= 10-line identical block (the production
    /// minimum) → a duplicate-block finding.
    const BODY: &str = "pub fn compute(input: &[i64]) -> i64 {\n\
        \x20   let mut total = 0;\n\
        \x20   for value in input {\n\
        \x20       if *value > 0 {\n\
        \x20           total += *value;\n\
        \x20       } else {\n\
        \x20           total -= *value;\n\
        \x20       }\n\
        \x20   }\n\
        \x20   total * 2 + 1\n\
        }\n";

    async fn scan(f: &crate::test_fixtures::EffortFixture) -> Vec<CodeQualityFinding> {
        DuplicationRecorder::new(&f.svc)
            .record(
                f.svc.layout.project_dir.clone(),
                Revision::Working,
                None,
                "project".into(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_scan_records_its_findings_and_duplicate_line_facts() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        std::fs::write(root.join("a.rs"), BODY).unwrap();
        std::fs::write(root.join("b.rs"), BODY).unwrap();
        let findings = scan(&f).await;
        assert!(!findings.is_empty());

        let scans = crate::sql_gateway::SqlGateway::new(f.svc.db.clone())
            .query_sql(
                "SELECT status, revision FROM v_code_quality_scan",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(scans.rows).unwrap(),
            serde_json::json!([["done", "working"]])
        );

        let measure = f
            .svc
            .fact_store
            .get_measure("oxplow.duplicate_lines")
            .await
            .unwrap()
            .expect("built-in measure seeded");
        let facts = f
            .svc
            .fact_store
            .facts_for_measure(measure.id)
            .await
            .unwrap();
        assert!(!facts.is_empty(), "duplication facts written");
        assert!(facts.iter().all(|f| f.value >= 10.0));
        assert!(facts
            .iter()
            .all(|f| f.subject_kind.as_deref() == Some("block")));
        assert!(
            facts.iter().all(|f| f.detail.is_some()),
            "peer side in detail"
        );
        let cap = f
            .svc
            .fact_store
            .get_capture(facts[0].capture_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cap.producer, "duplication");
        assert!(cap.basis_ref.is_some());
    }

    #[tokio::test]
    async fn a_rescan_with_no_duplicates_clears_the_metric() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        std::fs::write(root.join("a.rs"), BODY).unwrap();
        std::fs::write(root.join("b.rs"), BODY).unwrap();
        scan(&f).await;
        assert!(!current(&f.svc).await.is_empty());
        std::fs::remove_file(root.join("b.rs")).unwrap();
        scan(&f).await;
        assert!(
            current(&f.svc).await.is_empty(),
            "a zero-hit rescan clears the current state"
        );
    }

    /// A change-scoped scan sees only its changed files, so it can't speak
    /// for the whole tree: it leaves the full-tree metric alone (tsk365).
    #[tokio::test]
    async fn a_scoped_scan_leaves_the_metric_alone() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        std::fs::write(root.join("a.rs"), BODY).unwrap();
        std::fs::write(root.join("b.rs"), BODY).unwrap();
        scan(&f).await;
        let before = current(&f.svc).await;
        assert!(!before.is_empty());
        std::fs::write(root.join("c.rs"), "fn unrelated() {}\n").unwrap();
        let findings = DuplicationRecorder::new(&f.svc)
            .record(
                root.clone(),
                Revision::Working,
                Some(vec!["c.rs".into()]),
                "change 1".into(),
            )
            .await
            .unwrap();
        assert!(findings.is_empty());
        assert_eq!(
            serde_json::to_value(current(&f.svc).await).unwrap(),
            serde_json::to_value(before).unwrap(),
            "the scoped scan didn't restate the tree"
        );
    }

    /// P5.B2 (tsk521): a scan reads the revision it names — a commit or
    /// a snapshot — never the disk, which has since lost the copies.
    #[tokio::test]
    async fn a_scan_reads_the_revision_it_names() {
        use oxplow_domain::stores::StreamStore as _;
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        std::fs::write(root.join("a.rs"), BODY).unwrap();
        std::fs::write(root.join("b.rs"), BODY).unwrap();
        let sha = crate::test_fixtures::commit_all(&root, "copies");
        let stream = svc.stream_store.list().await.unwrap()[0].id;
        let snap = svc.snapshot_store.create_snapshot(stream).await.unwrap();
        for path in ["a.rs", "b.rs"] {
            svc.snapshot_store
                .capture(oxplow_db::FileSnapshot {
                    id: 0,
                    stream_id: stream,
                    path: path.into(),
                    blob_hash: Some(svc.blobs.write(BODY.as_bytes()).unwrap()),
                    size_bytes: BODY.len() as i64,
                    captured_at: oxplow_domain::Timestamp::now(),
                    storage: oxplow_db::SnapshotStorage::Oxplow,
                    snapshot_id: Some(snap),
                    mtime_ms: None,
                    content_hash: None,
                })
                .await
                .unwrap();
        }
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(root.join("b.rs"), "fn b() {}\n").unwrap();
        let recorder = DuplicationRecorder::new(svc);
        for rev in [Revision::git(sha), Revision::Snapshot(snap)] {
            let findings = recorder
                .record(root.clone(), rev.clone(), None, "project".into())
                .await
                .unwrap();
            assert!(!findings.is_empty(), "no duplicates found at {rev}");
        }
        let on_disk = recorder
            .record(root.clone(), Revision::Working, None, "project".into())
            .await
            .unwrap();
        assert!(on_disk.is_empty(), "{on_disk:?}");
    }

    #[test]
    fn short_refs_cut_by_characters() {
        let multibyte = "日本語ブランチ名テスト";
        assert!(!multibyte.is_char_boundary(7));
        assert_eq!(short_ref(multibyte), multibyte);
        assert_eq!(short_ref("abcdef0123456789"), "abcdef0");
        assert_eq!(
            short_ref("日本語ブランチ名テストです工事中"),
            "日本語ブランチ"
        );
        assert_eq!(short_ref("main"), "main");
    }

    #[test]
    fn the_path_fingerprint_ignores_order() {
        let a = paths_fingerprint(&["x".into(), "y".into()]);
        assert_eq!(a, paths_fingerprint(&["y".into(), "x".into()]));
        assert_ne!(a, paths_fingerprint(&["x".into()]));
    }
}
