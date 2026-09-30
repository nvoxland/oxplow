//! LSP diagnostics the language servers have published this session
//! (`lsp_diagnostic`, read through `v_diagnostic`). Live state: cleared at
//! boot and when a server (re)starts. Written by `oxplow-app`'s
//! `lsp_diagnostics`. See `.context/semantic-layer.md`.

use oxplow_domain::{DomainError, Timestamp};

use crate::database::map_sql_err;
use crate::database::ts_to_string;
use crate::Database;

/// One diagnostic, positions 1-based.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DiagnosticRow {
    /// `error`, `warning`, `information` or `hint`.
    pub severity: String,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<String>,
    pub line: i64,
    pub col: i64,
    pub end_line: i64,
    pub end_col: i64,
}

pub use oxplow_domain::events::schema::DiagnosticCounts;

#[derive(Clone)]
pub struct SqliteDiagnosticStore {
    db: Database,
}

impl SqliteDiagnosticStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Replace one file's diagnostics from one server (an empty list
    /// clears them, which is how a server says the file is clean).
    pub async fn replace_file(
        &self,
        stream_id: i64,
        language: String,
        path: String,
        rows: Vec<DiagnosticRow>,
    ) -> Result<(), DomainError> {
        let at = ts_to_string(Timestamp::now());
        self.db
            .transaction(move |tx| {
                tx.execute(
                    "DELETE FROM lsp_diagnostic WHERE stream_id = ?1 AND language = ?2 AND path = ?3",
                    rusqlite::params![stream_id, language, path],
                )
                .map_err(map_sql_err)?;
                for r in &rows {
                    tx.execute(
                        "INSERT INTO lsp_diagnostic (stream_id, language, path, severity, message, source,
                           code, line, col, end_line, end_col, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                        rusqlite::params![
                            stream_id, language, path, r.severity, r.message, r.source, r.code,
                            r.line, r.col, r.end_line, r.end_col, at
                        ],
                    )
                    .map_err(map_sql_err)?;
                }
                Ok(())
            })
            .await
    }

    /// Drop one server's diagnostics for a stream (it restarted or
    /// stopped); the paths that had some.
    pub async fn clear_server(
        &self,
        stream_id: i64,
        language: String,
    ) -> Result<Vec<String>, DomainError> {
        self.db
            .transaction(move |tx| {
                let mut stmt = tx
                    .prepare(
                        "DELETE FROM lsp_diagnostic WHERE stream_id = ?1 AND language = ?2
                         RETURNING path",
                    )
                    .map_err(map_sql_err)?;
                let mut paths = stmt
                    .query_map(rusqlite::params![stream_id, language], |r| {
                        r.get::<_, String>(0)
                    })
                    .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
                    .map_err(map_sql_err)?;
                paths.sort();
                paths.dedup();
                Ok(paths)
            })
            .await
    }

    /// One file's diagnostics in a stream, every server's, by position.
    pub async fn list_file(
        &self,
        stream_id: i64,
        path: String,
    ) -> Result<Vec<DiagnosticRow>, DomainError> {
        self.db
            .read(move |tx| {
                let mut stmt = tx
                    .prepare(
                        "SELECT severity, message, source, code, line, col, end_line, end_col
                         FROM lsp_diagnostic WHERE stream_id = ?1 AND path = ?2
                         ORDER BY line, col, message",
                    )
                    .map_err(map_sql_err)?;
                stmt.query_map(rusqlite::params![stream_id, path], |r| {
                    Ok(DiagnosticRow {
                        severity: r.get(0)?,
                        message: r.get(1)?,
                        source: r.get(2)?,
                        code: r.get(3)?,
                        line: r.get(4)?,
                        col: r.get(5)?,
                        end_line: r.get(6)?,
                        end_col: r.get(7)?,
                    })
                })
                .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
                .map_err(map_sql_err)
            })
            .await
    }

    /// How many diagnostics of each severity one file has in a stream.
    pub async fn counts(
        &self,
        stream_id: i64,
        path: String,
    ) -> Result<DiagnosticCounts, DomainError> {
        self.db
            .read(move |tx| {
                let mut stmt = tx
                    .prepare(
                        "SELECT severity, count(*) FROM lsp_diagnostic
                         WHERE stream_id = ?1 AND path = ?2 GROUP BY severity",
                    )
                    .map_err(map_sql_err)?;
                let mut counts = DiagnosticCounts::default();
                let rows = stmt
                    .query_map(rusqlite::params![stream_id, path], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?))
                    })
                    .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
                    .map_err(map_sql_err)?;
                for (severity, n) in rows {
                    match severity.as_str() {
                        "error" => counts.error = n,
                        "warning" => counts.warning = n,
                        "information" => counts.information = n,
                        _ => counts.hint = n,
                    }
                }
                Ok(counts)
            })
            .await
    }

    /// Drop everything (at boot: no server is running yet).
    pub async fn clear_all(&self) -> Result<(), DomainError> {
        self.db
            .call(|c| c.execute("DELETE FROM lsp_diagnostic", []).map(|_| ()))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SemanticLayer;
    use serde_json::json;

    fn diag(sev: &str, msg: &str, line: i64) -> DiagnosticRow {
        DiagnosticRow {
            severity: sev.into(),
            message: msg.into(),
            source: Some("rustc".into()),
            code: Some("E0308".into()),
            line,
            col: 5,
            end_line: line,
            end_col: 9,
        }
    }

    #[tokio::test]
    async fn diagnostics_replace_per_file_and_clear_per_server() {
        let db = Database::in_memory();
        let store = SqliteDiagnosticStore::new(db.clone());
        let sl = SemanticLayer::new(db);
        let rows = |sql: &'static str| {
            let sl = sl.clone();
            async move {
                serde_json::to_value(sl.query_sql(sql, vec![], None).await.unwrap().rows).unwrap()
            }
        };
        store
            .replace_file(
                1,
                "rust".into(),
                "src/a.rs".into(),
                vec![
                    diag("error", "mismatched types", 3),
                    diag("warning", "unused", 7),
                ],
            )
            .await
            .unwrap();
        store
            .replace_file(
                1,
                "typescript".into(),
                "src/b.ts".into(),
                vec![diag("error", "no such name", 1)],
            )
            .await
            .unwrap();
        assert_eq!(
            rows("SELECT path, severity, message, line, col, source, code FROM v_diagnostic ORDER BY path, line").await,
            json!([
                ["src/a.rs", "error", "mismatched types", 3, 5, "rustc", "E0308"],
                ["src/a.rs", "warning", "unused", 7, 5, "rustc", "E0308"],
                ["src/b.ts", "error", "no such name", 1, 5, "rustc", "E0308"]
            ])
        );
        // A fresh publish for a file replaces it; an empty one clears it.
        store
            .replace_file(1, "rust".into(), "src/a.rs".into(), vec![])
            .await
            .unwrap();
        assert_eq!(
            rows("SELECT count(*) FROM v_diagnostic").await,
            json!([[1]])
        );
        store.clear_server(1, "typescript".into()).await.unwrap();
        assert_eq!(
            rows("SELECT count(*) FROM v_diagnostic").await,
            json!([[0]])
        );
    }
}
