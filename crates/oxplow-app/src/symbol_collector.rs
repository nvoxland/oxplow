//! The symbol collector (P5.C6, `.context/lsp.md`): the `symbols.collect`
//! pump consumer on `snapshot.taken` (registered after `search.index`).
//! For each file a snapshot changed, whose language a configured server
//! covers, it asks that server for the file's symbols
//! (`CodeIntelligence::document_symbols`) and restates the file in
//! `symbol` (`v_symbol`); a deleted file drops out. It is bounded by
//! `symbolsMaxFilesPerSnapshot` — the rest are recorded as over budget,
//! never an error — and **never starts a server**: a collector doesn't run
//! programs on a person's machine, so a file whose server isn't running is
//! recorded as such. Each snapshot's coverage is a `symbol_capture` row
//! (`v_symbol_capture`). Symbols are read from the file as it is on disk
//! when the event is handled, and pinned to the snapshot that triggered it.

use std::sync::{Arc, Weak};

use oxplow_db::{FileSymbols, SnapshotStorage, SymbolCapture, SymbolRow};
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
            .and_then(|r| r.strip_prefix("stream:"))
            .and_then(StreamId::try_from_str);
        let snapshot = payload["snapshot"]
            .as_str()
            .and_then(|r| r.strip_prefix("snapshot:"))
            .and_then(|n| n.parse::<i64>().ok());
        let (Some(stream), Some(snapshot)) = (stream, snapshot) else {
            return Ok(());
        };
        collect(&svc, stream, snapshot).await
    }
}

/// Collect `snapshot`'s changed files' symbols for `stream`.
pub async fn collect(svc: &Services, stream: StreamId, snapshot: i64) -> Result<(), DomainError> {
    let bound = svc
        .config
        .read()
        .map(|c| c.symbols_max_files_per_snapshot as usize)
        .unwrap_or(oxplow_config::DEFAULT_SYMBOLS_MAX_FILES_PER_SNAPSHOT as usize);
    let mut changed = svc.snapshot_store.list_files_for_snapshot(snapshot).await?;
    changed.sort_by(|a, b| a.path.cmp(&b.path));
    let stream_key = stream.to_string();
    let mut files = Vec::new();
    let mut capture = SymbolCapture::default();
    for file in changed {
        let Some(language) = svc.lsp_sessions.language_for_path(&file.path) else {
            continue;
        };
        if file.storage == SnapshotStorage::Deleted {
            files.push(FileSymbols {
                path: file.path,
                language,
                rows: None,
            });
            continue;
        }
        if !svc.lsp_sessions.is_running(&stream_key, &language).await {
            capture.files_without_server += 1;
            continue;
        }
        if capture.files_collected as usize >= bound {
            capture.files_over_budget += 1;
            continue;
        }
        match svc.code_intel.document_symbols(stream, &file.path).await {
            Ok(symbols) => {
                capture.files_collected += 1;
                files.push(FileSymbols {
                    path: file.path,
                    language,
                    rows: Some(symbols.into_iter().map(row).collect()),
                });
            }
            Err(e) => {
                // A server that can't answer for one file leaves it as it
                // was; the rest go on.
                tracing::warn!(path = file.path, error = %e, "document symbols failed");
            }
        }
    }
    if files.is_empty() && capture == SymbolCapture::default() {
        return Ok(());
    }
    svc.symbol_store
        .record(stream.value(), snapshot, files, capture)
        .await
}

fn row(s: oxplow_domain::code_intel::Symbol) -> SymbolRow {
    let r = s.location.range;
    SymbolRow {
        name: s.name,
        kind: s.kind,
        container: s.container,
        line: r.start.line.into(),
        col: r.start.col.into(),
        end_line: r.end.line.into(),
        end_col: r.end.col.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::events::schema::{SnapshotTaken, SnapshotTakenV1};

    fn sql(e: rusqlite::Error) -> DomainError {
        DomainError::Storage(e.to_string())
    }

    /// Services over a repo with three python files the fake server covers
    /// (and a README no server does), collecting at most `bound` files.
    async fn fixture(bound: u32) -> (Arc<Services>, tempfile::TempDir, StreamId) {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        for f in ["a.py", "b.py", "c.py", "README.md"] {
            std::fs::write(
                dir.path().join(f),
                "class Widget:\n    def spin(self): pass\n",
            )
            .unwrap();
        }
        let svc = Arc::new(Services::in_memory(dir.path()).unwrap());
        {
            let mut cfg = svc.config.write().unwrap();
            cfg.lsp_servers
                .push(crate::lsp_fake::config("python", &["py"]));
            cfg.symbols_max_files_per_snapshot = bound;
        }
        let stream = svc.streams.ensure_primary().await.unwrap();
        register(&svc);
        (svc, dir, stream.id)
    }

    /// A snapshot in which `paths` changed, announced as `snapshot.taken`.
    async fn snapshot(svc: &Services, stream: StreamId, paths: &[&str], deleted: &[&str]) -> i64 {
        let rows: Vec<(String, &str)> = paths
            .iter()
            .map(|p| (p.to_string(), "oxplow"))
            .chain(deleted.iter().map(|p| (p.to_string(), "deleted")))
            .collect();
        let id = svc
            .db
            .transaction(move |c| {
                c.execute(
                    "INSERT INTO snapshot (stream_id, created_at) VALUES (?1, '2026-09-30T00:00:00.000000Z')",
                    [stream.value()],
                )
                .map_err(sql)?;
                let id = c.last_insert_rowid();
                for (path, storage) in &rows {
                    c.execute(
                        "INSERT INTO file_snapshot (stream_id, path, blob_hash, size_bytes, captured_at, storage, snapshot_id)
                         VALUES (?1, ?2, 'h', 1, '2026-09-30T00:00:00.000000Z', ?3, ?4)",
                        rusqlite::params![stream.value(), path, storage, id],
                    )
                    .map_err(sql)?;
                }
                Ok(id)
            })
            .await
            .unwrap();
        svc.event_log_store
            .append(oxplow_domain::Envelope::typed::<SnapshotTaken>(
                "system:test",
                &SnapshotTakenV1 {
                    stream: format!("stream:{stream}"),
                    snapshot: format!("snapshot:{id}"),
                    parent: None,
                    trigger: oxplow_domain::snapshot::SnapshotTrigger::TurnEnd,
                    unchanged: false,
                    file_count: (paths.len() + deleted.len()) as u32,
                    elapsed_ms: 1,
                    budget_ms: None,
                    over_budget: false,
                },
            ))
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        id
    }

    async fn rows(svc: &Services, sql_text: &'static str) -> serde_json::Value {
        let r = crate::sql_gateway::SqlGateway::new(svc.db.clone())
            .query_sql(sql_text, vec![], None)
            .await
            .unwrap();
        serde_json::to_value(r.rows).unwrap()
    }

    /// P5.C6: a snapshot's changed files get their `v_symbol` rows, pinned
    /// `@snap:`; the bound truncates, and the capture records it.
    #[tokio::test]
    async fn a_snapshots_files_get_their_symbols_within_the_bound() {
        let (svc, dir, stream) = fixture(2).await;
        svc.lsp_sessions
            .ensure(&stream.to_string(), "python", dir.path().to_path_buf())
            .await
            .unwrap();
        let snap = snapshot(&svc, stream, &["a.py", "b.py", "c.py", "README.md"], &[]).await;
        assert_eq!(
            rows(
                &svc,
                "SELECT ref, kind, container FROM v_symbol ORDER BY ref"
            )
            .await,
            serde_json::json!([
                [
                    format!("symbol:a.py/Widget::spin@snap:{snap}"),
                    "method",
                    "Widget"
                ],
                [format!("symbol:a.py/Widget@snap:{snap}"), "class", null],
                [
                    format!("symbol:b.py/Widget::spin@snap:{snap}"),
                    "method",
                    "Widget"
                ],
                [format!("symbol:b.py/Widget@snap:{snap}"), "class", null],
            ])
        );
        assert_eq!(
            rows(
                &svc,
                "SELECT files_collected, files_over_budget, files_without_server FROM v_symbol_capture"
            )
            .await,
            serde_json::json!([[2, 1, 0]])
        );

        // A later snapshot restates what changed; a deleted file drops out.
        let later = snapshot(&svc, stream, &["b.py"], &["a.py"]).await;
        assert_eq!(
            rows(
                &svc,
                "SELECT DISTINCT path, snapshot_id FROM v_symbol ORDER BY path"
            )
            .await,
            serde_json::json!([["b.py", later]])
        );
    }

    /// The collector never starts a server: with none running, nothing is
    /// collected and the capture says why.
    #[tokio::test]
    async fn without_a_running_server_nothing_is_started() {
        let (svc, _dir, stream) = fixture(50).await;
        snapshot(&svc, stream, &["a.py", "b.py"], &[]).await;
        assert!(
            !svc.lsp_sessions
                .is_running(&stream.to_string(), "python")
                .await
        );
        assert_eq!(
            rows(&svc, "SELECT count(*) FROM v_symbol").await,
            serde_json::json!([[0]])
        );
        assert_eq!(
            rows(
                &svc,
                "SELECT files_collected, files_over_budget, files_without_server FROM v_symbol_capture"
            )
            .await,
            serde_json::json!([[0, 0, 2]])
        );
    }
}
