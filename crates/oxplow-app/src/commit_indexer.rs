//! Mirror git commit edges into the unified `page_ref` graph.
//!
//! For every commit we walk:
//! - The diff against parent#0 produces `(git-commit:<sha>) --
//!   touched_file --> (file:<path>)` edges.
//! - The commit message (subject + body) is run through the shared
//!   ref extractor so `(git-commit:<sha>) --task_body_mention/
//!   wikilink/finding_mention--> (target)` edges appear too.
//!
//! Idempotent. The indexer uses [`SqlitePageRefStore::replace_source`]
//! per commit, so re-walking the same commit set is a no-op. We
//! avoid an explicit cursor by checking [`source_already_indexed`]
//! before re-diffing — the diff + edge build is the only expensive
//! step, the existence probe is one indexed SELECT.
//!
//! The boot path scans the most-recent N commits; the
//! [`OxplowEvent::GitRefsChanged`] subscriber re-runs the same scan
//! on every ref movement (debounced upstream by `GitRefsWatcher`).
//!
//! Lives in `oxplow-app` rather than `oxplow-git` so we can pull in
//! the `oxplow-db` page-ref types without leaking a DB dep into the
//! pure git crate.

use std::path::Path;

use oxplow_db::page_ref_projections::{
    work_item_id, KIND_COMMIT, KIND_FILE, KIND_FINDING, KIND_WIKI, KIND_WORK_ITEM, RT_BODY_COMMIT,
    RT_BODY_FINDING, RT_BODY_TASK, RT_TOUCHED_FILE, RT_WIKILINK,
};
use oxplow_db::{PageRefEdge, SqlitePageRefStore};
use oxplow_domain::refs::extract;
use oxplow_git::log::{get_commit_detail, get_git_log, CommitDetail, GitLogOptions};

/// Default depth for the boot-time + ref-change scans. 500 commits
/// covers most active branches without a full-history walk; older
/// commits still appear in backlinks if they're referenced from a
/// newer source.
pub const DEFAULT_INDEX_DEPTH: usize = 500;

/// Pure: build the edge set for one commit. Exposed so tests can
/// exercise the projection independently of a real repo.
pub fn commit_edges(detail: &CommitDetail) -> Vec<PageRefEdge> {
    let sha = detail.sha.as_str();
    let mut out = Vec::new();
    // touched-file edges from the diff.
    for f in &detail.files {
        if f.path.is_empty() {
            continue;
        }
        out.push(PageRefEdge::new(
            KIND_COMMIT,
            sha,
            KIND_FILE,
            f.path.clone(),
            RT_TOUCHED_FILE,
        ));
    }
    // Parsed-message edges. Subject + body run through the shared
    // extractor so the same wikilink + inline-mention rules that
    // apply to wiki bodies and task descriptions also apply to
    // commit messages.
    let mut combined = String::new();
    combined.push_str(&detail.subject);
    if !detail.body.is_empty() {
        combined.push('\n');
        combined.push_str(&detail.body);
    }
    let refs = extract(&combined);
    for task_id in refs.tasks {
        out.push(PageRefEdge::new(
            KIND_COMMIT,
            sha,
            KIND_WORK_ITEM,
            work_item_id(oxplow_domain::TaskId::new(task_id)),
            RT_BODY_TASK,
        ));
    }
    for w in refs.wikis {
        out.push(PageRefEdge::new(
            KIND_COMMIT,
            sha,
            KIND_WIKI,
            w,
            RT_WIKILINK,
        ));
    }
    for f in refs.findings {
        out.push(PageRefEdge::new(
            KIND_COMMIT,
            sha,
            KIND_FINDING,
            f,
            RT_BODY_FINDING,
        ));
    }
    for c in refs.commits {
        // Don't self-link if the message happens to mention its own
        // sha; commits referencing OTHER commits is fine.
        if c == sha || sha.starts_with(&c) {
            continue;
        }
        out.push(PageRefEdge::new(
            KIND_COMMIT,
            sha,
            KIND_COMMIT,
            c,
            RT_BODY_COMMIT,
        ));
    }
    out
}

/// Walk the most-recent `limit` commits reachable from HEAD, store each
/// one (`v_commit`, `v_commit_file`) and project it into `page_ref`.
/// Skips commits already stored, so subsequent calls only index new
/// commits (and commits indexed into `page_ref` before commits were
/// stored get backfilled once). Returns the number of commits newly
/// indexed.
pub async fn index_recent(
    repo_path: &Path,
    page_refs: &SqlitePageRefStore,
    git: &oxplow_db::SqliteGitStore,
    limit: usize,
) -> usize {
    let log = {
        let repo_path = repo_path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            get_git_log(
                &repo_path,
                GitLogOptions {
                    limit: Some(limit),
                    all: false,
                },
            )
        })
        .await
        .unwrap_or_else(|_| oxplow_git::log::GitLogResult {
            commits: vec![],
            branch_heads: vec![],
            tags: vec![],
        })
    };

    let mut indexed = 0usize;
    for commit in log.commits {
        // Cheap probe: a stored commit is fully indexed. replace_source
        // is idempotent, but the diff walk is O(filecount) and we'd
        // rather not pay it on every boot for old commits.
        if git.has_commit(&commit.sha).await.unwrap_or(false) {
            continue;
        }
        let repo_path = repo_path.to_path_buf();
        let sha = commit.sha.clone();
        let detail = tokio::task::spawn_blocking(move || get_commit_detail(&repo_path, &sha))
            .await
            .ok()
            .flatten();
        let Some(detail) = detail else {
            continue;
        };
        let edges = commit_edges(&detail);
        if let Err(e) = page_refs
            .replace_source(KIND_COMMIT, &commit.sha, edges)
            .await
        {
            tracing::warn!(?e, sha = %commit.sha, "commit indexer write failed");
            continue;
        }
        if let Err(e) = git.upsert_commit(commit_row(&detail)).await {
            tracing::warn!(?e, sha = %commit.sha, "commit indexer: storing the commit failed");
            continue;
        }
        indexed += 1;
    }
    indexed
}

/// Index new commits and restate the branch list, for the primary
/// worktree. Returns the number of commits newly indexed.
pub async fn refresh(svc: &crate::Services) -> usize {
    use oxplow_domain::stores::StreamStore as _;
    let repo_path = svc.layout.project_dir.clone();
    let n = index_recent(
        &repo_path,
        &svc.page_ref_store,
        &svc.git_store,
        DEFAULT_INDEX_DEPTH,
    )
    .await;
    let streams: Vec<(i64, String)> = svc
        .stream_store
        .list()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|s| (s.id.value(), s.branch))
        .collect();
    refresh_branches(&repo_path, &streams, &svc.git_store).await;
    n
}

/// The stored form of a commit.
fn commit_row(d: &CommitDetail) -> oxplow_db::GitCommitRow {
    oxplow_db::GitCommitRow {
        sha: d.sha.clone(),
        author: d.author.clone(),
        email: d.email.clone(),
        committed_secs: d.timestamp_secs,
        subject: d.subject.clone(),
        body: d.body.clone(),
        parents: d.parents.clone(),
        files: d
            .files
            .iter()
            .filter(|f| !f.path.is_empty())
            .map(|f| oxplow_db::GitCommitFileRow {
                path: f.path.clone(),
                status: f.status.clone(),
                additions: f.additions as i64,
                deletions: f.deletions as i64,
            })
            .collect(),
    }
}

/// Restate `v_branch`: every local and remote-tracking branch with its
/// head, and which stream (`(stream_id, branch)` pairs) has a local one
/// checked out.
pub async fn refresh_branches(
    repo_path: &Path,
    streams: &[(i64, String)],
    git: &oxplow_db::SqliteGitStore,
) {
    let path = repo_path.to_path_buf();
    let branches = tokio::task::spawn_blocking(move || oxplow_git::list_branches(&path))
        .await
        .unwrap_or_default();
    let rows = branch_rows(&branches, streams);
    if let Err(e) = git.replace_branches(rows).await {
        tracing::warn!(?e, "branch refresh failed");
    }
}

/// Pure: branch refs + stream checkouts → stored rows.
fn branch_rows(
    branches: &[oxplow_git::BranchRef],
    streams: &[(i64, String)],
) -> Vec<oxplow_db::GitBranchRow> {
    use oxplow_git::BranchRefKind;
    branches
        .iter()
        .map(|b| {
            let local = b.kind == BranchRefKind::Local;
            oxplow_db::GitBranchRow {
                name: b.name.clone(),
                kind: if local { "local" } else { "remote" }.into(),
                remote: b.remote.clone(),
                head_sha: b.head.clone(),
                stream_id: if local {
                    streams
                        .iter()
                        .find(|(_, br)| *br == b.name)
                        .map(|(id, _)| *id)
                } else {
                    None
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_git::log::{CommitDetail, CommitDetailFile};

    fn commit(sha: &str, subject: &str, body: &str, paths: &[&str]) -> CommitDetail {
        CommitDetail {
            sha: sha.into(),
            short_sha: sha[..7.min(sha.len())].into(),
            author: "a".into(),
            email: "a@b".into(),
            timestamp_secs: 0,
            subject: subject.into(),
            body: body.into(),
            parents: vec![],
            files: paths
                .iter()
                .map(|p| CommitDetailFile {
                    path: (*p).into(),
                    additions: 0,
                    deletions: 0,
                    status: "modified".into(),
                })
                .collect(),
        }
    }

    #[test]
    fn touched_file_edges_from_diff() {
        let c = commit(
            "abc1234567890",
            "fix bug",
            "",
            &["src/app.rs", "src/lib.rs"],
        );
        let edges = commit_edges(&c);
        let files: std::collections::BTreeSet<_> = edges
            .iter()
            .filter(|e| e.ref_type == "touched_file")
            .map(|e| e.target_id.as_str())
            .collect();
        assert!(files.contains("src/app.rs"));
        assert!(files.contains("src/lib.rs"));
    }

    #[test]
    fn message_body_picks_up_task_and_wiki_refs() {
        let c = commit(
            "abc1234567890",
            "Resolve tsk42 and clarify [[architecture]]",
            "see finding:fnd-7 for details",
            &[],
        );
        let edges = commit_edges(&c);
        let targets: Vec<_> = edges
            .iter()
            .map(|e| (e.target_kind.as_str(), e.target_id.as_str()))
            .collect();
        assert!(targets.contains(&("work_item", "oxplow:tsk42")));
        assert!(targets.contains(&("wiki", "architecture")));
        assert!(targets.contains(&("finding", "fnd-7")));
    }

    #[test]
    fn self_referential_sha_is_dropped() {
        // If a commit message accidentally contains its own short
        // sha (e.g. "revert abc1234"), don't emit a self-loop.
        let c = commit("abc1234567890def", "revert abc1234", "", &[]);
        let edges = commit_edges(&c);
        assert!(
            !edges
                .iter()
                .any(|e| e.ref_type == "commit_mention" && e.target_id == "abc1234"),
            "self-ref must be dropped"
        );
    }

    #[tokio::test]
    async fn index_recent_against_real_repo() {
        let dir = tempfile::tempdir().unwrap();
        // Build a tiny real git repo with one commit referencing wi-X.
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "a@b"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "a"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn main() {}").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-q", "-m", "fix tsk42 and touch [[architecture]]"])
            .current_dir(dir.path())
            .status()
            .unwrap();

        let db = oxplow_db::Database::in_memory();
        let page_refs = SqlitePageRefStore::new(db.clone());
        let git = oxplow_db::SqliteGitStore::new(db.clone());
        let n = index_recent(dir.path(), &page_refs, &git, 50).await;
        assert_eq!(n, 1, "should index the one commit");

        // The commit, its file and its task mention read through v_*.
        let sl = oxplow_db::SemanticLayer::new(db.clone());
        let q = |sql: &'static str| {
            let sl = sl.clone();
            async move {
                serde_json::to_value(sl.query_sql(sql, vec![], None).await.unwrap().rows).unwrap()
            }
        };
        assert_eq!(
            q("SELECT subject, author, parent_count FROM v_commit").await,
            serde_json::json!([["fix tsk42 and touch [[architecture]]", "a", 0]])
        );
        assert_eq!(
            q("SELECT path, status FROM v_commit_file").await,
            serde_json::json!([["a.rs", "added"]])
        );
        assert_eq!(
            q("SELECT task_id FROM v_commit_task").await,
            serde_json::json!([[42]])
        );

        // tsk42 has the commit as a backlink.
        let inbound = page_refs
            .list_backlinks("work_item", "oxplow:tsk42", None)
            .await
            .unwrap();
        assert!(inbound.iter().any(|e| e.source_kind == "commit"));
        // file backlink covers a.rs.
        let file_inbound = page_refs
            .list_backlinks("file", "a.rs", None)
            .await
            .unwrap();
        assert!(file_inbound.iter().any(|e| e.source_kind == "commit"));

        // Re-index — nothing new.
        let n2 = index_recent(dir.path(), &page_refs, &git, 50).await;
        assert_eq!(n2, 0, "second pass must skip already-indexed commits");
    }

    #[test]
    fn local_branches_map_to_the_stream_that_checks_them_out() {
        use oxplow_git::{BranchRef, BranchRefKind};
        let b = |kind, name: &str, remote: Option<&str>| BranchRef {
            kind,
            name: name.into(),
            ref_: String::new(),
            remote: remote.map(str::to_string),
            head: Some("abc".into()),
        };
        let rows = branch_rows(
            &[
                b(BranchRefKind::Local, "main", None),
                b(BranchRefKind::Local, "feature", None),
                b(BranchRefKind::Remote, "main", Some("origin")),
            ],
            &[(1, "main".into()), (2, "feature".into())],
        );
        let got: Vec<(&str, &str, Option<i64>)> = rows
            .iter()
            .map(|r| (r.name.as_str(), r.kind.as_str(), r.stream_id))
            .collect();
        assert_eq!(
            got,
            vec![
                ("main", "local", Some(1)),
                ("feature", "local", Some(2)),
                ("main", "remote", None)
            ]
        );
    }
}
