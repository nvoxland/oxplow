# VCS capability

How core reads and changes version control without knowing it's git
(P5, `target-architecture.md` §6.2). Git watching, the commit indexer
and the git-native extras live in [git-integration.md](./git-integration.md).

## The trait

`oxplow_domain::vcs::Vcs` is async and takes a **workspace root** on
every call, never a stream. A provider manages directories; routing a
stream to its directory is core's job (`WorktreeRouter`, below), so a
provider stays testable against a tempdir.

- **Revisions:** `head`, `resolve` (a branch, a short id, `HEAD` → the
  full id), `log`, `revision` (message and changed files),
  `revisions_between`, `file_history`.
- **Trees:** `files_at(rev)` (path → `ObjectId`), `diff(a, b)`,
  `object_store(ws)` — a synchronous `ObjectStore` (`read(id)`,
  `id_of(bytes)`: the id without writing it) — and
  `clean_baseline(ws)`, a synchronous `CleanBaseline` that vouches, by
  file stat, for files still byte-for-byte their head object (git: the
  index's cached stat). Both are sync because snapshot capture and
  content hashing use them from blocking threads.
- **Status, branches, blame:** `status` (sorted entries plus the
  in-progress op; conflicted paths show as `Conflicted`), `branches`,
  `divergence(base, head)`, `merge_base(a, b)`, `blame(path, rev?)` —
  `None` blames the working file, where an uncommitted line's
  `BlameLine.revision` is `None`.
- **Mutations:** `commit`, `stage`, `discard`, `fetch`, `pull`, `push`,
  `merge`, `checkout_branch`, `rename_branch`, `delete_branch`,
  `resolve_conflict(path, Ours | Theirs | Auto)`. They run only through
  the `vcs.*` bus commands (below), which announce the change: a
  provider emits nothing.
- **Working copies:** `detect(root)` (`Primary`, `Secondary` — another
  checkout of a repository whose primary lives elsewhere — or `None`),
  and, feature `isolated_workspaces`, `create_workspace`,
  `list_workspaces` and `remove_workspace` — what backs streams
  (`StreamService` in `oxplow-session` takes the `Vcs`).
- **Watching and the graph:** `watch_refs(ws, on_change)` calls back
  when refs move until the returned guard drops (the workspace watch
  registry turns it into `GitRefsChanged`); `revision_graph(ws)` is a
  synchronous `RevisionGraph` (ancestry, a revision's time) that metric
  visibility caches over.

`rev_kind()` names the provider's revisions in a ref's `@<kind>:<rev>`
slot (`git`); `features()` says what it supports beyond the floor. The
types are neutral (`RevisionInfo`, `FileStatus`, `Branch`, `OpOutcome`,
…). The only git types that cross IPC are the git provider's own, behind
the `git_*` RPCs (`ChangeScopes`, `CommitRefLabel`, `RemoteBranchEntry`,
re-exported from `oxplow_app::vcs`).

## The git provider

`oxplow_app::vcs::GitProvider`, a stateless unit struct over `oxplow_git`
that runs each call under `spawn_blocking`. `Services.vcs` holds it as
`Arc<dyn Vcs>`. Notes:

- `diff` compares content trees (`oxplow_domain::diff_trees`), so
  `rename_detection` is `false`: a rename is a delete plus an add.
- `merge` and `pull` run oxplow's smart merge over what they left
  conflicted (see [smart-merge.md](./smart-merge.md)); `OpOutcome`
  reports how many it auto-resolved and which paths remain.
- `resolve_conflict(Auto)` is that same pass, refused for a path whose
  edits overlap.
- `blame` takes a revision (`git blame --porcelain <rev>`), so history
  views blame the version they show.

## Around the provider

- **`WorktreeRouter`** (`Services.worktrees`): stream → directory. The
  stream store is the truth (`stream.worktree_path` never changes); the
  router memoizes lookups and `forget`s a deleted stream. `resolve`
  falls back to the primary checkout for no stream or an unknown one.
  `resolve_strict` refuses instead, and any stream-scoped write uses it
  (see git-integration.md "Stream-scoped destructive ops").
- **`WorkspaceFiles`** (`Services.workspace_files`): list, read, write,
  create, rename and delete under a stream's workspace, with
  path-traversal protection, annotated with `vcs.status`. Writes
  announce `WorkspaceChanged`. `WorkspaceEntry` and
  `WorkspaceIndexedFile` serialize camelCase (`status`, `hasChanges`),
  and the desktop uses the generated types directly. The hand-written
  TS copies went: `listWorkspaceEntries` had never mapped the
  snake_case wire row, so the file tree's change markers were
  undefined.
- **`BranchReconciler`** (`Services.branch_reconciler`, spawned at
  boot): keeps `stream.branch` equal to the checked-out branch (from
  `vcs.head`), once per stream at boot and on each `GitRefsChanged`. A
  detached head leaves the row alone.

## Revisions and `Trees`

**`Revision`** (`oxplow_domain::vcs`) is the one "which version" type:
`working`, `snap:<id>` or `<rev_kind>:<rev>` (`git:HEAD`). On the wire,
in tab ids and in `v_change.base_revision` / `head_revision` and
`v_code_quality_scan.revision` it is that string; in the ref grammar it
is the `@rev` slot, where the working tree is the omitted slot
(`rev_slot` / `from_rev_slot`). The desktop mirror is
`apps/desktop/src/revision.ts`. It replaced `TreeVersion` (and the
`oxplow-tree-source` crate), `DiffEndpoint` and the desktop's
`FileVersion`. A revision of another VCS than the workspace's is
refused, naming the workspace's.

**`Trees`** (`crates/oxplow-app/src/trees.rs`, `Services.trees`) reads
any revision of a workspace: `files_at`, `read_at`, `corpus` (text files,
for the code-quality scans) and `diff(from?, to)` with line counts.
Snapshots read their blobs from the blob store and VCS-backed rows
through the object store; the working tree reuses the head's object id
for a file `status` calls clean and hashes the rest (`ObjectStore::id_of`). Every side honours the workspace filter
(`generated:` plus `.gitignore`) — the working-tree corpus no longer has
its own skip list. Two VCS revisions diff through `Vcs::diff`; two
snapshots settle un-hashed rows first (`resolve_for_compare`); a mixed
pair normalizes the snapshot side into VCS ids.

The neutral RPCs are `read_at { streamId, path, revision }`, `files_at`
and `diff { streamId, from, to }` (UI), and MCP `read_at`; they replaced
`read_file`, `read_file_at_ref` and `diff_endpoints`. Change analysis
and the duplicate scan read through `Trees`, so a closed effort's
snapshot head is scanned too. (The wiki's old `@<rev>` link syntax went
in P5.C3: a link names a file; its version is the edge's pin —
[knowledge.md](./knowledge.md).)

## The stream's reads (P5.B4)

`oxplow_app::vcs::reads` is what the UI and agent tools call: `head`
(`HeadInfo { revision, branch }`), `status`, `blame(path, revision)`,
`revision(revision)` (a commit's message and files) and
`merge_base(a, b)` — each routed by stream and naming versions as
`Revision`s (a snapshot answers with the revision its tree equals). RPCs
`vcs_head`, `vcs_status`, `vcs_blame`, `vcs_revision`, `vcs_merge_base`
(UI; `vcs_blame` also on MCP) replaced `get_repo_conflict_state`,
`get_workspace_status_summary`, `git_blame`, `local_blame`,
`get_commit_detail` and `get_branch_changes`; MCP `git_diff` became
`diff { from?, to, since_fork }`. On the desktop: the changed-files lists
diff `head → working` (`vcsHead` + `diffRevisions`), branch scopes diff
from `vcsMergeBase`, the rail's counts and conflicts come from
`vcsStatus` (`countStatus`), the commit page and dashboard stats from
`vcsRevision`, and the blame overlay from `vcsBlame(…, WORKING)`.
`DiffViewPage.smoke.test.tsx` renders the diff view and the commit page
over a command Proxy that records any git-shaped call.

## History and branches on the models (P5.B5)

The commit indexer walks every stream's head (`Vcs::log`, `revision`)
into `v_commit` (with `parents`, v2) and `v_commit_file`, and restates
`v_branch` (`is_default`, v2) and `v_tag` from `Vcs::branches` / `tags`.
The desktop reads history from the models (`apps/desktop/src/vcsHistory.ts`):
a stream's history is a recursive CTE over `json_each(parents)` from its
head (`vcsHead`), re-run on `modelsChanged` (`useRerunOnChange`); the
branch picker, compare list and new-stream form read `v_branch` /
`v_tag`. What is computed stays live: `vcs_divergence`,
`vcs_revisions_between`, `vcs_file_history` (the index keeps 500
commits per head; a file's full history can be older),
`vcs_list_adoptable_workspaces`, and for agents `vcs_log` /
`vcs_branches` (MCP too). Git-native reads keep `git_` names:
`git_change_scopes`, `git_resolve_commit_ref_labels`,
`git_list_recent_remote_branches`. Deleted: `get_git_log`,
`get_commits_ahead_of`, `list_file_commits`, `list_branches`,
`list_local_branches`, `get_default_branch`, `get_ahead_behind`,
`list_stream_divergences`, `list_adoptable_worktrees`, `list_all_refs`
(the "Compare with…" list it fed had been casting a grouped object to a
flat list; it now reads `readRefOptions`).

## Mutations are commands (P5.B6)

Every change to a repository is a bus command (`commands/vcs.rs`,
table in [commands.md](./commands.md)): `vcs.commit`, `vcs.stage`,
`vcs.discard`, `vcs.fetch`, `vcs.pull`, `vcs.push`, `vcs.merge`,
`vcs.checkout_branch`, `vcs.rename_branch`, `vcs.delete_branch`,
`vcs.resolve_conflict`, and git's own `git.rebase`, `git.cherry_pick`,
`git.revert`, `git.ignore` (inherent methods on `GitProvider`). They are
`External` (the VCS isn't the bus's to roll back), a person's only,
not undoable, and name their `stream` (`WorktreeRouter::resolve_strict`).
Discard, merge, branch delete, rebase and revert are `Destructive`. The
audit row keeps the returned `OpOutcome`, so a merge's conflicts are on
record. The desktop runs them with `runCommand` through `api.ts`
wrappers (`vcsMerge`, `vcsPush`, `gitRebase`, …); a destructive one
passes `confirmed` only from behind the person's own confirmation
(the branch picker's and commit page's inline confirms, the Files
panel's rollback confirm, the dashboard's), and an unconfirmed call is
refused `NEEDS_CONFIRMATION`. Long ones still run inside a background
task row (`GitOpKickoff`), whose result is the `OpOutcome`;
`apps/desktop/src/git-op.ts` normalizes it. `OpOutcome` is exported to
the bindings by `specta_builder().typ`, since no RPC returns it.

Deleted with them: the `git_*` mutation RPCs (`git_commit_all`,
`git_add_path`, `restore_path`, `append_to_gitignore`, `git_fetch`,
`git_pull`, `git_pull_remote_into_current`, `git_push`,
`git_push_current_to`, `git_merge_into`, `git_rebase_onto`,
`git_cherry_pick`, `git_revert`), `rename_branch`, `delete_branch`,
`checkout_stream_branch` (which shelled `git checkout` itself; the
branch reconciler now records a checkout, so
`StreamService::record_branch_checkout` went too), `GitService`'s
mutations, and the Push/Pull dialog's force, set-upstream and rebase
checkboxes, which the old RPCs had never passed on.

## Snapshots and revisions

A snapshot taken on a clean workspace *is* its head revision:
`snapshot.revision` (V112, was `git_commit`) holds it as a `Revision`
string (`git:<sha>`), and `snapshot.branch` (was `git_branch`) the
branch. `SnapshotCaptureService::clean_head` asks `Vcs::head` +
`Vcs::status`; the git-refs listener re-stamps the latest snapshot's
revision when the head moves onto an unchanged tree (`vcs.head.moved`).
`Trees::revision_of(snapshot)` and `Trees::snapshots_at(ws, rev)` map the
two ways (`git:HEAD` and short ids resolve first).

Capture reads git only through the capability: the sweep's clean files
come from `Vcs::clean_baseline` and are stored by object id (storage
class `git`, the VCS object store); `same_content` compares an un-hashed
VCS row by `ObjectStore::id_of`. Reading a captured file's bytes is
`SnapshotContent` (`Services.snapshot_content`: the blob store plus the
VCS object store — every workspace shares one), which also hashes VCS
objects into the content-identity space for `resolve_for_compare`.
`file_ref_version::resolve` reads the head through `Vcs::head`.

## Conformance

`oxplow_app::vcs_conformance` (test-only) holds the contract as
`async fn(&dyn Vcs, &Path)` checks over a fresh workspace, so a second
provider runs the same list:

1. a commit reads back what it captured, whatever the working tree
   holds now, through the object store; the clean baseline vouches for
   an untouched committed file by its stat and never for an edited one;
2. two revisions diff to what changed, deletions included;
3. status reports added/modified/deleted/untracked and is clean after a
   commit;
4. two lines of history meet at their merge base (checkout creates and
   switches branches);
5. the head resolves (a short id too; an unknown rev errors) and the
   log walks newest first;
6. blame attributes each line to its revision, at any revision, and in
   the working tree leaves uncommitted lines unattributed.

7 (a snapshot taken on a clean workspace maps to its head revision
and back, and diffs empty against it) needs the snapshot store, so it
runs in `trees.rs`. Still to come: branch listing and divergence, and 8 (isolated
workspaces share history).

**`only_the_git_provider_touches_git`** (`vcs/mod.rs`) scans the
crates' production code: `oxplow_git::` and `git2::` appear only in
`vcs/git.rs`. Test code — after `#[cfg(test)]`, a `#![cfg(test)]`
module, `tests/` and `benches/` — is exempt; it builds real repositories
as fixtures. (Through B3–B6 a `NOT_YET_ON_VCS` list held the files still
to move; it emptied in B7.) Since the provider is its only caller,
`oxplow-git` keeps only what `vcs/git.rs` (or the crate itself) uses:
what the moved callers left behind — `get_ahead_behind`, `list_all_refs`
and its ref-option shapes, `clean_head_blob_oids`, `status_for_path` —
was deleted with its tests.

## The last callers (P5.B7)

- **Streams**: `StreamService` validates the project with `detect` (not
  under version control → `NotARepo`, a secondary checkout →
  `InWorktree`), reads a branch with `head`, and creates and removes
  worktrees with `create_workspace` / `remove_workspace` — the raw `git
  worktree remove/prune` shell-outs went. Its tests need a real
  provider, so they live in `oxplow-app` (`stream_service_tests.rs`).
- **Branch stamps** on facts and captures (collection, metrics, the
  effort-lifecycle metrics) read `Vcs::head`.
- **Revert detection** (token waste) reads the head and the reverted
  commits with `Vcs::head` / `Vcs::revision`.
- **Workspace context** asks `detect` and the default branch from
  `branches()`.
- **Text search** is `WorkspaceFiles::search_text`, no longer `git grep`.
- **Co-change** reads the commit index (`crate::co_change`,
  [git-integration.md](./git-integration.md) "Commit indexer").
- **Metric visibility** asks `revision_graph` (it had opened libgit2
  itself).
- `GitService` is gone: its git-native reads are `GitProvider` methods,
  and `Services.git` is the provider.

Still git-named: the commit-id columns outside snapshots
(`closest_git_version`, `git_version_exact`), the `GitRefsChanged`
event, the workspace context's `is_git_repo`, and the project-root
`.git` watcher (git-integration.md) — renamed in their own task.
