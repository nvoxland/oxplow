//! Stored change analysis (`v_change*`): one `change` per diff plus its
//! analyzed files, functions, imports, co-change and duplicates. Written by
//! core's producer (`oxplow-app/src/change_analysis.rs`); read by lenses.
//! See `.context/semantic-layer.md`.

use oxplow_domain::vcs::Revision;
use oxplow_domain::DomainError;
use rusqlite::OptionalExtension;
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
    /// The older side (`oxplow_domain::vcs::Revision`); `None` for a
    /// root commit.
    pub base_revision: Option<Revision>,
    /// The newer side.
    pub head_revision: Option<Revision>,
    pub status: String,
    pub error: Option<String>,
    pub computed_at: Option<String>,
    /// The stream's snapshot the analysis was computed against.
    pub snapshot_id: Option<i64>,
    /// The event log's highest seq when the analysis began.
    pub events_to: Option<i64>,
    /// What the analysis was computed from (the build, its trees, an
    /// effort's own files); a rerun from the same needn't recompute.
    pub analyzed_from: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChangeFileRow {
    pub path: String,
    pub status: String,
    pub additions: i64,
    pub deletions: i64,
    pub zone: Option<String>,
    pub is_test: bool,
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
    pub test_files: Vec<ChangeTestFileRow>,
}

#[derive(Clone)]
pub struct SqliteChangeStore {
    db: Database,
}

/// A stored revision column (`NULL` → `None`).
fn revision_at(r: &rusqlite::Row<'_>, i: usize) -> rusqlite::Result<Option<Revision>> {
    r.get::<_, Option<String>>(i)?
        .map(|s| {
            s.parse().map_err(|e: String| {
                rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, e.into())
            })
        })
        .transpose()
}

impl SqliteChangeStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// The change keyed `(stream_id, kind, target)`, created `pending` if
    /// new, with its revisions updated. The flag says whether an existing
    /// row's head moved (its `head_revision` differs from the given one):
    /// its stored results are then of another head and must be recomputed.
    /// A change whose revisions are already these is only read: a write
    /// would announce `v_change`, and a page re-asks on every announcement
    /// (tsk1024).
    pub async fn get_or_create(
        &self,
        stream_id: i64,
        kind: &str,
        target: &str,
        base: Option<&Revision>,
        head: &Revision,
    ) -> Result<(ChangeRow, bool), DomainError> {
        let (kind, target) = (kind.to_string(), target.to_string());
        let base = base.map(Revision::to_string);
        let head = head.to_string();
        let (id, moved): (i64, bool) = self
            .db
            .call(move |c| {
                let existing: Option<(i64, Option<String>, Option<String>)> = c
                    .query_row(
                        "SELECT id, base_revision, head_revision FROM change
                         WHERE stream_id = ?1 AND kind = ?2 AND target = ?3",
                        rusqlite::params![stream_id, kind, target],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                let Some((id, stored_base, stored_head)) = existing else {
                    c.execute(
                        "INSERT INTO change (stream_id, kind, target, base_revision, head_revision, status)
                         VALUES (?1, ?2, ?3, ?4, ?5, 'pending')",
                        rusqlite::params![stream_id, kind, target, base, head],
                    )?;
                    return Ok((c.last_insert_rowid(), false));
                };
                let moved = stored_head.as_deref() != Some(head.as_str());
                let rebased = base.is_some() && base != stored_base;
                if moved || rebased {
                    c.execute(
                        "UPDATE change SET base_revision = coalesce(?2, base_revision), head_revision = ?3
                         WHERE id = ?1",
                        rusqlite::params![id, base, head],
                    )?;
                }
                Ok((id, moved))
            })
            .await?;
        let row = self.get(id).await?.ok_or(DomainError::NotFound)?;
        Ok((row, moved))
    }

    pub async fn get(&self, id: i64) -> Result<Option<ChangeRow>, DomainError> {
        self.db
            .call(move |c| {
                let mut st = c.prepare(
                    "SELECT id, stream_id, kind, target, base_revision, head_revision, status, error, computed_at,
                            snapshot_id, events_to, analyzed_from
                     FROM change WHERE id = ?1",
                )?;
                let mut rows = st.query_map([id], |r| {
                    Ok(ChangeRow {
                        id: r.get(0)?,
                        stream_id: r.get(1)?,
                        kind: r.get(2)?,
                        target: r.get(3)?,
                        base_revision: revision_at(r, 4)?,
                        head_revision: revision_at(r, 5)?,
                        status: r.get(6)?,
                        error: r.get(7)?,
                        computed_at: r.get(8)?,
                        snapshot_id: r.get(9)?,
                        events_to: r.get(10)?,
                        analyzed_from: r.get(11)?,
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

    /// Replace the change's analysis and mark it `done`, computed against
    /// `snapshot_id` with its inputs as of `events_to`.
    pub async fn store_results(
        &self,
        id: i64,
        results: ChangeResults,
        snapshot_id: Option<i64>,
        events_to: i64,
        elapsed_ms: i64,
        analyzed_from: Option<String>,
    ) -> Result<(), DomainError> {
        let at = serde_json::to_value(oxplow_domain::Timestamp::now())
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        self.db
            .transaction(move |tx| {
                for t in ["change_function", "change_import", "change_test_file"] {
                    tx.execute(&format!("DELETE FROM {t} WHERE change_id = ?1"), [id])
                        .map_err(map_sql_err)?;
                }
                write_files(tx, id, &results.files, &at, events_to)?;
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
                    "UPDATE change SET status = 'done', error = NULL, computed_at = ?2,
                       snapshot_id = ?3, events_to = ?4, elapsed_ms = ?5, analyzed_from = ?6
                     WHERE id = ?1",
                    rusqlite::params![id, at, snapshot_id, events_to, elapsed_ms, analyzed_from],
                )
                .map_err(map_sql_err)?;
                Ok(())
            })
            .await
    }

    /// Stage one: replace the change's files — status and line
    /// counts — alone, stamped with what they saw, and for a working tree
    /// git's in-progress operation and conflict count. The deep analysis
    /// is left as it was. A list older than the stored one is dropped.
    pub async fn store_files(
        &self,
        id: i64,
        files: Vec<ChangeFileRow>,
        events_to: i64,
        conflicted: Option<i64>,
        in_progress: Option<String>,
    ) -> Result<(), DomainError> {
        let at = serde_json::to_value(oxplow_domain::Timestamp::now())
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        self.db
            .transaction(move |tx| {
                if write_files(tx, id, &files, &at, events_to)? {
                    tx.execute(
                        "UPDATE change SET conflicted = ?2, in_progress = ?3 WHERE id = ?1",
                        rusqlite::params![id, conflicted, in_progress],
                    )
                    .map_err(map_sql_err)?;
                }
                Ok(())
            })
            .await
    }

    /// Replace the change's duplicates (they arrive later, from a slower
    /// scan) — when they belong to its latest analysis (`events_to`); a
    /// newer analysis supersedes them. Whether they were stored.
    pub async fn store_duplicates(
        &self,
        id: i64,
        rows: Vec<ChangeDuplicateRow>,
        events_to: i64,
    ) -> Result<bool, DomainError> {
        self.db
            .transaction(move |tx| {
                let latest: Option<i64> = tx
                    .query_row("SELECT events_to FROM change WHERE id = ?1", [id], |r| r.get(0))
                    .optional()
                    .map_err(map_sql_err)?
                    .flatten();
                if latest != Some(events_to) {
                    return Ok(false);
                }
                tx.execute("DELETE FROM change_duplicate WHERE change_id = ?1", [id])
                    .map_err(map_sql_err)?;
                tx.execute(
                    "UPDATE change SET duplicates_events_to = ?2 WHERE id = ?1",
                    rusqlite::params![id, events_to],
                )
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
                Ok(true)
            })
            .await
    }
}

/// A change analyzed since its duplicates were last stored: what a scan
/// of it needs.
#[derive(Debug, Clone, PartialEq)]
pub struct AwaitingDuplicates {
    pub id: i64,
    pub stream_id: i64,
    /// The revision its files are read at.
    pub head: Revision,
    /// The analysis the scan belongs to.
    pub events_to: i64,
    /// Its changed files, the scan's scope.
    pub paths: Vec<String>,
}

impl SqliteChangeStore {
    /// The analyzed changes whose duplicates weren't stored for their
    /// latest analysis (a scan a stop cut off), with changed files to
    /// scan.
    pub async fn awaiting_duplicates(&self) -> Result<Vec<AwaitingDuplicates>, DomainError> {
        self.db
            .call(|c| {
                let mut st = c.prepare(
                    "SELECT c.id, c.stream_id, c.head_revision, c.events_to,
                            (SELECT json_group_array(path) FROM
                               (SELECT f.path FROM change_file f WHERE f.change_id = c.id ORDER BY f.path))
                     FROM change c
                     WHERE c.status = 'done' AND c.events_to IS NOT NULL
                       AND c.duplicates_events_to IS NOT c.events_to
                       AND EXISTS (SELECT 1 FROM change_file f WHERE f.change_id = c.id)
                     ORDER BY c.id",
                )?;
                let rows = st.query_map([], |r| {
                    let paths: String = r.get(4)?;
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        revision_at(r, 2)?,
                        r.get::<_, i64>(3)?,
                        paths,
                    ))
                })?;
                let mut out = Vec::new();
                for row in rows {
                    let (id, stream_id, head, events_to, paths) = row?;
                    let Some(head) = head else { continue };
                    out.push(AwaitingDuplicates {
                        id,
                        stream_id,
                        head,
                        events_to,
                        paths: serde_json::from_str(&paths).unwrap_or_default(),
                    });
                }
                Ok(out)
            })
            .await
    }
}

/// Replace `id`'s file rows and stamp their freshness, unless the stored
/// list saw later events (stage one and the deep analysis both write it).
/// Whether they were written.
fn write_files(
    tx: &rusqlite::Transaction<'_>,
    id: i64,
    files: &[ChangeFileRow],
    at: &str,
    events_to: i64,
) -> Result<bool, DomainError> {
    let stored: Option<i64> = tx
        .query_row(
            "SELECT files_events_to FROM change WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sql_err)?
        .flatten();
    if stored.is_some_and(|s| s > events_to) {
        return Ok(false);
    }
    tx.execute("DELETE FROM change_file WHERE change_id = ?1", [id])
        .map_err(map_sql_err)?;
    for f in files {
        tx.execute(
            "INSERT INTO change_file (change_id, path, status, additions, deletions, zone, is_test)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                id,
                f.path,
                f.status,
                f.additions,
                f.deletions,
                f.zone,
                i64::from(f.is_test)
            ],
        )
        .map_err(map_sql_err)?;
    }
    tx.execute(
        "UPDATE change SET files_at = ?2, files_events_to = ?3 WHERE id = ?1",
        rusqlite::params![id, at, events_to],
    )
    .map_err(map_sql_err)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SemanticLayer;
    use serde_json::json;

    /// An effort analyzed while open (head: the working tree) that has since
    /// closed (head: its end snapshot) must be recomputed; the store says
    /// when the head moved.
    #[tokio::test]
    async fn get_or_create_reports_a_moved_head() {
        let store = SqliteChangeStore::new(Database::in_memory());
        let (_, moved) = store
            .get_or_create(1, "effort", "7", None, &Revision::Working)
            .await
            .unwrap();
        assert!(!moved, "new rows didn't move");
        let (_, moved) = store
            .get_or_create(1, "effort", "7", None, &Revision::Working)
            .await
            .unwrap();
        assert!(!moved);
        let (row, moved) = store
            .get_or_create(1, "effort", "7", None, &Revision::Snapshot(12))
            .await
            .unwrap();
        assert!(moved);
        assert_eq!(row.head_revision, Some(Revision::Snapshot(12)));
    }

    /// tsk1024: asking for a change that is already there writes nothing,
    /// so it announces nothing: a page re-asks whenever `v_change` changes,
    /// and a write here would make it ask forever.
    #[tokio::test]
    async fn asking_for_an_unchanged_change_writes_nothing() {
        let db = Database::in_memory();
        let store = SqliteChangeStore::new(db.clone());
        let base = Revision::git("abc^");
        let head = Revision::git("abc");
        store
            .get_or_create(1, "commit", "abc", Some(&base), &head)
            .await
            .unwrap();
        let mut rx = db.subscribe_changes();
        store
            .get_or_create(1, "commit", "abc", Some(&base), &head)
            .await
            .unwrap();
        store
            .get_or_create(1, "commit", "abc", None, &head)
            .await
            .unwrap();
        assert!(
            rx.try_recv().is_err(),
            "an unchanged change announced a write"
        );
        let (_, moved) = store
            .get_or_create(1, "commit", "abc", Some(&base), &Revision::git("def"))
            .await
            .unwrap();
        assert!(moved);
        assert!(rx.try_recv().is_ok(), "a moved head is written");
    }

    #[tokio::test]
    async fn changes_are_keyed_and_results_replace_and_read_through_views() {
        let db = Database::in_memory();
        let store = SqliteChangeStore::new(db.clone());
        let c = store
            .get_or_create(
                1,
                "commit",
                "abc",
                Some(&Revision::git("abc^")),
                &Revision::git("abc"),
            )
            .await
            .unwrap()
            .0;
        assert_eq!(c.status, "pending");
        let again = store
            .get_or_create(1, "commit", "abc", None, &Revision::git("abc"))
            .await
            .unwrap()
            .0;
        assert_eq!(again.id, c.id, "same key, same change");
        let other = store
            .get_or_create(1, "working", "", None, &Revision::Working)
            .await
            .unwrap()
            .0;
        assert_ne!(other.id, c.id);

        let results = |path: &str| ChangeResults {
            files: vec![ChangeFileRow {
                path: path.into(),
                status: "modified".into(),
                additions: 3,
                deletions: 1,
                zone: Some("core".into()),
                is_test: false,
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
            test_files: Vec::new(),
        };
        store
            .store_results(c.id, results("a.rs"), Some(3), 10, 1, None)
            .await
            .unwrap();
        store
            .store_results(c.id, results("b.rs"), Some(4), 11, 1, None)
            .await
            .unwrap();
        let dup = || {
            vec![ChangeDuplicateRow {
                path: "b.rs".into(),
                start_line: 1,
                end_line: 9,
                lines: 9,
                peer_path: "c.rs".into(),
                peer_start_line: 20,
                peer_end_line: 28,
            }]
        };
        // Analyzed, its duplicates not yet stored: it awaits them, with
        // what a scan needs.
        let awaiting = store.awaiting_duplicates().await.unwrap();
        assert_eq!(awaiting.len(), 1);
        assert_eq!(
            (
                awaiting[0].id,
                awaiting[0].events_to,
                awaiting[0].paths.clone()
            ),
            (c.id, 11, vec!["b.rs".to_string()])
        );
        // A scan of the superseded analysis stores nothing; the latest's does.
        assert!(!store.store_duplicates(c.id, dup(), 10).await.unwrap());
        assert!(!store.awaiting_duplicates().await.unwrap().is_empty());
        assert!(store.store_duplicates(c.id, dup(), 11).await.unwrap());
        assert!(store.awaiting_duplicates().await.unwrap().is_empty());
        let row = store.get(c.id).await.unwrap().unwrap();
        assert_eq!(
            (row.status.as_str(), row.snapshot_id, row.events_to),
            ("done", Some(4), Some(11))
        );

        let sl = SemanticLayer::new(db);
        let q = |sql: &'static str| {
            let sl = sl.clone();
            async move {
                serde_json::to_value(sl.query_sql(sql, vec![], None).await.unwrap().rows).unwrap()
            }
        };
        assert_eq!(
            q("SELECT path, zone, is_test FROM v_change_file").await,
            json!([["b.rs", "core", 0]])
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
            q("SELECT peer_path, lines FROM v_change_duplicate").await,
            json!([["c.rs", 9]])
        );
        assert_eq!(
            q("SELECT kind, target, status FROM v_change ORDER BY id").await,
            json!([["commit", "abc", "done"], ["working", "", "pending"]])
        );
    }
}
