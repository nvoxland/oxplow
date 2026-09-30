//! The symbol index (`symbol`, `symbol_capture`; read as `v_symbol`,
//! `v_symbol_capture`): what the running language servers report for each
//! stream's files, restated per changed file at each snapshot by the
//! symbol collector (P5.C6, `.context/lsp.md`).

use oxplow_domain::{DomainError, Timestamp};

use crate::database::{map_sql_err, ts_to_string};
use crate::Database;

/// One symbol, 1-based positions. `name` is the symbol's own name,
/// `container` its enclosing path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolRow {
    pub name: String,
    pub kind: String,
    pub container: Option<String>,
    pub line: i64,
    pub col: i64,
    pub end_line: i64,
    pub end_col: i64,
}

/// A file's symbols as read at a snapshot, from one language's server;
/// `None` rows means the file is gone.
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
    pub files_over_budget: i64,
    pub files_without_server: i64,
    /// Asked about, but the server failed (an error, or no answer in
    /// time).
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
                        let qualified = match &r.container {
                            Some(c) => format!("{c}::{}", r.name),
                            None => r.name.clone(),
                        };
                        tx.execute(
                            "INSERT INTO symbol (ref, snapshot_id, stream_id, path, name, kind,
                               container, language, line, col, end_line, end_col)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                            rusqlite::params![
                                format!("symbol:{}/{qualified}@snap:{snapshot_id}", file.path),
                                snapshot_id,
                                stream_id,
                                file.path,
                                r.name,
                                r.kind,
                                r.container,
                                file.language,
                                r.line,
                                r.col,
                                r.end_line,
                                r.end_col,
                            ],
                        )
                        .map_err(map_sql_err)?;
                    }
                }
                tx.execute(
                    "INSERT OR REPLACE INTO symbol_capture (snapshot_id, stream_id,
                       files_collected, files_over_budget, files_without_server, files_failed,
                       captured_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        snapshot_id,
                        stream_id,
                        capture.files_collected,
                        capture.files_over_budget,
                        capture.files_without_server,
                        capture.files_failed,
                        at
                    ],
                )
                .map_err(map_sql_err)?;
                Ok(())
            })
            .await
    }
}
