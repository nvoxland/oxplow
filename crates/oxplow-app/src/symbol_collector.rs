//! The symbol collector (`.context/language-support.md` "Symbols"): the
//! `symbols.collect` pump consumer on `snapshot.taken` (registered after
//! `search.index`). For each file a snapshot changed in a supported
//! language, it reads the file as the snapshot captured it, extracts its
//! definitions with oxplow's own tree-sitter queries
//! (`oxplow_code_metrics::extract`) and restates the file in `symbol`
//! (`v_symbol`); a deleted file drops out. No language server is involved.
//! A snapshot that kept no contents ("Track changes only") is read from the
//! working tree instead. Each snapshot's coverage is a `symbol_capture` row
//! (`v_symbol_capture`). At boot, an empty index is filled from each
//! stream's latest snapshot ([`backfill`]).

use std::path::PathBuf;
use std::sync::{Arc, Weak};

use oxplow_db::{FileSymbols, SnapshotStorage, SymbolCapture, SymbolRow};
use oxplow_domain::stores::StreamStore;
use oxplow_domain::vcs::Revision;
use oxplow_domain::{DomainError, StoredEvent, StreamId};

use crate::event_pump::AsyncEventConsumer;
use crate::Services;

pub const SYMBOLS_COLLECT: &str = "symbols.collect";

/// Register the collector on `svc`'s pump (boot, after `search.index`).
pub fn register(svc: &Arc<Services>) {
    svc.event_pump.register_async(Arc::new(SymbolCollector {
        services: Arc::downgrade(svc),
    }));
}

struct SymbolCollector {
    services: Weak<Services>,
}

#[async_trait::async_trait]
impl AsyncEventConsumer for SymbolCollector {
    fn name(&self) -> &'static str {
        SYMBOLS_COLLECT
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == "snapshot.taken"
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        let Some(svc) = self.services.upgrade() else {
            return Err(DomainError::Busy("services are shutting down".into()));
        };
        let payload = &event.envelope.payload;
        if payload["unchanged"].as_bool() == Some(true) {
            return Ok(());
        }
        let stream = payload["stream"]
            .as_str()
            .and_then(|s| s.parse::<StreamId>().ok());
        let snapshot = payload["snapshot"].as_i64();
        let (Some(stream), Some(snapshot)) = (stream, snapshot) else {
            return Ok(());
        };
        collect(&svc, stream, snapshot).await
    }
}

/// Collect `snapshot`'s changed files' symbols for `stream`.
pub async fn collect(svc: &Services, stream: StreamId, snapshot: i64) -> Result<(), DomainError> {
    let mut changed = svc.snapshot_store.list_files_for_snapshot(snapshot).await?;
    changed.sort_by(|a, b| a.path.cmp(&b.path));
    let keeps_contents = svc.snapshot_captures.no_contents_reason().is_none();
    let files_api = svc.snapshot_files();
    let mut files = Vec::new();
    let mut capture = SymbolCapture::default();
    for file in changed {
        let Some(language) = oxplow_code_metrics::language_for_path(&file.path) else {
            continue;
        };
        if file.storage == SnapshotStorage::Deleted {
            files.push(FileSymbols {
                path: file.path,
                language: language.name().into(),
                rows: None,
            });
            continue;
        }
        let bytes = if keeps_contents {
            files_api
                .read_file_at_snapshot(snapshot, &file.path)
                .await
                .ok()
                .flatten()
        } else {
            svc.workspace_files
                .read(Some(&stream.to_string()), file.path.clone())
                .await
                .ok()
                .map(|f| f.content.into_bytes())
        };
        match bytes.and_then(|b| String::from_utf8(b).ok()) {
            Some(text) => {
                capture.files_collected += 1;
                files.push(file_symbols(&file.path, &text, language));
            }
            None => capture.files_failed += 1,
        }
    }
    if files.is_empty() && capture == SymbolCapture::default() {
        return Ok(());
    }
    svc.symbol_store
        .record(stream.value(), snapshot, files, capture)
        .await
}

/// Fill an empty index (a new database, or one a migration cleared) from
/// each live stream's latest snapshot: every file of a supported language
/// in that snapshot's tree, recorded as that snapshot's collection.
pub async fn backfill(svc: &Services) -> Result<usize, DomainError> {
    if !svc.symbol_store.is_empty().await? {
        return Ok(0);
    }
    let keeps_contents = svc.snapshot_captures.no_contents_reason().is_none();
    let project_dir = svc.snapshot_files().project_dir;
    let mut filled = 0;
    for stream in svc.stream_store.list().await? {
        if stream.archived_at.is_some() {
            continue;
        }
        let Some(snapshot) = svc
            .snapshot_store
            .latest_snapshot_id_for_stream(stream.id)
            .await?
        else {
            continue;
        };
        let ws = match stream.worktree_path.as_str() {
            "" => project_dir.clone(),
            p => PathBuf::from(p),
        };
        let rev = if keeps_contents {
            Revision::Snapshot(snapshot)
        } else {
            Revision::Working
        };
        let supported = |p: &str| oxplow_code_metrics::language_for_path(p).is_some();
        let corpus = svc.trees.corpus(&ws, &rev, supported).await?;
        let files: Vec<FileSymbols> = corpus
            .iter()
            .filter_map(|(path, text)| {
                let language = oxplow_code_metrics::language_for_path(path)?;
                Some(file_symbols(path, text, language))
            })
            .collect();
        let capture = SymbolCapture {
            files_collected: files.len() as i64,
            files_failed: 0,
        };
        filled += files.len();
        svc.symbol_store
            .record(stream.id.value(), snapshot, files, capture)
            .await?;
    }
    Ok(filled)
}

fn file_symbols(path: &str, text: &str, language: oxplow_code_metrics::Language) -> FileSymbols {
    let rows = oxplow_code_metrics::extract_with_language(text, language)
        .into_iter()
        .map(|d| SymbolRow {
            container: (!d.container.is_empty()).then(|| d.container.join("::")),
            kind: d.kind.as_str().into(),
            id: d.id,
            name: d.name,
            signature: d.signature,
            line: d.name_line.into(),
            col: d.name_col.into(),
            start_line: d.start_line.into(),
            start_col: d.start_col.into(),
            end_line: d.end_line.into(),
            end_col: d.end_col.into(),
        })
        .collect();
    FileSymbols {
        path: path.into(),
        language: language.name().into(),
        rows: Some(rows),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::events::schema::{SnapshotTaken, SnapshotTakenV3};

    /// Services over a repo, with the collector registered.
    async fn fixture() -> (Arc<Services>, tempfile::TempDir, StreamId) {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let svc = Services::in_memory(dir.path()).await.unwrap();
        let stream = svc.streams.ensure_primary().await.unwrap();
        register(&svc);
        (svc, dir, stream.id)
    }

    /// A snapshot capturing `files` (`None` content: deleted), announced as
    /// `snapshot.taken` and handled.
    async fn snapshot(svc: &Services, stream: StreamId, files: &[(&str, Option<&str>)]) -> i64 {
        use oxplow_domain::Timestamp;
        let snap = svc.snapshot_store.create_snapshot(stream).await.unwrap();
        for (path, content) in files {
            let (blob_hash, size, storage) = match content {
                Some(text) => (
                    Some(svc.blobs.write(text.as_bytes()).unwrap()),
                    text.len() as i64,
                    SnapshotStorage::Oxplow,
                ),
                None => (None, 0, SnapshotStorage::Deleted),
            };
            svc.snapshot_store
                .capture(oxplow_db::FileSnapshot {
                    id: 0,
                    stream_id: stream,
                    path: path.to_string(),
                    blob_hash,
                    size_bytes: size,
                    captured_at: Timestamp::from_unix_ms(0),
                    storage,
                    snapshot_id: Some(snap),
                    mtime_ms: Some(0),
                    content_hash: None,
                })
                .await
                .unwrap();
        }
        svc.event_log_store
            .append(oxplow_domain::Envelope::typed::<SnapshotTaken>(
                "system:test",
                &SnapshotTakenV3 {
                    stream: stream.to_string(),
                    snapshot: snap,
                    parent: None,
                    trigger: oxplow_domain::snapshot::SnapshotTrigger::TurnEnd,
                    unchanged: false,
                    file_count: files.len() as u32,
                    elapsed_ms: 1,
                    budget_ms: None,
                    over_budget: false,
                },
            ))
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        snap
    }

    async fn rows(svc: &Services, sql_text: &'static str) -> serde_json::Value {
        let r = crate::sql_gateway::SqlGateway::new(svc.db.clone())
            .query_sql(sql_text, vec![], None)
            .await
            .unwrap();
        serde_json::to_value(r.rows).unwrap()
    }

    const WIDGET: &str = "class Widget:\n    def spin(self): pass\n";

    /// A snapshot's changed files get their `v_symbol` rows from
    /// tree-sitter — no language server running — under rev-less refs; a
    /// later snapshot restates what changed and a deleted file drops out.
    #[tokio::test]
    async fn a_snapshots_files_get_their_symbols_without_a_language_server() {
        let (svc, _dir, stream) = fixture().await;
        let snap = snapshot(
            &svc,
            stream,
            &[
                ("a.py", Some(WIDGET)),
                ("Lib/b.py", Some(WIDGET)),
                ("README.md", Some("# hi")),
            ],
        )
        .await;
        assert_eq!(
            rows(
                &svc,
                "SELECT uri, kind, container, snapshot_id FROM v_symbol ORDER BY uri"
            )
            .await,
            serde_json::json!([
                [
                    "oxplow:oxplow/symbol?path=Lib/b.py&symbol=Widget",
                    "class",
                    null,
                    snap
                ],
                [
                    "oxplow:oxplow/symbol?path=Lib/b.py&symbol=Widget::spin",
                    "method",
                    "Widget",
                    snap
                ],
                [
                    "oxplow:oxplow/symbol?path=a.py&symbol=Widget",
                    "class",
                    null,
                    snap
                ],
                [
                    "oxplow:oxplow/symbol?path=a.py&symbol=Widget::spin",
                    "method",
                    "Widget",
                    snap
                ],
            ])
        );
        assert_eq!(
            rows(
                &svc,
                "SELECT files_collected, files_failed FROM v_symbol_capture"
            )
            .await,
            serde_json::json!([[2, 0]])
        );

        let later = snapshot(
            &svc,
            stream,
            &[("Lib/b.py", Some("def go(): pass\n")), ("a.py", None)],
        )
        .await;
        assert_eq!(
            rows(&svc, "SELECT uri, snapshot_id FROM v_symbol ORDER BY uri").await,
            serde_json::json!([["oxplow:oxplow/symbol?path=Lib/b.py&symbol=go", later]])
        );
    }

    /// An empty index is filled from each stream's latest snapshot; a
    /// filled one is left alone.
    #[tokio::test]
    async fn backfill_fills_an_empty_index_from_the_latest_snapshot() {
        let (svc, _dir, stream) = fixture().await;
        let snap = snapshot(&svc, stream, &[("a.py", Some(WIDGET))]).await;
        svc.db
            .transaction(|tx| {
                tx.execute("DELETE FROM symbol", [])
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(backfill(&svc).await.unwrap(), 1);
        assert_eq!(
            rows(&svc, "SELECT uri, snapshot_id FROM v_symbol ORDER BY uri").await,
            serde_json::json!([
                ["oxplow:oxplow/symbol?path=a.py&symbol=Widget", snap],
                ["oxplow:oxplow/symbol?path=a.py&symbol=Widget::spin", snap]
            ])
        );
        assert_eq!(backfill(&svc).await.unwrap(), 0);
    }
}
