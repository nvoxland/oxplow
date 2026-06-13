//! Git integration for oxplow.
//!
//! Repo detection, branch listing, worktree management, and conflict
//! state. Uses `git2` for in-process ops where it's well-supported,
//! falling back to `Command::new("git")` for cases libgit2 doesn't
//! cover well (e.g. `git worktree add`).

pub mod blame;
mod branch;
mod branch_changes;
mod branch_ops;
pub mod co_change;
mod conflict;
pub mod log;
pub mod refs;
mod refs_watch;
mod repo;
pub mod smart_merge;
pub mod status;
pub mod sync;
pub mod tree;
pub mod workspace;
mod worktree;

pub use blame::{
    git_blame, local_blame, parse_porcelain, BlameLine, LocalBlameEntry, BLAME_ZERO_SHA,
};
pub use branch::{list_branches, BranchRef, BranchRefKind};
pub use branch_changes::{
    get_change_scopes, list_branch_changes, BranchChangeEntry, BranchChanges, ChangeKind,
    ChangeScopes,
};
pub use branch_ops::{
    append_to_gitignore, delete_branch, detect_default_branch, get_ahead_behind,
    get_commits_ahead_of, rename_branch, restore_path, AheadBehind, BranchOpError,
};
pub use conflict::{get_repo_conflict_state, GitOperationKind, RepoConflictState};
pub use log::{
    get_commit_detail, get_git_log, CommitDetail, CommitDetailFile, GitLogCommit, GitLogOptions,
    GitLogResult,
};
pub use refs::{
    list_all_refs, list_file_commits, list_recent_remote_branches, read_file_at_ref,
    resolve_commit_ref_labels, CommitRefLabel, CommitRefLabelKind, GroupedGitRefs, RefKind,
    RefOption, RemoteBranchEntry,
};
pub use refs_watch::{GitRefsWatcher, RefsChangeEvent};
pub use repo::{detect_current_branch, is_git_repo, is_git_worktree};
pub use smart_merge::{auto_resolve_conflicts, merge3, merge3_str, tokenize, AutoResolveReport};
pub use status::{head_commit_sha, list_git_statuses, status_for_path};
pub use sync::{
    add_path, commit_all, fetch, merge, pull, pull_remote_into_current, push, push_current_to,
    rebase, search_workspace_text, GitOpResult, TextSearchHit,
};
pub use tree::{diff_commits, tree_at_commit};
pub use workspace::{
    create_workspace_directory, create_workspace_file, delete_workspace_path,
    list_workspace_entries, list_workspace_files, read_workspace_file, rename_workspace_path,
    summarize_git_statuses, write_workspace_file, GitFileStatus, WorkspaceEntry,
    WorkspaceEntryKind, WorkspaceError, WorkspaceFile, WorkspaceIndexedFile,
    WorkspaceStatusSummary,
};
pub use worktree::{
    ensure_worktree, list_adoptable_worktrees, list_existing_worktrees, EnsureWorktreeError,
    GitWorktreeEntry,
};
