//! The VCS capability (P5, `.context/target-architecture.md` §6.2,
//! `.context/vcs.md`): what core needs from a version-control system,
//! with no git in it. The default provider is git
//! (`oxplow_app::vcs::GitProvider`); a second provider (jj, Sapling) implements
//! this trait and passes the same conformance suite
//! (`oxplow_app::vcs_conformance`).
//!
//! Every call takes a **workspace root** — a directory the VCS manages —
//! never a stream: routing a stream to its worktree is core's job
//! (`WorktreeRouter`), so the provider stays testable against a tempdir.
//! Mutations are reached only through the `vcs.*` bus commands, which
//! also announce the change; the provider itself emits nothing.

use std::collections::BTreeMap;
use std::path::Path;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::FileChange;

/// A content address in the provider's object store (a git blob oid).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct ObjectId(pub String);

/// What the provider supports beyond the floor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct VcsFeatures {
    /// Several working copies of one repository (git worktrees) — what
    /// backs streams.
    pub isolated_workspaces: bool,
    /// Remotes to fetch from and push to.
    pub remotes: bool,
    /// `diff` reports renames rather than a delete plus an add.
    pub rename_detection: bool,
}

/// Where a workspace is: the revision it's on and its branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Head {
    /// The full revision id; `None` before the first commit.
    pub revision: Option<String>,
    /// The branch checked out; `None` when detached.
    pub branch: Option<String>,
}

/// Which revisions `log` walks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct LogQuery {
    /// At most this many (newest first); `None` for the provider's default.
    pub limit: Option<u32>,
    /// Every branch's history, not only the head's.
    pub all: bool,
}

/// One revision, as a list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RevisionInfo {
    pub id: String,
    pub short_id: String,
    pub author: String,
    pub email: String,
    /// Seconds since the Unix epoch.
    pub time: i64,
    pub subject: String,
    pub parents: Vec<String>,
}

/// One revision in full: its message and the files it changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RevisionDetail {
    pub info: RevisionInfo,
    pub body: String,
    pub files: Vec<RevisionFile>,
}

/// A file a revision changed, against its first parent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RevisionFile {
    pub path: String,
    pub status: FileStatus,
    pub additions: u32,
    pub deletions: u32,
}

/// How a path differs from the head, or how a revision changed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

impl FileStatus {
    /// Its wire name (`added`, `modified`, …).
    pub fn as_str(self) -> &'static str {
        match self {
            FileStatus::Added => "added",
            FileStatus::Modified => "modified",
            FileStatus::Deleted => "deleted",
            FileStatus::Renamed => "renamed",
            FileStatus::Untracked => "untracked",
            FileStatus::Conflicted => "conflicted",
        }
    }
}

/// One changed path in a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct StatusEntry {
    pub path: String,
    pub status: FileStatus,
}

/// An operation paused mid-way, waiting on its conflicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum InProgressOp {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

/// A workspace's changes against its head.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct WorkspaceStatus {
    /// Changed paths, sorted.
    pub entries: Vec<StatusEntry>,
    /// The operation in progress, if one is.
    pub in_progress: Option<InProgressOp>,
}

/// A branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Branch {
    pub name: String,
    /// The remote it tracks a copy of; `None` for a local branch.
    pub remote: Option<String>,
    /// The revision it points at, when it resolves.
    pub head: Option<String>,
    /// The repository's default branch (`main`).
    pub is_default: bool,
}

/// A tag: a name fixed to one revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Tag {
    pub name: String,
    pub revision: String,
}

/// Whether `head` would merge into `base` cleanly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MergeReadiness {
    /// `head` has nothing `base` lacks.
    AlreadyIntegrated,
    /// `head` is ahead and no file changed on both sides.
    Clean,
    /// `head` is ahead and some file changed on both sides.
    Conflict,
}

/// How far `head` and `base` have diverged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Divergence {
    pub ahead: u32,
    pub behind: u32,
    /// Files changed on both sides since they split, sorted.
    pub overlapping_files: Vec<String>,
    pub readiness: MergeReadiness,
}

/// Who last changed one line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct BlameLine {
    /// 1-based.
    pub line: u32,
    /// The revision that last changed the line; `None` for a line not
    /// committed yet (blaming the working tree).
    pub revision: Option<String>,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub summary: String,
}

/// What a mutation did. `log` is the provider's own account (a CLI's
/// output) for the person to read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct OpOutcome {
    pub success: bool,
    pub log: String,
    /// Paths still conflicted after the operation.
    pub conflicts: Vec<String>,
    /// Conflicts oxplow's smart merge resolved on its own.
    pub auto_resolved: u32,
}

/// A commit to make.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CommitRequest {
    pub message: String,
    /// Commit untracked files too, not only changes to tracked ones.
    pub include_untracked: bool,
}

/// A remote branch to push to or pull from; `None` means the branch's
/// own upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RemoteBranch {
    pub remote: String,
    pub branch: String,
}

/// How to settle one conflicted path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ConflictChoice {
    /// Keep the workspace's side.
    Ours,
    /// Take the incoming side.
    Theirs,
    /// oxplow's smart merge, when the edits don't overlap.
    Auto,
}

/// One working copy of the repository (feature `isolated_workspaces`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct VcsWorkspace {
    pub path: String,
    pub branch: Option<String>,
    pub head: Option<String>,
    /// The repository's primary working copy.
    pub is_main: bool,
}

/// Why a VCS call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VcsError {
    #[error("{0} isn't under version control")]
    NotARepository(String),
    #[error("no revision `{0}`")]
    UnknownRevision(String),
    #[error("{0}")]
    Failed(String),
}

impl From<VcsError> for crate::DomainError {
    fn from(e: VcsError) -> Self {
        match e {
            VcsError::NotARepository(_) | VcsError::UnknownRevision(_) => {
                crate::DomainError::Invalid(e.to_string())
            }
            VcsError::Failed(m) => crate::DomainError::Invariant(m),
        }
    }
}

/// A VCS's content-addressed object store.
pub trait ObjectStore: Send + Sync {
    /// An object's bytes; `None` when the store doesn't have it (never
    /// written, or collected after a history rewrite).
    fn read(&self, id: &ObjectId) -> Option<Vec<u8>>;
    /// The id `bytes` would have in the store (not written).
    fn id_of(&self, bytes: &[u8]) -> ObjectId;
}

/// Which files are, per the VCS's own cached stat, byte-for-byte their
/// head object ([`Vcs::clean_baseline`]).
pub trait CleanBaseline: Send + Sync {
    /// The head object `path` still is, given its current size and
    /// mtime `(secs, nanos)`; `None` when that can't be vouched for (not
    /// in the head, edited, or touched too close to the VCS's last stat
    /// to tell).
    fn clean_object(&self, path: &str, size: u64, mtime: (i64, u32)) -> Option<ObjectId>;
    /// How many files it can vouch for at most.
    fn candidates(&self) -> usize;
}

/// The VCS capability. See the module docs.
#[async_trait]
pub trait Vcs: Send + Sync {
    /// The provider's revision kind — the `@<kind>:<rev>` slot of a ref
    /// (`git`).
    fn rev_kind(&self) -> &'static str;
    fn features(&self) -> VcsFeatures;
    /// Whether `root` is the top of a workspace this provider manages.
    async fn detect(&self, root: &Path) -> bool;

    // --- revisions ---
    async fn head(&self, ws: &Path) -> Result<Head, VcsError>;
    /// The full id `rev` names (a branch, a short id, `HEAD`).
    async fn resolve(&self, ws: &Path, rev: &str) -> Result<String, VcsError>;
    async fn log(&self, ws: &Path, query: LogQuery) -> Result<Vec<RevisionInfo>, VcsError>;
    async fn revision(&self, ws: &Path, rev: &str) -> Result<Option<RevisionDetail>, VcsError>;
    /// Revisions on `head` that `base` lacks, newest first.
    async fn revisions_between(
        &self,
        ws: &Path,
        base: &str,
        head: &str,
        limit: u32,
    ) -> Result<Vec<RevisionInfo>, VcsError>;
    /// Revisions that changed `path`, newest first.
    async fn file_history(
        &self,
        ws: &Path,
        path: &str,
        limit: u32,
    ) -> Result<Vec<RevisionInfo>, VcsError>;

    // --- trees ---
    /// Every file at `rev`: path → object id.
    async fn files_at(&self, ws: &Path, rev: &str) -> Result<BTreeMap<String, ObjectId>, VcsError>;
    /// The object store behind `ws` (every workspace of one repository
    /// shares it). Synchronous: snapshot capture and content hashing read
    /// it from blocking threads.
    fn object_store(&self, ws: &Path) -> std::sync::Arc<dyn ObjectStore>;
    /// What the VCS already knows is unchanged since the head, by file
    /// stat — so a capture can back those files by their head objects
    /// without reading them. Blocking; build it once per sweep.
    fn clean_baseline(&self, ws: &Path) -> Box<dyn CleanBaseline>;
    /// What changed from `a` to `b`, sorted by path.
    async fn diff(&self, ws: &Path, a: &str, b: &str) -> Result<Vec<FileChange>, VcsError>;

    // --- status, branches, blame ---
    async fn status(&self, ws: &Path) -> Result<WorkspaceStatus, VcsError>;
    async fn branches(&self, ws: &Path) -> Result<Vec<Branch>, VcsError>;
    /// Every tag, sorted by name.
    async fn tags(&self, ws: &Path) -> Result<Vec<Tag>, VcsError>;
    async fn divergence(&self, ws: &Path, base: &str, head: &str) -> Result<Divergence, VcsError>;
    /// Who last changed each line of `path` at `rev`, or in the working
    /// tree when `rev` is `None`.
    async fn blame(
        &self,
        ws: &Path,
        path: &str,
        rev: Option<&str>,
    ) -> Result<Vec<BlameLine>, VcsError>;
    /// Where the histories of `a` and `b` fork; `None` when they share
    /// none.
    async fn merge_base(&self, ws: &Path, a: &str, b: &str) -> Result<Option<String>, VcsError>;

    // --- mutations (through the `vcs.*` commands only) ---
    /// Commit the workspace's changes; the new revision's id.
    async fn commit(&self, ws: &Path, req: CommitRequest) -> Result<String, VcsError>;
    async fn stage(&self, ws: &Path, paths: &[String]) -> Result<(), VcsError>;
    /// Throw away the workspace's changes to `paths`.
    async fn discard(&self, ws: &Path, paths: &[String]) -> Result<(), VcsError>;
    async fn fetch(&self, ws: &Path, remote: Option<&str>) -> Result<OpOutcome, VcsError>;
    async fn pull(&self, ws: &Path, from: Option<RemoteBranch>) -> Result<OpOutcome, VcsError>;
    async fn push(&self, ws: &Path, to: Option<RemoteBranch>) -> Result<OpOutcome, VcsError>;
    async fn merge(&self, ws: &Path, rev: &str) -> Result<OpOutcome, VcsError>;
    async fn checkout_branch(&self, ws: &Path, name: &str, create: bool) -> Result<(), VcsError>;
    async fn rename_branch(&self, ws: &Path, from: &str, to: &str) -> Result<(), VcsError>;
    async fn delete_branch(&self, ws: &Path, name: &str, force: bool) -> Result<(), VcsError>;
    async fn resolve_conflict(
        &self,
        ws: &Path,
        path: &str,
        choice: ConflictChoice,
    ) -> Result<(), VcsError>;

    // --- feature `isolated_workspaces` ---
    /// A new working copy at `at` on `branch`, created from `from` when
    /// the branch doesn't exist yet.
    async fn create_workspace(
        &self,
        repo: &Path,
        at: &Path,
        branch: &str,
        from: &str,
    ) -> Result<(), VcsError>;
    async fn list_workspaces(&self, repo: &Path) -> Result<Vec<VcsWorkspace>, VcsError>;
}

/// One version of a workspace's tree: the working tree, a local-history
/// snapshot, or a VCS revision. The one "which version of a file" type
/// (it replaced `TreeVersion`, `DiffEndpoint` and the desktop's
/// `FileVersion`); `oxplow_app::trees::Trees` reads and diffs any of
/// them.
///
/// On the wire it is a string: `working`, `snap:<id>`, or the VCS's
/// `<rev_kind>:<rev>` (`git:HEAD`, `git:4c44d4…`) — the ref grammar's
/// `@rev` slot (`.context/refs.md`), where the working tree is the
/// omitted slot ([`Revision::rev_slot`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Revision {
    Working,
    Snapshot(i64),
    /// A revision of the VCS whose `rev_kind` is `kind`.
    Vcs {
        kind: String,
        rev: String,
    },
}

impl Revision {
    /// A git revision (a sha, a branch, `HEAD`).
    pub fn git(rev: impl Into<String>) -> Self {
        Revision::Vcs {
            kind: "git".into(),
            rev: rev.into(),
        }
    }

    /// The id a VCS revision names in its VCS (a commit sha); `None`
    /// for the working tree and snapshots.
    pub fn vcs_rev(&self) -> Option<&str> {
        match self {
            Revision::Vcs { rev, .. } => Some(rev),
            _ => None,
        }
    }

    /// The ref grammar's `@rev` slot: `None` for the working tree.
    pub fn rev_slot(&self) -> Option<String> {
        match self {
            Revision::Working => None,
            other => Some(other.to_string()),
        }
    }

    /// Inverse of [`Self::rev_slot`].
    pub fn from_rev_slot(slot: Option<&str>) -> Result<Self, String> {
        match slot {
            None => Ok(Revision::Working),
            Some(s) => s.parse(),
        }
    }
}

impl std::fmt::Display for Revision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Revision::Working => f.write_str("working"),
            Revision::Snapshot(id) => write!(f, "snap:{id}"),
            Revision::Vcs { kind, rev } => write!(f, "{kind}:{rev}"),
        }
    }
}

impl std::str::FromStr for Revision {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        if s == "working" {
            return Ok(Revision::Working);
        }
        let (kind, value) = s.split_once(':').ok_or_else(|| {
            format!("`{s}` isn't a revision (`working`, `snap:<id>`, `<vcs>:<rev>`)")
        })?;
        if value.is_empty() {
            return Err(format!("`{s}` names no revision"));
        }
        match kind {
            "snap" => value
                .parse()
                .map(Revision::Snapshot)
                .map_err(|_| format!("`{value}` isn't a snapshot id")),
            "" => Err(format!("`{s}` names no revision kind")),
            _ => Ok(Revision::Vcs {
                kind: kind.into(),
                rev: value.into(),
            }),
        }
    }
}

impl Serialize for Revision {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Revision {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl specta::Type for Revision {
    fn definition(_types: &mut specta::Types) -> specta::datatype::DataType {
        specta::datatype::DataType::Reference(specta_typescript::define("string"))
    }
}

#[cfg(test)]
mod revision_tests {
    use super::Revision;

    /// P5.B2 (tsk521): a revision round-trips through its wire string and
    /// the ref grammar's `@rev` slot, where the working tree is the
    /// omitted slot.
    #[test]
    fn revisions_round_trip_through_the_wire_and_the_rev_slot() {
        for (rev, wire) in [
            (Revision::Working, "working"),
            (Revision::Snapshot(42), "snap:42"),
            (Revision::git("HEAD"), "git:HEAD"),
            (Revision::git("a:b"), "git:a:b"),
        ] {
            assert_eq!(rev.to_string(), wire);
            assert_eq!(wire.parse::<Revision>().unwrap(), rev);
            assert_eq!(serde_json::to_value(&rev).unwrap(), serde_json::json!(wire));
            assert_eq!(
                Revision::from_rev_slot(rev.rev_slot().as_deref()).unwrap(),
                rev
            );
        }
        assert_eq!(Revision::Working.rev_slot(), None);
        for bad in ["", "HEAD", "snap:x", "git:", ":x"] {
            assert!(bad.parse::<Revision>().is_err(), "{bad}");
        }
    }
}
