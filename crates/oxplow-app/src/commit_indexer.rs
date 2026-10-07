//! Mirror git commit edges into the unified `page_ref` graph.
//!
//! For every commit we walk:
//! - The diff against parent#0 produces `(git-commit:<sha>) --
//!   touched_file --> (file:<path>)` edges.
//! - The commit message (subject + body) is run through the shared
//!   ref extractor so `(git-commit:<sha>) --work_item_mention/
//!   wikilink/finding_mention--> (target)` edges appear too.
//!
//! Idempotent. The indexer uses [`SqlitePageRefStore::replace_source`]
//! per commit, so re-walking the same commit set is a no-op. We
//! avoid an explicit cursor by checking [`source_already_indexed`]
//! before re-diffing — the diff + edge build is the only expensive
//! step, the existence probe is one indexed SELECT.
//!
//! The boot path scans the commits reachable from every stream's head
//! down to the ones already stored ([`IndexDepth`]) (so a worktree stream's own commits are in `v_commit`);
//! the [`OxplowEvent::VcsRefsChanged`] subscriber re-runs the same scan
//! on every ref movement (debounced upstream by `GitRefsWatcher`). The
//! same pass restates `v_branch` and `v_tag`. Everything reads through
//! the VCS capability (`.context/vcs.md`).

use std::path::Path;

use oxplow_db::page_ref_projections::{
    KIND_COMMIT, KIND_FILE, KIND_FINDING, KIND_WIKI, KIND_WORK_ITEM, RT_BODY_COMMIT,
    RT_BODY_FINDING, RT_BODY_WORK_ITEM, RT_TOUCHED_FILE, RT_WIKILINK,
};
use oxplow_db::{PageRefEdge, SqlitePageRefStore};
use oxplow_domain::refs::extract;
use oxplow_domain::refs::kind::KindRegistry;
use oxplow_domain::vcs::{Branch, LogQuery, RevisionDetail, Vcs};

/// How far a pass walks from a head. It reads `window` revisions, and
/// while the oldest it read was new (new history is longer than the
/// window — a big pull), doubles it and reads on, up to `max`. So new
/// history up to `max` deep is indexed whole, with no gap below a
/// window. `max` is also the horizon: a first boot on a long history
/// indexes the newest `max`, and older history stays out (as does the
/// rest of a single ref move of more than `max` new revisions).
#[derive(Debug, Clone, Copy)]
pub struct IndexDepth {
    pub window: usize,
    pub max: usize,
}

impl IndexDepth {
    /// The boot and ref-change scans: 500 at a time, 5000 at most.
    pub const DEFAULT: IndexDepth = IndexDepth {
        window: 500,
        max: 5000,
    };
}

/// Pure: build the edge set for one commit. Exposed so tests can
/// exercise the projection independently of a real repo.
pub fn commit_edges(kinds: &KindRegistry, detail: &RevisionDetail) -> Vec<PageRefEdge> {
    let sha = detail.info.id.as_str();
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
    combined.push_str(&detail.info.subject);
    if !detail.body.is_empty() {
        combined.push('\n');
        combined.push_str(&detail.body);
    }
    let refs = extract(kinds, &combined);
    for item in refs.work_items {
        out.push(PageRefEdge::new(
            KIND_COMMIT,
            sha,
            KIND_WORK_ITEM,
            item,
            RT_BODY_WORK_ITEM,
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

/// Walk the revisions reachable from workspace `ws`'s head, newest
/// first and as deep as `depth` says, store each new one (`v_commit`,
/// `v_commit_file`) and project it into `page_ref`. Returns the ones
/// newly indexed, with when each was committed.
pub async fn index_recent(
    kinds: &KindRegistry,
    vcs: &dyn Vcs,
    ws: &Path,
    page_refs: &SqlitePageRefStore,
    git: &oxplow_db::SqliteGitStore,
    depth: IndexDepth,
) -> Vec<(String, oxplow_domain::Timestamp)> {
    let mut this_pass: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut indexed = Vec::new();
    let mut limit = depth.window.min(depth.max);
    loop {
        let log = vcs
            .log(
                ws,
                LogQuery {
                    limit: Some(limit as u32),
                    all: false,
                },
            )
            .await
            .unwrap_or_default();
        let exhausted = log.len() < limit;
        let oldest = log.last().map(|i| i.id.clone());
        for info in log {
            if this_pass.contains(&info.id) {
                continue;
            }
            // Cheap probe: a stored commit is fully indexed. replace_source
            // is idempotent, but the diff walk is O(filecount) and we'd
            // rather not pay it on every boot for old commits.
            if git.has_commit(&info.id).await.unwrap_or(false) {
                continue;
            }
            if let Some(committed) = index_one(kinds, vcs, ws, page_refs, git, &info.id).await {
                this_pass.insert(info.id.clone());
                indexed.push((info.id, committed));
            }
        }
        // The walk goes on below the window only while its oldest
        // revision was new: new history is longer than the window.
        let oldest_was_new = oldest.is_some_and(|o| this_pass.contains(&o));
        if !oldest_was_new || exhausted || limit >= depth.max {
            return indexed;
        }
        limit = (limit * 2).min(depth.max);
    }
}

/// Store and project one revision; when it was committed, if it was
/// stored.
async fn index_one(
    kinds: &KindRegistry,
    vcs: &dyn Vcs,
    ws: &Path,
    page_refs: &SqlitePageRefStore,
    git: &oxplow_db::SqliteGitStore,
    sha: &str,
) -> Option<oxplow_domain::Timestamp> {
    let Ok(Some(detail)) = vcs.revision(ws, sha).await else {
        return None;
    };
    let edges = commit_edges(kinds, &detail);
    if let Err(e) = page_refs.replace_source(KIND_COMMIT, sha, edges).await {
        tracing::warn!(?e, %sha, "commit indexer write failed");
        return None;
    }
    if let Err(e) = git.upsert_commit(commit_row(&detail)).await {
        tracing::warn!(?e, %sha, "commit indexer: storing the commit failed");
        return None;
    }
    Some(oxplow_domain::Timestamp::from_unix_ms(
        detail.info.time * 1000,
    ))
}

/// Index new revisions from every stream's head and restate the branch
/// and tag lists. Returns the number of revisions newly indexed.
pub async fn refresh(svc: &crate::Services) -> usize {
    use oxplow_domain::stores::StreamStore as _;
    let streams = svc.stream_store.list().await.unwrap_or_default();
    // Each workspace once, with the first stream it's for.
    let mut workspaces: Vec<std::path::PathBuf> = Vec::new();
    let mut stream_of: Vec<Option<oxplow_domain::StreamId>> = Vec::new();
    for s in &streams {
        let ws = svc.worktrees.resolve(Some(&s.id.to_string())).await;
        if !workspaces.contains(&ws) {
            workspaces.push(ws);
            stream_of.push(Some(s.id));
        }
    }
    let primary = svc.worktrees.project_dir().to_path_buf();
    if !workspaces.contains(&primary) {
        workspaces.insert(0, primary.clone());
        stream_of.insert(0, None);
    }
    let vocabulary = svc.vocabulary.current();
    let mut n = 0;
    for (ws, stream) in workspaces.iter().zip(&stream_of) {
        let indexed = index_recent(
            &vocabulary.kinds,
            &*svc.vcs,
            ws,
            &svc.page_ref_store,
            &svc.git_store,
            IndexDepth::DEFAULT,
        )
        .await;
        n += indexed.len();
        // Each new commit, to the tasks whose effort's work it holds
        // (tsk1035).
        for (sha, committed) in &indexed {
            if let Err(e) = crate::commit_links::link_commit(svc, ws, sha, *committed).await {
                tracing::warn!(?e, %sha, "linking a commit to its effort failed");
            }
            if let Some(stream) = stream {
                log_indexed(svc, *stream, sha, *committed).await;
            }
        }
    }
    // Which stream has a branch checked out is its workspace's head —
    // not the stream row, which the branch reconciler updates later.
    let mut checkouts: Vec<(i64, String)> = Vec::new();
    for s in &streams {
        let ws = svc.worktrees.resolve(Some(&s.id.to_string())).await;
        if let Ok(oxplow_domain::vcs::Head {
            branch: Some(branch),
            ..
        }) = svc.vcs.head(&ws).await
        {
            checkouts.push((s.id.value(), branch));
        }
    }
    refresh_refs(&*svc.vcs, &primary, &checkouts, &svc.git_store).await;
    n
}

/// `vcs.commit.indexed` for a commit found from `stream`'s workspace:
/// once per commit, however many streams reach it.
async fn log_indexed(
    svc: &crate::Services,
    stream: oxplow_domain::StreamId,
    sha: &str,
    committed: oxplow_domain::Timestamp,
) {
    use oxplow_domain::events::schema::{VcsCommitIndexed, VcsCommitIndexedV1};
    let env = oxplow_domain::Envelope::typed::<VcsCommitIndexed>(
        oxplow_domain::refs::build::system_source("commit_indexer"),
        &VcsCommitIndexedV1 {
            commit: format!("commit:{sha}"),
            stream: oxplow_domain::refs::build::stream_ref(stream),
            committed_at: committed.to_text(),
        },
    )
    .with_anchors(oxplow_domain::Anchors {
        stream_id: Some(stream),
        ..Default::default()
    })
    .with_subject([format!("commit:{sha}")])
    .with_dedupe_key(format!("vcs.commit.indexed:{sha}"));
    match svc.event_log_store.append(env).await {
        Ok(_) | Err(oxplow_domain::DomainError::Constraint(_)) => {}
        Err(e) => tracing::warn!(?e, %sha, "logging an indexed commit failed"),
    }
}

/// The stored form of a revision.
fn commit_row(d: &RevisionDetail) -> oxplow_db::GitCommitRow {
    oxplow_db::GitCommitRow {
        sha: d.info.id.clone(),
        author: d.info.author.clone(),
        email: d.info.email.clone(),
        committed_secs: d.info.time,
        subject: d.info.subject.clone(),
        body: d.body.clone(),
        parents: d.info.parents.clone(),
        files: d
            .files
            .iter()
            .filter(|f| !f.path.is_empty())
            .map(|f| oxplow_db::GitCommitFileRow {
                path: f.path.clone(),
                status: f.status.as_str().to_string(),
                additions: f.additions as i64,
                deletions: f.deletions as i64,
            })
            .collect(),
    }
}

/// Restate `v_branch` — every local and remote-tracking branch with its
/// head, which stream (`(stream_id, branch)` pairs) has a local one
/// checked out, and the default — and `v_tag`.
pub async fn refresh_refs(
    vcs: &dyn Vcs,
    ws: &Path,
    streams: &[(i64, String)],
    git: &oxplow_db::SqliteGitStore,
) {
    let branches = vcs.branches(ws).await.unwrap_or_default();
    if let Err(e) = git.replace_branches(branch_rows(&branches, streams)).await {
        tracing::warn!(?e, "branch refresh failed");
    }
    let tags = vcs
        .tags(ws)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|t| oxplow_db::GitTagRow {
            name: t.name,
            sha: t.revision,
        })
        .collect();
    if let Err(e) = git.replace_tags(tags).await {
        tracing::warn!(?e, "tag refresh failed");
    }
}

/// Pure: branch refs + stream checkouts → stored rows.
fn branch_rows(branches: &[Branch], streams: &[(i64, String)]) -> Vec<oxplow_db::GitBranchRow> {
    branches
        .iter()
        .map(|b| {
            let local = b.remote.is_none();
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
                is_default: b.is_default,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The core kinds with oxplow's tasks as the work list (`tsk<n>`).
    fn tasks_kinds() -> oxplow_domain::refs::kind::KindRegistry {
        oxplow_domain::refs::kind::core_kinds()
            .with_work_item_ids("oxplow", r"tsk\d+")
            .unwrap()
    }
    use oxplow_domain::vcs::{FileStatus, RevisionFile, RevisionInfo};

    fn commit(sha: &str, subject: &str, body: &str, paths: &[&str]) -> RevisionDetail {
        RevisionDetail {
            info: RevisionInfo {
                id: sha.into(),
                short_id: sha[..7.min(sha.len())].into(),
                author: "a".into(),
                email: "a@b".into(),
                time: 0,
                subject: subject.into(),
                parents: vec![],
            },
            body: body.into(),
            files: paths
                .iter()
                .map(|p| RevisionFile {
                    path: (*p).into(),
                    additions: 0,
                    deletions: 0,
                    status: FileStatus::Modified,
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
        let edges = commit_edges(&tasks_kinds(), &c);
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
        let edges = commit_edges(&tasks_kinds(), &c);
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
        let edges = commit_edges(&tasks_kinds(), &c);
        assert!(
            !edges
                .iter()
                .any(|e| e.ref_type == "commit_mention" && e.target_id == "abc1234"),
            "self-ref must be dropped"
        );
    }

    /// P5.B5 (tsk524): the indexer walks every stream's head, so a commit
    /// made only on a worktree stream's branch reaches `v_commit` (with its
    /// parents), `v_branch` marks the default branch, and tags are in
    /// `v_tag`. A stream's history is a recursive read from its head.
    #[tokio::test]
    async fn every_stream_head_is_indexed_and_history_reads_from_its_head() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        let base = crate::test_fixtures::commit_all(&root, "base");
        let main = svc.vcs.head(&root).await.unwrap().branch.unwrap();
        let git = |dir: &std::path::Path, args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .unwrap();
            assert!(out.status.success(), "{out:?}");
        };
        git(&root, &["tag", "v1"]);
        let side = svc
            .streams
            .create_worktree("b5-side", "Side", "b5-side", main.clone())
            .await
            .unwrap();
        let side_dir = std::path::PathBuf::from(&side.worktree_path);
        let _cleanup = scopeguard(side_dir.clone());
        std::fs::write(side_dir.join("side.txt"), "s\n").unwrap();
        git(&side_dir, &["add", "-A"]);
        git(
            &side_dir,
            &["commit", "-q", "-m", "only on the side branch"],
        );

        refresh(svc).await;
        let q = |sql: &'static str| async move {
            serde_json::to_value(svc.sql.query_sql(sql, vec![], None).await.unwrap().rows).unwrap()
        };
        assert_eq!(
            q("SELECT subject, parents FROM v_commit WHERE subject = 'only on the side branch'")
                .await,
            serde_json::json!([["only on the side branch", format!("[\"{base}\"]")]])
        );
        assert_eq!(
            q("SELECT name, is_default FROM v_branch WHERE kind = 'local' ORDER BY name").await,
            serde_json::json!([["b5-side", 0], [main, 1]])
        );
        assert_eq!(
            q("SELECT name, sha FROM v_tag").await,
            serde_json::json!([["v1", base.clone()]])
        );
        // The side stream's history, from its branch head.
        assert_eq!(
            q("WITH RECURSIVE reach(sha) AS (
                   SELECT head_sha FROM v_branch WHERE name = 'b5-side' AND kind = 'local'
                   UNION
                   SELECT p.value FROM reach JOIN v_commit c ON c.sha = reach.sha, json_each(c.parents) p
                 )
                 SELECT c.subject FROM v_commit c JOIN reach USING (sha) ORDER BY c.subject")
            .await,
            serde_json::json!([["base"], ["init"], ["only on the side branch"]])
        );

        // tsk573: a checkout the stream's row doesn't know yet (the branch
        // reconciler hasn't run): the branch a workspace has checked out
        // is its stream's, read from the workspace's head.
        git(&side_dir, &["checkout", "-q", "-b", "b5-moved"]);
        refresh(svc).await;
        let side_id = side.id.value();
        assert_eq!(
            q(
                "SELECT name, stream_id FROM v_branch WHERE kind = 'local' AND name LIKE 'b5-%'
               ORDER BY name"
            )
            .await,
            serde_json::json!([["b5-moved", side_id], ["b5-side", null]])
        );
    }

    /// Removes a sibling worktree directory when the test ends.
    fn scopeguard(dir: std::path::PathBuf) -> impl Drop {
        struct G(std::path::PathBuf);
        impl Drop for G {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        G(dir)
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
        let n = index_recent(
            &tasks_kinds(),
            &crate::vcs::GitProvider,
            dir.path(),
            &page_refs,
            &git,
            IndexDepth {
                window: 50,
                max: 50,
            },
        )
        .await
        .len();
        assert_eq!(n, 1, "should index the one commit");

        // The commit, its file and its task mention read through v_*.
        let sl = crate::sql_gateway::SqlGateway::new(db.clone());
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
        let n2 = index_recent(
            &tasks_kinds(),
            &crate::vcs::GitProvider,
            dir.path(),
            &page_refs,
            &git,
            IndexDepth {
                window: 50,
                max: 50,
            },
        )
        .await
        .len();
        assert_eq!(n2, 0, "second pass must skip already-indexed commits");
    }

    /// tsk568: more new commits than a window are indexed whole — the
    /// walk goes on below the window while its oldest is new — and a
    /// pass stops at its cap.
    #[tokio::test]
    async fn a_pull_larger_than_the_window_leaves_no_gap() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let commit_n = |from: usize, to: usize| {
            for i in from..to {
                std::fs::write(dir.path().join("n.txt"), i.to_string()).unwrap();
                crate::test_fixtures::commit_all(dir.path(), "c");
            }
        };
        commit_n(0, 7);
        let db = oxplow_db::Database::in_memory();
        let page_refs = SqlitePageRefStore::new(db.clone());
        let git = oxplow_db::SqliteGitStore::new(db.clone());
        let pass = |max: usize| {
            let (page_refs, git) = (page_refs.clone(), git.clone());
            let ws = dir.path().to_path_buf();
            async move {
                index_recent(
                    &tasks_kinds(),
                    &crate::vcs::GitProvider,
                    &ws,
                    &page_refs,
                    &git,
                    IndexDepth { window: 3, max },
                )
                .await
                .len()
            }
        };
        let stored = || {
            let db = db.clone();
            async move {
                db.read(|c| {
                    c.query_row("SELECT count(*) FROM git_commit", [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
                })
                .await
                .unwrap()
            }
        };
        // A first pass walks past its window to the whole history.
        let total = 1 + 7; // init_git_repo's commit and ours
        assert_eq!(pass(100).await, total);
        assert_eq!(stored().await, total as i64);
        // A pull of 5 on top of a window of 3: all 5.
        commit_n(7, 12);
        assert_eq!(pass(100).await, 5);
        assert_eq!(stored().await, total as i64 + 5);
        assert_eq!(pass(100).await, 0);
        // The cap bounds a pass: 4 more new ones, a cap of 2.
        commit_n(12, 16);
        assert_eq!(pass(2).await, 2);
    }

    #[test]
    fn local_branches_map_to_the_stream_that_checks_them_out() {
        let b = |name: &str, remote: Option<&str>| Branch {
            name: name.into(),
            remote: remote.map(str::to_string),
            head: Some("abc".into()),
            is_default: name == "main" && remote.is_none(),
        };
        let rows = branch_rows(
            &[
                b("main", None),
                b("feature", None),
                b("main", Some("origin")),
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
