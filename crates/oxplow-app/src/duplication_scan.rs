//! Recording a duplicate-block scan. The change analyzer runs one for each
//! change it analyzes (scoped to the changed files, against the whole tree
//! at the change's head); this is where the scan leaves its record:
//!
//! - the scan and its findings in the code-quality store, read through
//!   `v_code_quality_scan` / `v_code_quality_finding`;
//! - `oxplow.duplicate_lines` facts under one capture, an empty one when
//!   nothing was found, so the metric's current state clears after a
//!   refactor removes every duplicate (tsk44);
//! - a status-bar background task and `CodeQualityScanned` events.

use std::path::PathBuf;
use std::sync::Arc;

use oxplow_db::CodeQualityScanStatus;
use oxplow_domain::DomainError;
use oxplow_tree_source::{
    AllFiles, DiskTreeSource, ExplicitPaths, FileFilter, GitTreeSource, TreeSource, TreeVersion,
};

use crate::code_quality_runner::{run_duplication_scan_scoped, CodeQualityFinding};
use crate::{
    BackgroundTaskKind, BackgroundTaskStore, CodeQualityScanPhase, EventBus, OxplowEvent, Services,
    StartInput,
};

/// What a scan needs to leave its record, owned so it can run on a
/// background task.
#[derive(Clone)]
pub struct DuplicationRecorder {
    code_quality_store: Arc<oxplow_db::SqliteCodeQualityStore>,
    fact_store: Arc<oxplow_db::SqliteFactStore>,
    stream_store: Arc<oxplow_db::SqliteStreamStore>,
    config: Arc<std::sync::RwLock<oxplow_config::OxplowConfig>>,
    background_tasks: BackgroundTaskStore,
    events: EventBus,
}

impl DuplicationRecorder {
    pub fn new(svc: &Services) -> Self {
        Self {
            code_quality_store: svc.code_quality_store.clone(),
            fact_store: svc.fact_store.clone(),
            stream_store: svc.stream_store.clone(),
            config: svc.config.clone(),
            background_tasks: svc.background_tasks.clone(),
            events: svc.events.clone(),
        }
    }

    /// Scan `root` at `tree_version` for duplicated blocks anchored in
    /// `paths` (every file when `None`), with the whole tree as the corpus
    /// so a copy of an unchanged file is still found. Records the scan as
    /// the module docs describe and returns its findings.
    pub async fn record(
        &self,
        root: PathBuf,
        tree_version: TreeVersion,
        paths: Option<Vec<String>>,
        scope: String,
    ) -> Result<Vec<CodeQualityFinding>, DomainError> {
        let svc = self;
        let source: Arc<dyn TreeSource> = match &tree_version {
            TreeVersion::Disk => Arc::new(DiskTreeSource::new(root.clone())),
            TreeVersion::Ref { r#ref } => Arc::new(GitTreeSource::new(root.clone(), r#ref.clone())),
            TreeVersion::Snapshot { .. } => {
                return Err(DomainError::Invalid(
                    "snapshot tree versions can't be scanned for duplicates yet".into(),
                ))
            }
        };
        let kind_tag = tree_version.kind_tag().to_string();
        let value = tree_version.value().map(str::to_string);
        let (filter, fingerprint): (Arc<dyn FileFilter>, String) = match paths {
            None => (Arc::new(AllFiles), "all".into()),
            Some(paths) => {
                let fp = paths_fingerprint(&paths);
                (Arc::new(ExplicitPaths::new(paths)), fp)
            }
        };

        let scan_id = svc
            .code_quality_store
            .create_scan_with(
                "duplication",
                &scope,
                &kind_tag,
                value.as_deref(),
                &fingerprint,
            )
            .await?;
        let scanned = |phase| OxplowEvent::CodeQualityScanned {
            stream_id: None,
            scan_id,
            tool: "duplication".into(),
            scope: scope.clone(),
            phase,
        };
        svc.events.emit(scanned(CodeQualityScanPhase::Started));
        let label = match &tree_version {
            TreeVersion::Disk => "Scanning duplicates (working tree)".to_string(),
            TreeVersion::Ref { r#ref } => format!("Scanning duplicates @{}", short_ref(r#ref)),
            TreeVersion::Snapshot { id } => format!("Scanning duplicates @snapshot {id}"),
        };
        let task = svc.background_tasks.start(StartInput {
            kind: BackgroundTaskKind::CodeQuality,
            label,
            detail: Some(format!("scope: {scope}")),
            progress: None,
        });
        let workspace_filter = {
            let cfg = svc.config.read().unwrap_or_else(|e| e.into_inner());
            oxplow_fs_watch::WorkspaceFilter::for_project(
                &root,
                &cfg.generated.exclude,
                &cfg.generated.include,
            )
        };

        let findings =
            match run_duplication_scan_scoped(source, filter, workspace_filter, None, None).await {
                Ok(findings) => findings,
                Err(e) => {
                    svc.code_quality_store
                        .finish_scan(scan_id, CodeQualityScanStatus::Failed, Some(e.to_string()))
                        .await?;
                    svc.events.emit(scanned(CodeQualityScanPhase::Failed));
                    svc.background_tasks.fail(&task.id, e.to_string(), None);
                    return Err(DomainError::Invalid(e.to_string()));
                }
            };

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
            .await?;
        if let Err(error) = write_facts(svc, &findings, &kind_tag, value.as_deref()).await {
            tracing::warn!(%error, scan_id, "duplication: writing facts failed");
        }
        svc.events.emit(scanned(CodeQualityScanPhase::Completed));
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

/// One `oxplow.duplicate_lines` fact per duplicate block, under a single
/// capture stamped with the primary stream (the scan has no stream of its
/// own) and the scanned tree version. Written even when empty.
async fn write_facts(
    svc: &DuplicationRecorder,
    findings: &[CodeQualityFinding],
    kind_tag: &str,
    value: Option<&str>,
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
    let basis = match value {
        Some(v) => format!("{kind_tag}:{v}"),
        None => kind_tag.to_string(),
    };
    let capture = oxplow_db::NewMetricCapture {
        basis_ref: Some(basis),
        closest_git_version: value.map(str::to_string),
        trigger: Some("change-analysis".into()),
        ..oxplow_db::NewMetricCapture::done(primary.id.value(), "duplication", "duplication")
    };
    svc.fact_store.record_facts(capture, facts).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
                TreeVersion::Disk,
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

        let scans = oxplow_db::SemanticLayer::new(f.svc.db.clone())
            .query_sql(
                "SELECT status, tree_version_kind FROM v_code_quality_scan",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(scans.rows).unwrap(),
            serde_json::json!([["done", "disk"]])
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
        let rollup = |svc: Arc<Services>| async move {
            svc.metric_engine
                .rollup("oxplow.duplicate_lines", "oxplow.package")
                .await
                .unwrap()
        };
        assert!(!rollup(f.svc.clone()).await.is_empty());
        std::fs::remove_file(root.join("b.rs")).unwrap();
        scan(&f).await;
        assert!(
            rollup(f.svc.clone()).await.is_empty(),
            "a zero-hit rescan clears the current state"
        );
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
