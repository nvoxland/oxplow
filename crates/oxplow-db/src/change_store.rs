//! Stored change analysis (`v_change*`): one `change` per diff plus its
//! analyzed files, functions, imports, co-change and duplicates. Written by
//! core's producer (`oxplow-app/src/change_analysis.rs`); read by lenses.
//! See `.context/semantic-layer.md`.

use oxplow_domain::DomainError;
use serde::{Deserialize, Serialize};

use crate::database::map_sql_err;
use crate::Database;

/// A change's identity and state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRow {
    pub id: i64,
    pub stream_id: i64,
    pub kind: String,
    pub target: String,
    pub base_label: Option<String>,
    pub head_label: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub computed_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChangeFileRow {
    pub path: String,
    pub status: String,
    pub additions: i64,
    pub deletions: i64,
    pub zone: Option<String>,
    pub is_test: bool,
    pub interest: f64,
    pub interest_reasons: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChangeFunctionRow {
    pub path: String,
    pub container: String,
    pub name: String,
    pub status: String,
    pub signature_changed: bool,
    pub body_changed: bool,
    pub start_line: i64,
    pub visibility: String,
    pub is_test: bool,
    pub complexity: Option<f64>,
    pub length: Option<i64>,
    pub params_before: Option<i64>,
    pub params_after: Option<i64>,
    pub complexity_delta: Option<f64>,
    pub length_delta: Option<i64>,
    pub added_lines: Option<i64>,
    pub deleted_lines: Option<i64>,
    pub modified_lines: Option<i64>,
    pub churn_share: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChangeImportRow {
    pub path: String,
    pub module: String,
    pub direction: String,
    pub start_line: Option<i64>,
    pub from_zone: Option<String>,
    pub to_zone: Option<String>,
    pub cross_zone: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChangeCoChangeRow {
    pub path: String,
    pub reason: String,
    pub expected: Option<String>,
    pub dormant_days: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChangeDuplicateRow {
    pub path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub lines: i64,
    pub peer_path: String,
    pub peer_start_line: i64,
    pub peer_end_line: i64,
}

/// How much one changed file's tests check, before and after.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChangeTestFileRow {
    pub path: String,
    pub tests_before: i64,
    pub tests_after: i64,
    pub assertions_before: i64,
    pub assertions_after: i64,
    pub skips_before: i64,
    pub skips_after: i64,
}

/// Everything the producer computed for one change.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChangeResults {
    pub files: Vec<ChangeFileRow>,
    pub functions: Vec<ChangeFunctionRow>,
    pub imports: Vec<ChangeImportRow>,
    pub co_changes: Vec<ChangeCoChangeRow>,
    pub test_files: Vec<ChangeTestFileRow>,
}

#[derive(Clone)]
pub struct SqliteChangeStore {
    db: Database,
}

impl SqliteChangeStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// The change for `(stream_id, kind, target)`, creating it `pending`.
    pub async fn get_or_create(
        &self,
        stream_id: i64,
        kind: &str,
        target: &str,
        base_label: Option<String>,
        head_label: Option<String>,
    ) -> Result<ChangeRow, DomainError> {
        let (kind, target) = (kind.to_string(), target.to_string());
        let id: i64 = self
            .db
            .call(move |c| {
                c.execute(
                    "INSERT INTO change (stream_id, kind, target, base_label, head_label, status)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'pending')
                     ON CONFLICT (stream_id, kind, target) DO UPDATE SET
                       base_label = coalesce(excluded.base_label, change.base_label),
                       head_label = coalesce(excluded.head_label, change.head_label)",
                    rusqlite::params![stream_id, kind, target, base_label, head_label],
                )?;
                c.query_row(
                    "SELECT id FROM change WHERE stream_id = ?1 AND kind = ?2 AND target = ?3",
                    rusqlite::params![stream_id, kind, target],
                    |r| r.get(0),
                )
            })
            .await?;
        self.get(id).await?.ok_or(DomainError::NotFound)
    }

    pub async fn get(&self, id: i64) -> Result<Option<ChangeRow>, DomainError> {
        self.db
            .call(move |c| {
                let mut st = c.prepare(
                    "SELECT id, stream_id, kind, target, base_label, head_label, status, error, computed_at
                     FROM change WHERE id = ?1",
                )?;
                let mut rows = st.query_map([id], |r| {
                    Ok(ChangeRow {
                        id: r.get(0)?,
                        stream_id: r.get(1)?,
                        kind: r.get(2)?,
                        target: r.get(3)?,
                        base_label: r.get(4)?,
                        head_label: r.get(5)?,
                        status: r.get(6)?,
                        error: r.get(7)?,
                        computed_at: r.get(8)?,
                    })
                })?;
                rows.next().transpose()
            })
            .await
    }

    pub async fn set_status(
        &self,
        id: i64,
        status: &str,
        error: Option<String>,
    ) -> Result<(), DomainError> {
        let status = status.to_string();
        self.db
            .call(move |c| {
                c.execute(
                    "UPDATE change SET status = ?2, error = ?3 WHERE id = ?1",
                    rusqlite::params![id, status, error],
                )
                .map(|_| ())
            })
            .await
    }

    /// Replace the change's analysis and mark it `done`.
    pub async fn store_results(&self, id: i64, results: ChangeResults) -> Result<(), DomainError> {
        let at = serde_json::to_value(oxplow_domain::Timestamp::now())
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        self.db
            .transaction(move |tx| {
                for t in [
                    "change_file",
                    "change_function",
                    "change_import",
                    "change_co_change",
                    "change_test_file",
                ] {
                    tx.execute(&format!("DELETE FROM {t} WHERE change_id = ?1"), [id])
                        .map_err(map_sql_err)?;
                }
                for f in &results.files {
                    tx.execute(
                        "INSERT INTO change_file (change_id, path, status, additions, deletions, zone, is_test, interest, interest_reasons)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        rusqlite::params![
                            id, f.path, f.status, f.additions, f.deletions, f.zone, i64::from(f.is_test),
                            f.interest, f.interest_reasons.join("; ")
                        ],
                    )
                    .map_err(map_sql_err)?;
                }
                for f in &results.functions {
                    tx.execute(
                        "INSERT OR REPLACE INTO change_function (change_id, path, container, name, status, signature_changed,
                           body_changed, start_line, visibility, is_test, complexity, length, params_before, params_after,
                           complexity_delta, length_delta, added_lines, deleted_lines, modified_lines, churn_share)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
                        rusqlite::params![
                            id, f.path, f.container, f.name, f.status, i64::from(f.signature_changed),
                            i64::from(f.body_changed), f.start_line, f.visibility, i64::from(f.is_test), f.complexity,
                            f.length, f.params_before, f.params_after, f.complexity_delta, f.length_delta,
                            f.added_lines, f.deleted_lines, f.modified_lines, f.churn_share
                        ],
                    )
                    .map_err(map_sql_err)?;
                }
                for i in &results.imports {
                    tx.execute(
                        "INSERT INTO change_import (change_id, path, module, direction, start_line, from_zone, to_zone, cross_zone)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        rusqlite::params![
                            id, i.path, i.module, i.direction, i.start_line, i.from_zone, i.to_zone,
                            i64::from(i.cross_zone)
                        ],
                    )
                    .map_err(map_sql_err)?;
                }
                for c in &results.co_changes {
                    tx.execute(
                        "INSERT OR REPLACE INTO change_co_change (change_id, path, reason, expected, dormant_days)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        rusqlite::params![id, c.path, c.reason, c.expected, c.dormant_days],
                    )
                    .map_err(map_sql_err)?;
                }
                for t in &results.test_files {
                    tx.execute(
                        "INSERT OR REPLACE INTO change_test_file (change_id, path, tests_before, tests_after,
                           assertions_before, assertions_after, skips_before, skips_after)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        rusqlite::params![
                            id, t.path, t.tests_before, t.tests_after, t.assertions_before,
                            t.assertions_after, t.skips_before, t.skips_after
                        ],
                    )
                    .map_err(map_sql_err)?;
                }
                tx.execute(
                    "UPDATE change SET status = 'done', error = NULL, computed_at = ?2 WHERE id = ?1",
                    rusqlite::params![id, at],
                )
                .map_err(map_sql_err)?;
                Ok(())
            })
            .await
    }

    /// Replace the change's duplicates (they arrive later, from a slower scan).
    pub async fn store_duplicates(
        &self,
        id: i64,
        rows: Vec<ChangeDuplicateRow>,
    ) -> Result<(), DomainError> {
        self.db
            .transaction(move |tx| {
                tx.execute("DELETE FROM change_duplicate WHERE change_id = ?1", [id])
                    .map_err(map_sql_err)?;
                for d in &rows {
                    tx.execute(
                        "INSERT INTO change_duplicate (change_id, path, start_line, end_line, lines, peer_path,
                           peer_start_line, peer_end_line)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        rusqlite::params![
                            id, d.path, d.start_line, d.end_line, d.lines, d.peer_path, d.peer_start_line,
                            d.peer_end_line
                        ],
                    )
                    .map_err(map_sql_err)?;
                }
                Ok(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SemanticLayer;
    use serde_json::json;

    #[tokio::test]
    async fn changes_are_keyed_and_results_replace_and_read_through_views() {
        let db = Database::in_memory();
        let store = SqliteChangeStore::new(db.clone());
        let c = store
            .get_or_create(1, "commit", "abc", Some("abc^".into()), Some("abc".into()))
            .await
            .unwrap();
        assert_eq!(c.status, "pending");
        let again = store
            .get_or_create(1, "commit", "abc", None, None)
            .await
            .unwrap();
        assert_eq!(again.id, c.id, "same key, same change");
        let other = store
            .get_or_create(1, "working", "", None, None)
            .await
            .unwrap();
        assert_ne!(other.id, c.id);

        let results = |path: &str| ChangeResults {
            files: vec![ChangeFileRow {
                path: path.into(),
                status: "modified".into(),
                additions: 3,
                deletions: 1,
                zone: Some("core".into()),
                is_test: false,
                interest: 2.5,
                interest_reasons: vec!["complexity +2 across 1 fn".into()],
            }],
            functions: vec![ChangeFunctionRow {
                path: path.into(),
                container: "Foo".into(),
                name: "bar".into(),
                status: "modified".into(),
                body_changed: true,
                start_line: 4,
                visibility: "public".into(),
                complexity_delta: Some(2.0),
                ..Default::default()
            }],
            imports: vec![ChangeImportRow {
                path: path.into(),
                module: "crate::db".into(),
                direction: "added".into(),
                cross_zone: true,
                ..Default::default()
            }],
            co_changes: vec![ChangeCoChangeRow {
                path: path.into(),
                reason: "dormant".into(),
                dormant_days: Some(120),
                ..Default::default()
            }],
            test_files: Vec::new(),
        };
        store.store_results(c.id, results("a.rs")).await.unwrap();
        store.store_results(c.id, results("b.rs")).await.unwrap();
        store
            .store_duplicates(
                c.id,
                vec![ChangeDuplicateRow {
                    path: "b.rs".into(),
                    start_line: 1,
                    end_line: 9,
                    lines: 9,
                    peer_path: "c.rs".into(),
                    peer_start_line: 20,
                    peer_end_line: 28,
                }],
            )
            .await
            .unwrap();
        assert_eq!(store.get(c.id).await.unwrap().unwrap().status, "done");

        let sl = SemanticLayer::new(db);
        let q = |sql: &'static str| {
            let sl = sl.clone();
            async move {
                serde_json::to_value(sl.query_sql(sql, vec![], None).await.unwrap().rows).unwrap()
            }
        };
        assert_eq!(
            q("SELECT path, zone, interest, interest_reasons FROM v_change_file").await,
            json!([["b.rs", "core", 2.5, "complexity +2 across 1 fn"]])
        );
        assert_eq!(
            q("SELECT container, name, body_changed FROM v_change_function").await,
            json!([["Foo", "bar", 1]])
        );
        assert_eq!(
            q("SELECT module, cross_zone FROM v_change_import").await,
            json!([["crate::db", 1]])
        );
        assert_eq!(
            q("SELECT reason, dormant_days FROM v_change_co_change").await,
            json!([["dormant", 120]])
        );
        assert_eq!(
            q("SELECT peer_path, lines FROM v_change_duplicate").await,
            json!([["c.rs", 9]])
        );
        assert_eq!(
            q("SELECT kind, target, status FROM v_change ORDER BY id").await,
            json!([["commit", "abc", "done"], ["working", "", "pending"]])
        );
    }
}
