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
- **Trees:** `files_at(rev)` (path → `ObjectId`), `read_object`,
  `object_id_of(bytes)` (the id without writing it), `diff(a, b)`.
- **Status, branches, blame:** `status` (sorted entries plus the
  in-progress op; conflicted paths show as `Conflicted`), `branches`,
  `divergence(base, head)`, `blame(path, rev)`.
- **Mutations:** `commit`, `stage`, `discard`, `fetch`, `pull`, `push`,
  `merge`, `checkout_branch`, `rename_branch`, `delete_branch`,
  `resolve_conflict(path, Ours | Theirs | Auto)`. They run only through
  the `vcs.*` bus commands (P5.B6), which announce the change: a provider
  emits nothing.
- **Feature `isolated_workspaces`:** `create_workspace`,
  `list_workspaces` — what backs streams.

`rev_kind()` names the provider's revisions in a ref's `@<kind>:<rev>`
slot (`git`); `features()` says what it supports beyond the floor. The
types are neutral (`RevisionInfo`, `FileStatus`, `Branch`, `OpOutcome`,
…). No `oxplow_git` type is meant to cross IPC: the workspace listings
already carry `FileStatus`, and the remaining git-shaped RPCs go in
B4–B7.

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

## Conformance

`oxplow_app::vcs_conformance` (test-only) holds the contract as
`async fn(&dyn Vcs, &Path)` checks over a fresh workspace, so a second
provider runs the same list:

1. a commit reads back what it captured, whatever the working tree
   holds now;
3. status reports added/modified/deleted/untracked and is clean after a
   commit;
5. the head resolves (a short id too; an unknown rev errors) and the
   log walks newest first;
6. blame attributes each line to its revision, at any revision.

Still to come: 2 (diffs, with `Trees`, B2), 4 (branches follow checkout
and diverge), 7 (a snapshot at a clean head maps to its revision, B3)
and 8 (isolated workspaces share history).

## Still git-shaped (P5 B2–B7)

`GitService` keeps its stream-taking methods until each RPC moves:
reads to the neutral `vcs_*` RPCs (B4), history and branches to SQL
(B5), and mutations to `vcs.*` commands (B6). After that, a crate-scan
test pins `oxplow_git::` to `vcs/git.rs` (B7).
