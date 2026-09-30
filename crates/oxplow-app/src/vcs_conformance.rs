//! The VCS capability's contract (P5, `.context/target-architecture.md`
//! §6.2, `.context/vcs.md`), as checks any provider must pass. Each
//! check takes a `&dyn Vcs` and a fresh workspace, so a second provider
//! runs the same list; the tests at the bottom run it against git. Test
//! code: the module compiles under `cfg(test)` only.
//!
//! 1. **Round-trip** — what a commit captured reads back from the store,
//!    whatever the working tree holds now.
//!    The clean baseline vouches for an untouched committed file by its
//!    stat, never for an edited one.
//! 2. **Diffs** — two revisions diff to what changed between them,
//!    deletions included, sorted by path.
//! 3. **Status** — added, modified, deleted and untracked paths show;
//!    a commit leaves the workspace clean.
//! 5. **Head and log** — the head resolves to the last commit, which the
//!    log lists first.
//! 6. **Blame** — each line names the revision that last changed it.
//!
//! (7, the snapshot↔revision mapping, joins with `Trees` in P5.B3.)

use std::path::Path;

use oxplow_domain::vcs::{CommitRequest, FileStatus, LogQuery, Vcs};
use oxplow_domain::ChangeStatus;

fn write(ws: &Path, path: &str, body: &str) {
    std::fs::write(ws.join(path), body).unwrap();
}

async fn commit(p: &dyn Vcs, ws: &Path, message: &str) -> String {
    p.commit(
        ws,
        CommitRequest {
            message: message.into(),
            include_untracked: true,
        },
    )
    .await
    .unwrap()
}

/// 1. A commit reads back what it captured, whatever the working tree
///    holds now.
pub async fn a_commit_reads_back_what_it_captured(p: &dyn Vcs, ws: &Path) {
    write(ws, "a.txt", "one\n");
    let first = commit(p, ws, "first").await;
    write(ws, "a.txt", "two\n");
    let files = p.files_at(ws, &first).await.unwrap();
    let id = files.get("a.txt").expect("a.txt is in the commit");
    let objects = p.object_store(ws);
    assert_eq!(objects.read(id).expect("the object"), b"one\n");
    assert_eq!(*id, objects.id_of(b"one\n"), "ids are content addresses");
    assert_ne!(objects.id_of(b"two\n"), *id);
    assert_eq!(objects.read(&objects.id_of(b"never stored")), None);
}

/// 2. Two revisions diff to what changed, deletions included.
pub async fn two_revisions_diff_to_what_changed(p: &dyn Vcs, ws: &Path) {
    write(ws, "kept.txt", "same\n");
    write(ws, "edited.txt", "v1\n");
    write(ws, "gone.txt", "bye\n");
    let first = commit(p, ws, "first").await;
    write(ws, "edited.txt", "v2\n");
    write(ws, "new.txt", "hi\n");
    std::fs::remove_file(ws.join("gone.txt")).unwrap();
    let second = commit(p, ws, "second").await;
    let changes = p.diff(ws, &first, &second).await.unwrap();
    assert_eq!(
        changes
            .iter()
            .map(|c| (c.path.as_str(), c.status))
            .collect::<Vec<_>>(),
        vec![
            ("edited.txt", ChangeStatus::Modified),
            ("gone.txt", ChangeStatus::Deleted),
            ("new.txt", ChangeStatus::Added),
        ]
    );
    assert!(p.diff(ws, &second, &second).await.unwrap().is_empty());
}

/// 1b. The clean baseline vouches for a committed, untouched file by its
///     stat — and for nothing edited since.
pub async fn the_clean_baseline_vouches_only_for_untouched_files(p: &dyn Vcs, ws: &Path) {
    write(ws, "clean.txt", "c\n");
    write(ws, "edited.txt", "e1\n");
    commit(p, ws, "base").await;
    // Stat granularity: an edit in the same instant as the index write
    // must still read as changed, so step past it.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    write(ws, "edited.txt", "e2 longer\n");
    let stat = |path: &str| {
        let md = std::fs::metadata(ws.join(path)).unwrap();
        let t = md
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        (md.len(), (t.as_secs() as i64, t.subsec_nanos()))
    };
    let baseline = p.clean_baseline(ws);
    let head = p.head(ws).await.unwrap().revision.unwrap();
    let files = p.files_at(ws, &head).await.unwrap();
    let (size, mtime) = stat("clean.txt");
    assert_eq!(
        baseline.clean_object("clean.txt", size, mtime),
        files.get("clean.txt").cloned()
    );
    let (size, mtime) = stat("edited.txt");
    assert_eq!(baseline.clean_object("edited.txt", size, mtime), None);
    assert_eq!(baseline.clean_object("nope.txt", 1, (0, 0)), None);
}

/// 3. Status shows each kind of change, and a commit clears it.
pub async fn status_reports_edits_and_is_clean_after_commit(p: &dyn Vcs, ws: &Path) {
    write(ws, "kept.txt", "kept\n");
    write(ws, "gone.txt", "gone\n");
    commit(p, ws, "base").await;
    write(ws, "kept.txt", "changed\n");
    std::fs::remove_file(ws.join("gone.txt")).unwrap();
    write(ws, "new.txt", "new\n");
    let status = p.status(ws).await.unwrap();
    let entries: Vec<(&str, FileStatus)> = status
        .entries
        .iter()
        .map(|e| (e.path.as_str(), e.status))
        .collect();
    assert_eq!(
        entries,
        vec![
            ("gone.txt", FileStatus::Deleted),
            ("kept.txt", FileStatus::Modified),
            ("new.txt", FileStatus::Untracked),
        ]
    );
    assert_eq!(status.in_progress, None);
    commit(p, ws, "all of it").await;
    assert!(p.status(ws).await.unwrap().entries.is_empty());
}

/// 5. The head resolves to the last commit, first in the log.
pub async fn head_resolves_and_the_log_walks_newest_first(p: &dyn Vcs, ws: &Path) {
    write(ws, "a.txt", "1\n");
    let first = commit(p, ws, "first").await;
    write(ws, "a.txt", "2\n");
    let second = commit(p, ws, "second").await;
    let head = p.head(ws).await.unwrap();
    assert_eq!(head.revision.as_deref(), Some(second.as_str()));
    assert!(head.branch.is_some(), "a fresh workspace is on a branch");
    assert_eq!(p.resolve(ws, "HEAD").await.unwrap(), second);
    assert_eq!(p.resolve(ws, &second[..8]).await.unwrap(), second);
    assert!(p.resolve(ws, "no-such-rev").await.is_err());
    let log = p
        .log(
            ws,
            LogQuery {
                limit: Some(2),
                all: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        log.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec![second.as_str(), first.as_str()]
    );
    assert_eq!(log[0].subject, "second");
    assert_eq!(log[0].parents, vec![first.clone()]);
    let detail = p.revision(ws, &second).await.unwrap().unwrap();
    assert_eq!(detail.files.len(), 1);
    assert_eq!(detail.files[0].path, "a.txt");
    assert_eq!(detail.files[0].status, FileStatus::Modified);
}

/// 6. Blame names the revision that last changed each line.
pub async fn blame_attributes_lines_to_their_revision(p: &dyn Vcs, ws: &Path) {
    write(ws, "a.txt", "one\n");
    let first = commit(p, ws, "first").await;
    write(ws, "a.txt", "one\ntwo\n");
    let second = commit(p, ws, "second").await;
    let lines = p.blame(ws, "a.txt", &second).await.unwrap();
    assert_eq!(
        lines
            .iter()
            .map(|l| (l.line, l.revision.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, first.as_str()), (2, second.as_str())]
    );
    // At the first revision, only its line exists.
    assert_eq!(p.blame(ws, "a.txt", &first).await.unwrap().len(), 1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcs::GitProvider;

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        dir
    }

    #[tokio::test]
    async fn git_a_commit_reads_back_what_it_captured() {
        let ws = workspace();
        a_commit_reads_back_what_it_captured(&GitProvider, ws.path()).await;
    }

    #[tokio::test]
    async fn git_the_clean_baseline_vouches_only_for_untouched_files() {
        let ws = workspace();
        the_clean_baseline_vouches_only_for_untouched_files(&GitProvider, ws.path()).await;
    }

    #[tokio::test]
    async fn git_two_revisions_diff_to_what_changed() {
        let ws = workspace();
        two_revisions_diff_to_what_changed(&GitProvider, ws.path()).await;
    }

    #[tokio::test]
    async fn git_status_reports_edits_and_is_clean_after_commit() {
        let ws = workspace();
        status_reports_edits_and_is_clean_after_commit(&GitProvider, ws.path()).await;
    }

    #[tokio::test]
    async fn git_head_resolves_and_the_log_walks_newest_first() {
        let ws = workspace();
        head_resolves_and_the_log_walks_newest_first(&GitProvider, ws.path()).await;
    }

    #[tokio::test]
    async fn git_blame_attributes_lines_to_their_revision() {
        let ws = workspace();
        blame_attributes_lines_to_their_revision(&GitProvider, ws.path()).await;
    }
}
