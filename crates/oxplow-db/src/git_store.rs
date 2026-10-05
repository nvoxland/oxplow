//! Git history and branches, as the semantic layer reads them (`v_commit`,
//! `v_commit_file`, `v_branch`; `v_commit_task` comes from `page_ref`).
//! Written by the commit indexer and the branch refresh in `oxplow-app`.
//! See `.context/semantic-layer.md`.

use oxplow_domain::{DomainError, Timestamp};

use crate::database::map_sql_err;
use crate::database::ts_to_string;
use crate::Database;

/// One commit, as the indexer read it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GitCommitRow {
    pub sha: String,
    pub author: String,
    pub email: String,
    /// Commit time, seconds since the epoch.
    pub committed_secs: i64,
    pub subject: String,
    pub body: String,
    pub parents: Vec<String>,
    pub files: Vec<GitCommitFileRow>,
}

/// A file a commit changed (against its first parent).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GitCommitFileRow {
    pub path: String,
    pub status: String,
    pub additions: i64,
    pub deletions: i64,
}

/// A branch and the commit it points at.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GitBranchRow {
    /// Short name (`main`, or `feature` for `origin/feature`).
    pub name: String,
    /// `local` or `remote`.
    pub kind: String,
    pub remote: Option<String>,
    pub head_sha: Option<String>,
    /// The stream whose worktree has it checked out, if any.
    pub stream_id: Option<i64>,
    /// The repository's default branch.
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct GitTagRow {
    pub name: String,
    pub sha: String,
}

#[derive(Clone)]
pub struct SqliteGitStore {
    db: Database,
}

impl SqliteGitStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Whether `sha` is stored.
    pub async fn has_commit(&self, sha: &str) -> Result<bool, DomainError> {
        let sha = sha.to_string();
        self.db
            .call(move |c| {
                c.query_row(
                    "SELECT EXISTS(SELECT 1 FROM git_commit WHERE sha = ?1)",
                    [sha],
                    |r| r.get::<_, i64>(0),
                )
                .map(|n| n != 0)
            })
            .await
    }

    /// The paths `sha` changed, as stored.
    pub async fn commit_paths(&self, sha: &str) -> Result<Vec<String>, DomainError> {
        let sha = sha.to_string();
        self.db
            .call(move |c| {
                let mut stmt =
                    c.prepare("SELECT path FROM git_commit_file WHERE sha = ?1 ORDER BY path")?;
                let rows = stmt.query_map([sha], |r| r.get(0))?;
                rows.collect()
            })
            .await
    }

    /// The commits stored as committed at or after `since`, oldest first:
    /// `(sha, committed_at)`.
    pub async fn commits_since(
        &self,
        since: Timestamp,
    ) -> Result<Vec<(String, Timestamp)>, DomainError> {
        let since = ts_to_string(since);
        self.db
            .call(move |c| {
                let mut stmt = c.prepare(
                    "SELECT sha, committed_at FROM git_commit WHERE committed_at >= ?1
                     ORDER BY committed_at, sha",
                )?;
                let rows = stmt.query_map([since], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .map(|rows| {
                rows.into_iter()
                    .filter_map(|(sha, at)| Some((sha, Timestamp::parse(&at).ok()?)))
                    .collect()
            })
    }

    /// Store (or restate) a commit and its files.
    pub async fn upsert_commit(&self, row: GitCommitRow) -> Result<(), DomainError> {
        let committed_at = ts_to_string(Timestamp::from_unix_ms(row.committed_secs * 1000));
        let parents = serde_json::to_string(&row.parents).unwrap_or_else(|_| "[]".into());
        self.db
            .transaction(move |tx| {
                tx.execute(
                    "INSERT INTO git_commit (sha, author, email, committed_at, subject, body, parents_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(sha) DO UPDATE SET
                        author = excluded.author, email = excluded.email,
                        committed_at = excluded.committed_at, subject = excluded.subject,
                        body = excluded.body, parents_json = excluded.parents_json",
                    rusqlite::params![
                        row.sha,
                        row.author,
                        row.email,
                        committed_at,
                        row.subject,
                        row.body,
                        parents
                    ],
                )
                .map_err(map_sql_err)?;
                tx.execute("DELETE FROM git_commit_file WHERE sha = ?1", [&row.sha])
                    .map_err(map_sql_err)?;
                for f in &row.files {
                    tx.execute(
                        "INSERT OR REPLACE INTO git_commit_file (sha, path, status, additions, deletions)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        rusqlite::params![row.sha, f.path, f.status, f.additions, f.deletions],
                    )
                    .map_err(map_sql_err)?;
                }
                Ok(())
            })
            .await
    }

    /// Replace the branch list.
    pub async fn replace_branches(&self, rows: Vec<GitBranchRow>) -> Result<(), DomainError> {
        let at = ts_to_string(Timestamp::now());
        self.db
            .transaction(move |tx| {
                tx.execute("DELETE FROM git_branch", [])
                    .map_err(map_sql_err)?;
                for b in &rows {
                    tx.execute(
                        "INSERT OR REPLACE INTO git_branch
                           (name, kind, remote, head_sha, stream_id, updated_at, is_default)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        rusqlite::params![
                            b.name,
                            b.kind,
                            b.remote,
                            b.head_sha,
                            b.stream_id,
                            at,
                            b.is_default
                        ],
                    )
                    .map_err(map_sql_err)?;
                }
                Ok(())
            })
            .await
    }
}

impl SqliteGitStore {
    /// Replace the tag list.
    pub async fn replace_tags(&self, rows: Vec<GitTagRow>) -> Result<(), DomainError> {
        let at = ts_to_string(Timestamp::now());
        self.db
            .transaction(move |tx| {
                tx.execute("DELETE FROM git_tag", []).map_err(map_sql_err)?;
                for t in &rows {
                    tx.execute(
                        "INSERT OR REPLACE INTO git_tag (name, sha, updated_at) VALUES (?1, ?2, ?3)",
                        rusqlite::params![t.name, t.sha, at],
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

    async fn rows(sl: &SemanticLayer, sql: &str) -> serde_json::Value {
        serde_json::to_value(sl.query_sql(sql, vec![], None).await.unwrap().rows).unwrap()
    }

    #[tokio::test]
    async fn commits_files_and_branches_read_through_their_views() {
        let db = Database::in_memory();
        let store = SqliteGitStore::new(db.clone());
        let sl = SemanticLayer::new(db);
        assert!(!store.has_commit("abc").await.unwrap());
        let commit = GitCommitRow {
            sha: "abc".into(),
            author: "Ada".into(),
            email: "ada@x".into(),
            committed_secs: 1_800_000_000,
            subject: "Fix it".into(),
            body: "Refs tsk1".into(),
            parents: vec!["p1".into(), "p2".into()],
            files: vec![GitCommitFileRow {
                path: "src/a.rs".into(),
                status: "modified".into(),
                additions: 3,
                deletions: 1,
            }],
        };
        store.upsert_commit(commit.clone()).await.unwrap();
        // Restating a commit replaces its files rather than piling up.
        store.upsert_commit(commit).await.unwrap();
        assert!(store.has_commit("abc").await.unwrap());
        assert_eq!(
            rows(&sl, "SELECT sha, author, subject, first_parent, parent_count, substr(committed_at, 1, 10) FROM v_commit").await,
            json!([["abc", "Ada", "Fix it", "p1", 2, "2027-01-15"]])
        );
        assert_eq!(
            rows(
                &sl,
                "SELECT sha, path, status, additions, deletions FROM v_commit_file"
            )
            .await,
            json!([["abc", "src/a.rs", "modified", 3, 1]])
        );

        store
            .replace_branches(vec![
                GitBranchRow {
                    name: "main".into(),
                    kind: "local".into(),
                    remote: None,
                    head_sha: Some("abc".into()),
                    stream_id: Some(1),
                    is_default: true,
                },
                GitBranchRow {
                    name: "main".into(),
                    kind: "remote".into(),
                    remote: Some("origin".into()),
                    head_sha: Some("abc".into()),
                    stream_id: None,
                    is_default: false,
                },
            ])
            .await
            .unwrap();
        assert_eq!(
            rows(
                &sl,
                "SELECT name, kind, remote, head_sha, stream_id FROM v_branch ORDER BY kind"
            )
            .await,
            json!([
                ["main", "local", null, "abc", 1],
                ["main", "remote", "origin", "abc", null]
            ])
        );
    }
}
