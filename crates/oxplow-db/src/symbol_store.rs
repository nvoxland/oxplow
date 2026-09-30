//! The symbol index (`symbol`, `symbol_capture`; read as `v_symbol`,
//! `v_symbol_capture`): what the running language servers report for each
//! stream's files, restated per changed file at each snapshot by the
//! symbol collector (P5.C6, `.context/lsp.md`).

use oxplow_domain::{DomainError, Timestamp};

use crate::database::{map_sql_err, ts_to_string};
use crate::Database;

/// One symbol, 1-based positions. `name` is the symbol's own name,
/// `container` its enclosing path; `line`/`col` is where its name is,
/// `start_*`..`end_*` its whole extent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolRow {
    pub name: String,
    pub kind: String,
    pub container: Option<String>,
    pub line: i64,
    pub col: i64,
    pub start_line: i64,
    pub start_col: i64,
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
                    // In position order, so a name path's later symbols (an
                    // overload, a setter after its getter) are numbered
                    // stably: `Widget::value`, `Widget::value~2`.
                    let mut rows: Vec<&SymbolRow> = file.rows.iter().flatten().collect();
                    rows.sort_by_key(|r| (r.line, r.col));
                    let mut seen: std::collections::HashMap<String, u32> =
                        std::collections::HashMap::new();
                    for r in rows {
                        let qualified = match &r.container {
                            Some(c) => format!("{c}::{}", r.name),
                            None => r.name.clone(),
                        };
                        let nth = seen.entry(qualified.clone()).or_insert(0);
                        *nth += 1;
                        let id = match *nth {
                            1 => qualified,
                            n => format!("{qualified}~{n}"),
                        };
                        tx.execute(
                            "INSERT INTO symbol (ref, snapshot_id, stream_id, path, name, kind,
                               container, language, line, col, start_line, start_col,
                               end_line, end_col)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                            rusqlite::params![
                                format!("symbol:{}/{id}@snap:{snapshot_id}", file.path),
                                snapshot_id,
                                stream_id,
                                file.path,
                                r.name,
                                r.kind,
                                r.container,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, line: i64) -> SymbolRow {
        SymbolRow {
            name: name.into(),
            kind: "method".into(),
            container: Some("Widget".into()),
            line,
            col: 5,
            start_line: line - 1,
            start_col: 1,
            end_line: line + 2,
            end_col: 1,
        }
    }

    /// tsk571: two symbols with one name path in a file (overloads, a
    /// getter and a setter) get distinct refs — the later ones numbered
    /// in position order — and the extent is stored beside the name.
    #[tokio::test]
    async fn same_named_symbols_get_distinct_refs() {
        let db = Database::in_memory();
        db.call(|c| {
            c.execute(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source,
                                      worktree_path, created_at, updated_at)
                 VALUES (1, 'primary', 's', 'main', 'r', 'r', '/r', 'now', 'now')",
                [],
            )
            .map(|_| ())
        })
        .await
        .unwrap();
        let store = SqliteSymbolStore::new(db.clone());
        store
            .record(
                1,
                7,
                vec![FileSymbols {
                    path: "w.py".into(),
                    language: "python".into(),
                    rows: Some(vec![row("value", 20), row("value", 10), row("spin", 30)]),
                }],
                SymbolCapture::default(),
            )
            .await
            .unwrap();
        let refs: Vec<(String, i64, i64, i64)> = db
            .call(|c| {
                let mut stmt =
                    c.prepare("SELECT ref, line, start_line, end_line FROM symbol ORDER BY line")?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>();
                rows
            })
            .await
            .unwrap();
        assert_eq!(
            refs,
            vec![
                ("symbol:w.py/Widget::value@snap:7".into(), 10, 9, 12),
                ("symbol:w.py/Widget::value~2@snap:7".into(), 20, 19, 22),
                ("symbol:w.py/Widget::spin@snap:7".into(), 30, 29, 32),
            ]
        );
    }
}
