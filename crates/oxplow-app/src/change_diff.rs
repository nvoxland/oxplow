//! What a script's `change_diff(change_id, paths)` answers (`tree.read`;
//! `oxplow_script::change_diff`): an analysed change's changed files, each
//! with its hunks and the lines they added and removed. The files are the
//! ones the analysis stored (`v_change_file`, so a request's own files
//! only); each side's text is read through [`Trees`] as the analyzer reads
//! it, and the ranges are [`crate::churn::diff_hunks`]'s.
//!
//! One call returns at most [`MAX_FILES`] files and [`MAX_BYTES`] of line
//! text (the `added` and `removed` lines it returns, not the files' sizes):
//! `{ files, rest, too_large }`. The files past either limit are `rest`,
//! in order, for the script to ask again with `paths = rest`; a file whose
//! own lines are past [`MAX_BYTES`] is `too_large` and never returned. A
//! file is never truncated.

use std::path::Path;
use std::sync::Arc;

use oxplow_db::{Database, SqliteChangeStore};
use oxplow_domain::vcs::Revision;
use serde_json::{json, Value};

use crate::trees::Trees;
use crate::worktrees::WorktreeRouter;

/// The most files one call returns.
pub const MAX_FILES: usize = 200;
/// The most line text (in bytes, the `added` and `removed` lines of every
/// file) one call returns.
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
        let root = crate::change_analysis::stream_root(&self.worktrees, row.stream_id).await;
        let mut bytes = 0usize;
        let mut out = Vec::new();
        let mut rest = Vec::new();
        let mut too_large = Vec::new();
        for (path, status) in files {
            // Once one file is left for the next call, so is every later
            // one: a call returns the files in order.
            if !rest.is_empty() || out.len() == MAX_FILES {
                rest.push(path);
                continue;
            }
            let base = self.side(&root, row.base_revision.as_ref(), &path).await?;
            let head = match &row.head_revision {
                Some(rev) => self.side(&root, Some(rev), &path).await?,
                None => String::new(),
            };
            let (hunks, size) = hunks(&base, &head);
            if size > MAX_BYTES {
                too_large.push(path);
            } else if bytes + size > MAX_BYTES {
                rest.push(path);
            } else {
                bytes += size;
                out.push(json!({ "path": path, "status": status, "hunks": hunks }));
            }
        }
        Ok(json!({ "files": out, "rest": rest, "too_large": too_large }))
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

/// `base` → `head`'s hunks with the lines each added and removed, and
/// the bytes of those lines' text: what a call's limit counts.
fn hunks(base: &str, head: &str) -> (Vec<Value>, usize) {
    let base_lines: Vec<&str> = base.lines().collect();
    let head_lines: Vec<&str> = head.lines().collect();
    let mut size = 0usize;
    let mut lines = |all: &[&str], range: Option<(u32, u32)>| -> Vec<Value> {
        let Some((start, end)) = range else {
            return Vec::new();
        };
        (start..=end)
            .filter_map(|n| {
                all.get(n as usize - 1).map(|text| {
                    size += text.len();
                    json!({ "line": n, "text": text })
                })
            })
            .collect()
    };
    let hunks = crate::churn::diff_hunks(base, head)
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
        .collect();
    (hunks, size)
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
        if let Some(files) = out["files"].as_array_mut() {
            files.retain(|f| {
                f["path"]
                    .as_str()
                    .is_some_and(|path| self.run.permits_path(path))
            });
        }
        for named in ["rest", "too_large"] {
            if let Some(paths) = out[named].as_array_mut() {
                paths.retain(|p| p.as_str().is_some_and(|path| self.run.permits_path(path)));
            }
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
        assert_eq!(out["rest"], json!([]));
        assert_eq!(out["too_large"], json!([]));
        assert_eq!(
            out["files"],
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
        assert_eq!(out["files"].as_array().unwrap().len(), 1);
        assert_eq!(out["files"][0]["path"], "b.txt");
        let err = reader.read(id + 100, None).await.unwrap_err();
        assert!(err.contains("doesn't exist"), "{err}");
    }

    /// A call returns the files that fit and names the rest, so a script
    /// asks again with `paths = rest`: past the file count, or past the
    /// text its hunk lines add up to. A file whose own lines are past the
    /// limit is `too_large`, never read.
    #[tokio::test]
    async fn a_call_past_a_limit_returns_what_fits_and_names_the_rest() {
        let (f, reader, id) = setup().await;
        // More files than the limit: pad the stored list.
        f.svc
            .db
            .transaction(move |tx| {
                for n in 0..MAX_FILES {
                    tx.execute(
                        "INSERT INTO change_file (change_id, path, status, additions, deletions, is_test)
                         VALUES (?1, ?2, 'added', 0, 0, 0)",
                        rusqlite::params![id, format!("pad/{n:03}.txt")],
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                }
                Ok(())
            })
            .await
            .unwrap();
        let out = reader.read(id, None).await.unwrap();
        assert_eq!(out["files"].as_array().unwrap().len(), MAX_FILES);
        assert_eq!(out["files"][0]["path"], "a.txt");
        let rest: Vec<String> = serde_json::from_value(out["rest"].clone()).unwrap();
        assert_eq!(
            rest,
            vec!["pad/198.txt".to_string(), "pad/199.txt".to_string()]
        );
        let again = reader.read(id, Some(&rest)).await.unwrap();
        assert_eq!(again["files"].as_array().unwrap().len(), 2);
        assert_eq!(again["rest"], json!([]));
        assert_eq!(again["too_large"], json!([]));

        // A large file whose change is one line counts that line, not its
        // two sides.
        let root = f.svc.layout.project_dir.clone();
        let lines = MAX_BYTES / 1000 + 1;
        let body = format!("{}\n", "x".repeat(999)).repeat(lines);
        std::fs::write(root.join("a.txt"), &body).unwrap();
        crate::test_fixtures::commit_all(&root, "big base");
        std::fs::write(root.join("a.txt"), format!("{body}y\n")).unwrap();
        let one_line = change_at(&f, &root, "one line").await;
        let out = reader.read(one_line, None).await.unwrap();
        assert_eq!(
            out["files"][0]["hunks"][0]["added"],
            json!([{ "line": lines + 1, "text": "y" }])
        );
        assert_eq!(out["rest"], json!([]));

        // Lines past the limit together: the first file fits, the second is
        // the rest; one past it alone is too large.
        let half = "h".repeat(MAX_BYTES / 2 + 1);
        std::fs::write(root.join("c.txt"), format!("{half}\n")).unwrap();
        std::fs::write(root.join("d.txt"), format!("{half}\n")).unwrap();
        std::fs::write(
            root.join("e.txt"),
            format!("{}\n", "e".repeat(MAX_BYTES + 1)),
        )
        .unwrap();
        let wide = change_at(&f, &root, "wide").await;
        let out = reader.read(wide, None).await.unwrap();
        let paths: Vec<&str> = out["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap())
            .collect();
        assert_eq!(paths, vec!["c.txt"]);
        assert_eq!(out["rest"], json!(["d.txt", "e.txt"]));
        assert_eq!(out["too_large"], json!([]));
        let rest: Vec<String> = serde_json::from_value(out["rest"].clone()).unwrap();
        let again = reader.read(wide, Some(&rest)).await.unwrap();
        assert_eq!(again["files"][0]["path"], "d.txt");
        assert_eq!(again["rest"], json!([]));
        assert_eq!(again["too_large"], json!(["e.txt"]));
    }

    /// The change of the commit of what's in `root` now.
    async fn change_at(f: &crate::test_fixtures::ThreadFixture, root: &Path, message: &str) -> i64 {
        let sha = crate::test_fixtures::commit_all(root, message);
        ensure_change(
            &f.svc,
            ChangeTarget::Commit {
                sha,
                stream_id: None,
            },
        )
        .await
        .unwrap()
        .id
    }
}
