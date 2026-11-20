//! What a script's `change_diff(change_id, paths)` answers (`tree.read`;
//! `oxplow_script::change_diff`): an analysed change's changed files, each
//! with its hunks and the lines they added and removed. The files are the
//! ones the analysis stored (`v_change_file`, so a request's own files
//! only); each side's text is read through [`Trees`] as the analyzer reads
//! it, and the ranges are [`crate::churn::diff_hunks`]'s.
//!
//! One call reads at most [`MAX_FILES`] files and [`MAX_BYTES`] of text;
//! past either it fails naming the limit, never truncating.

use std::path::Path;
use std::sync::Arc;

use oxplow_db::{Database, SqliteChangeStore};
use oxplow_domain::vcs::Revision;
use serde_json::{json, Value};

use crate::trees::Trees;
use crate::worktrees::WorktreeRouter;

/// The most files one call returns.
pub const MAX_FILES: usize = 200;
/// The most text (both sides of every file, in bytes) one call reads.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

/// Answers `change_diff` from the stores, on the runtime it was made on.
pub struct ChangeDiffReader {
    db: Database,
    changes: Arc<SqliteChangeStore>,
    trees: Arc<Trees>,
    worktrees: Arc<WorktreeRouter>,
    runtime: tokio::runtime::Handle,
}

impl ChangeDiffReader {
    /// Made inside the runtime its calls run on.
    pub fn new(
        db: Database,
        changes: Arc<SqliteChangeStore>,
        trees: Arc<Trees>,
        worktrees: Arc<WorktreeRouter>,
    ) -> Self {
        Self {
            db,
            changes,
            trees,
            worktrees,
            runtime: tokio::runtime::Handle::current(),
        }
    }

    /// The diff of change `change_id`, limited to `paths` when given.
    pub async fn read(&self, change_id: i64, paths: Option<&[String]>) -> Result<Value, String> {
        let row = self
            .changes
            .get(change_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("change {change_id} doesn't exist"))?;
        let mut files: Vec<(String, String)> = self
            .db
            .read(move |c| {
                let mut st = c
                    .prepare(
                        "SELECT path, status FROM change_file WHERE change_id = ?1 ORDER BY path",
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = st
                    .query_map([change_id], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(oxplow_db::map_sql_err)?;
                rows.collect::<Result<_, _>>()
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .map_err(|e| e.to_string())?;
        if let Some(paths) = paths {
            files.retain(|(path, _)| paths.contains(path));
        }
        if files.len() > MAX_FILES {
            return Err(format!(
                "change_diff reads at most {MAX_FILES} files per call; change {change_id} has {} here (pass `paths` to ask for fewer)",
                files.len()
            ));
        }
        let root = crate::change_analysis::stream_root(&self.worktrees, row.stream_id).await;
        let mut bytes = 0usize;
        let mut out = Vec::with_capacity(files.len());
        for (path, status) in files {
            let base = self.side(&root, row.base_revision.as_ref(), &path).await?;
            let head = match &row.head_revision {
                Some(rev) => self.side(&root, Some(rev), &path).await?,
                None => String::new(),
            };
            bytes += base.len() + head.len();
            if bytes > MAX_BYTES {
                return Err(format!(
                    "change_diff reads at most {MAX_BYTES} bytes of text per call; change {change_id} is past it at `{path}` (pass `paths` to ask for fewer files)"
                ));
            }
            out.push(json!({ "path": path, "status": status, "hunks": hunks(&base, &head) }));
        }
        Ok(Value::Array(out))
    }

    async fn side(
        &self,
        root: &Path,
        rev: Option<&Revision>,
        path: &str,
    ) -> Result<String, String> {
        let Some(rev) = rev else {
            return Ok(String::new());
        };
        let bytes = self
            .trees
            .read_at(root, rev, path)
            .await
            .map_err(|e| e.to_string())?;
        Ok(bytes
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default())
    }
}

/// `base` → `head`'s hunks with the lines each added and removed.
fn hunks(base: &str, head: &str) -> Vec<Value> {
    let base_lines: Vec<&str> = base.lines().collect();
    let head_lines: Vec<&str> = head.lines().collect();
    let lines = |all: &[&str], range: Option<(u32, u32)>| -> Vec<Value> {
        let Some((start, end)) = range else {
            return Vec::new();
        };
        (start..=end)
            .filter_map(|n| {
                all.get(n as usize - 1)
                    .map(|text| json!({ "line": n, "text": text }))
            })
            .collect()
    };
    crate::churn::diff_hunks(base, head)
        .into_iter()
        .map(|h| {
            json!({
                "head_start": h.head.map(|r| r.0),
                "head_end": h.head.map(|r| r.1),
                "base_start": h.base.map(|r| r.0),
                "base_end": h.base.map(|r| r.1),
                "head_after": h.head_after,
                "added": lines(&head_lines, h.head),
                "removed": lines(&base_lines, h.base),
            })
        })
        .collect()
}

impl oxplow_script::ChangeDiffSource for ChangeDiffReader {
    fn change_diff(&self, change_id: i64, paths: Option<Vec<String>>) -> Result<Value, String> {
        self.runtime
            .block_on(self.read(change_id, paths.as_deref()))
    }
}

/// [`ChangeDiffReader`] seen through a run's path bounds: a file the run
/// may not read is left out, as `files()` leaves it out of the tree.
pub struct Bounded {
    pub inner: Arc<dyn oxplow_script::ChangeDiffSource>,
    pub run: oxplow_domain::run::RunContext,
}

impl oxplow_script::ChangeDiffSource for Bounded {
    fn change_diff(&self, change_id: i64, paths: Option<Vec<String>>) -> Result<Value, String> {
        let mut out = self.inner.change_diff(change_id, paths)?;
        if let Some(files) = out.as_array_mut() {
            files.retain(|f| {
                f["path"]
                    .as_str()
                    .is_some_and(|path| self.run.permits_path(path))
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change_analysis::{ensure_change, ChangeTarget};

    const BASE: &str = "one\ntwo\nthree\nfour\n";
    const HEAD: &str = "one\nTWO\nfour\nfive\n";

    /// A committed change (modifying `a.txt`, adding `b.txt`) analysed,
    /// with a reader over the services' stores.
    async fn setup() -> (crate::test_fixtures::ThreadFixture, ChangeDiffReader, i64) {
        let f = crate::test_fixtures::services_with_thread().await;
        let root = f.svc.layout.project_dir.clone();
        std::fs::write(root.join("a.txt"), BASE).unwrap();
        crate::test_fixtures::commit_all(&root, "base");
        std::fs::write(root.join("a.txt"), HEAD).unwrap();
        std::fs::write(root.join("b.txt"), "new\n").unwrap();
        let sha = crate::test_fixtures::commit_all(&root, "change");
        let c = ensure_change(
            &f.svc,
            ChangeTarget::Commit {
                sha,
                stream_id: None,
            },
        )
        .await
        .unwrap();
        let reader = ChangeDiffReader::new(
            f.svc.db.clone(),
            f.svc.change_store.clone(),
            f.svc.trees.clone(),
            f.svc.worktrees.clone(),
        );
        (f, reader, c.id)
    }

    #[tokio::test]
    async fn a_change_reads_as_hunks_with_their_added_and_removed_lines() {
        let (_f, reader, id) = setup().await;
        let out = reader.read(id, None).await.unwrap();
        let line = |n: u64, t: &str| json!({ "line": n, "text": t });
        assert_eq!(
            out,
            json!([
                {
                    "path": "a.txt",
                    "status": "modified",
                    "hunks": [
                        {
                            "head_start": 2, "head_end": 2, "base_start": 2, "base_end": 3,
                            "head_after": null,
                            "added": [line(2, "TWO")],
                            "removed": [line(2, "two"), line(3, "three")],
                        },
                        {
                            "head_start": 4, "head_end": 4, "base_start": null, "base_end": null,
                            "head_after": null,
                            "added": [line(4, "five")],
                            "removed": [],
                        },
                    ]
                },
                {
                    "path": "b.txt",
                    "status": "added",
                    "hunks": [{
                        "head_start": 1, "head_end": 1, "base_start": null, "base_end": null,
                        "head_after": null,
                        "added": [line(1, "new")],
                        "removed": [],
                    }]
                }
            ])
        );
    }

    #[tokio::test]
    async fn paths_limit_the_files_and_an_unknown_change_is_refused() {
        let (_f, reader, id) = setup().await;
        let out = reader.read(id, Some(&["b.txt".to_string()])).await.unwrap();
        assert_eq!(out.as_array().unwrap().len(), 1);
        assert_eq!(out[0]["path"], "b.txt");
        let err = reader.read(id + 100, None).await.unwrap_err();
        assert!(err.contains("doesn't exist"), "{err}");
    }

    #[tokio::test]
    async fn a_call_past_a_limit_fails_naming_it() {
        let (f, reader, id) = setup().await;
        // More files than the limit: pad the stored list.
        f.svc
            .db
            .transaction(move |tx| {
                for n in 0..MAX_FILES {
                    tx.execute(
                        "INSERT INTO change_file (change_id, path, status, additions, deletions, is_test)
                         VALUES (?1, ?2, 'added', 0, 0, 0)",
                        rusqlite::params![id, format!("pad/{n}.txt")],
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                }
                Ok(())
            })
            .await
            .unwrap();
        let err = reader.read(id, None).await.unwrap_err();
        assert!(err.contains("at most 200 files"), "{err}");
        // Asking for fewer files is fine.
        reader.read(id, Some(&["a.txt".to_string()])).await.unwrap();
        // Too much text: one big file.
        let root = f.svc.layout.project_dir.clone();
        std::fs::write(root.join("a.txt"), "x\n".repeat(MAX_BYTES)).unwrap();
        let sha = crate::test_fixtures::commit_all(&root, "big");
        let big = ensure_change(
            &f.svc,
            ChangeTarget::Commit {
                sha,
                stream_id: None,
            },
        )
        .await
        .unwrap();
        let err = reader.read(big.id, None).await.unwrap_err();
        assert!(err.contains("at most 2097152 bytes"), "{err}");
    }
}
