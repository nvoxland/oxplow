//! The symbol index (`symbol`, `symbol_capture`; read as `v_symbol`,
//! `v_symbol_capture`): what each stream's files define, as oxplow's
//! tree-sitter extraction reads them, restated per changed file at each
//! snapshot by the symbol collector (`.context/language-support.md`
//! "Symbols").

use oxplow_domain::{DomainError, Timestamp};

use crate::database::{map_sql_err, ts_to_string};
use crate::Database;

/// One definition, 1-based positions. `id` is its symbol id within the
/// file (`Parser::parse`, `main~2`) — its ref is `symbol:<path>/<id>`;
/// `name` its own name, `container` what it's inside (`::`-joined);
/// `line`/`col` is where its name is, `start_*`..`end_*` its whole extent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolRow {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub container: Option<String>,
    pub signature: Option<String>,
    pub line: i64,
    pub col: i64,
    pub start_line: i64,
    pub start_col: i64,
    pub end_line: i64,
    pub end_col: i64,
}

/// A file's definitions as read at a snapshot; `None` rows means the file
/// is gone (or no longer in a supported language).
#[derive(Debug, Clone)]
pub struct FileSymbols {
    pub path: String,
    pub language: String,
    pub rows: Option<Vec<SymbolRow>>,
}

/// What one snapshot's collection covered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SymbolCapture {
    pub files_collected: i64,
    /// Content that couldn't be read at the snapshot.
    pub files_failed: i64,
}

#[derive(Clone)]
pub struct SqliteSymbolStore {
    db: Database,
}

impl SqliteSymbolStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Restate `files` for `stream_id` as read at `snapshot_id`, and record
    /// the capture — one transaction.
    pub async fn record(
        &self,
        stream_id: i64,
        snapshot_id: i64,
        files: Vec<FileSymbols>,
        capture: SymbolCapture,
    ) -> Result<(), DomainError> {
        let at = ts_to_string(Timestamp::now());
        self.db
            .transaction(move |tx| {
                for file in &files {
                    tx.execute(
                        "DELETE FROM symbol WHERE stream_id = ?1 AND path = ?2",
                        rusqlite::params![stream_id, file.path],
                    )
                    .map_err(map_sql_err)?;
                    for r in file.rows.iter().flatten() {
                        tx.execute(
                            "INSERT INTO symbol (ref, snapshot_id, stream_id, path, name, kind,
                               container, signature, language, line, col, start_line,
                               start_col, end_line, end_col)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                                     ?14, ?15)",
                            rusqlite::params![
                                oxplow_domain::refs::build::symbol_ref(&file.path, &r.id),
                                snapshot_id,
                                stream_id,
                                file.path,
                                r.name,
                                r.kind,
                                r.container,
                                r.signature,
                                file.language,
                                r.line,
                                r.col,
                                r.start_line,
                                r.start_col,
                                r.end_line,
                                r.end_col,
                            ],
                        )
                        .map_err(map_sql_err)?;
                    }
                }
                tx.execute(
                    "INSERT OR REPLACE INTO symbol_capture (snapshot_id, stream_id,
                       files_collected, files_failed, captured_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![
                        snapshot_id,
                        stream_id,
                        capture.files_collected,
                        capture.files_failed,
                        at
                    ],
                )
                .map_err(map_sql_err)?;
                Ok(())
            })
            .await
    }

    /// Whether the index holds nothing (the boot backfill fills it).
    pub async fn is_empty(&self) -> Result<bool, DomainError> {
        self.db
            .call(|c| c.query_row("SELECT NOT EXISTS (SELECT 1 FROM symbol)", [], |r| r.get(0)))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, line: i64) -> SymbolRow {
        SymbolRow {
            id: id.into(),
            name: id.rsplit("::").next().unwrap().into(),
            kind: "method".into(),
            container: Some("Widget".into()),
            signature: None,
            line,
            col: 5,
            start_line: line - 1,
            start_col: 1,
            end_line: line + 2,
            end_col: 1,
        }
    }

    async fn stream(db: &Database, id: i64) {
        db.call(move |c| {
            c.execute(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source,
                                      worktree_path, created_at, updated_at)
                 VALUES (?1, CASE ?1 WHEN 1 THEN 'primary' ELSE 'worktree' END, 's', 'main', 'r', 'r', '/r' || ?1, 'now', 'now')",
                [id],
            )
            .map(|_| ())
        })
        .await
        .unwrap();
    }

    /// A file's rows are restated whole, each under its rev-less ref; the
    /// same symbol in two streams is a row in each.
    #[tokio::test]
    async fn a_files_symbols_are_restated_under_their_refs_per_stream() {
        let db = Database::in_memory();
        stream(&db, 1).await;
        stream(&db, 2).await;
        let store = SqliteSymbolStore::new(db.clone());
        assert!(store.is_empty().await.unwrap());
        let file = |rows| FileSymbols {
            path: "README.md/w.py".into(),
            language: "python".into(),
            rows,
        };
        for (stream, snap) in [(1, 7), (2, 8)] {
            store
                .record(
                    stream,
                    snap,
                    vec![file(Some(vec![
                        row("Widget::value", 10),
                        row("Widget::value~2", 20),
                    ]))],
                    SymbolCapture::default(),
                )
                .await
                .unwrap();
        }
        store
            .record(
                1,
                9,
                vec![file(Some(vec![row("Widget::spin", 30)]))],
                SymbolCapture {
                    files_collected: 1,
                    files_failed: 0,
                },
            )
            .await
            .unwrap();
        let refs: Vec<(i64, String, i64)> = db
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT stream_id, ref, snapshot_id FROM symbol ORDER BY stream_id, line",
                )?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>();
                rows
            })
            .await
            .unwrap();
        assert_eq!(
            refs,
            vec![
                (1, "symbol:README.md/w.py/Widget::spin".into(), 9),
                (2, "symbol:README.md/w.py/Widget::value".into(), 8),
                (2, "symbol:README.md/w.py/Widget::value~2".into(), 8),
            ]
        );
        assert!(!store.is_empty().await.unwrap());
    }
}
