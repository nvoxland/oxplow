# Git integration

What this doc covers: the three filesystem watchers that keep git state
fresh in the UI, the runtime-side git operations, and the rule that
agents never call `git` directly. For the data side of commits (commit
points), see [data-model.md](./data-model.md) and
[agent-model.md](./agent-model.md).

## Three watchers

The runtime keeps three independent `fs.watch`-based watchers running.
Each cares about a different slice of the project state.

### 1. Workspace watcher

`src/git/workspace-watch.ts` — `WorkspaceWatcherRegistry`. One watcher
per stream, recursive on the worktree (`fs.watch(rootDir, { recursive: true })`),
**explicitly excluding `.git`** via `shouldIgnoreWorkspaceWatchPath`.

Drives `workspace.changed` events. Consumed by:

- `ProjectPanel` to refresh the file tree.
- `EditorPane` for external-file-changed prompts.
- `runtime.markDirty` (invoked from the watcher callback) to add the
  path to the per-stream in-memory dirty set, which the next snapshot
  flush reads as its optimizer hint.

Source files mutate constantly; this watcher's job is to keep the
file-tree current and to feed the snapshot dirty set.

### 2. Git root watcher

A tiny `fs.watch` on `projectDir` itself, set up inline in
`runtime.initialize` (`gitRootWatcher` field). Listens only for direntry
changes whose filename is `.git`.

Fires when the user runs `git init` (or removes `.git`) in the project
root. On change:

- Re-reads `isGitRepo(projectDir)` and updates `gitEnabledCached`.
- Publishes `workspace-context.changed` with the new `gitEnabled` flag
  so UI surfaces (e.g. branch picker, stream creation form) enable or
  disable themselves.
- Re-binds the **git refs watcher** for every stream (starts watching if
  `.git` just appeared, stops if it disappeared).

This is the only watcher that lives at the project-root level rather
than per-stream.

### 3. Git refs watcher

`src/git/git-refs-watch.ts` — `GitRefsWatcherRegistry`. One watcher per
stream, recursive on the stream's `.git/` directory, debounced ~200ms
(a single `git commit` fires a dozen events touching `HEAD`, `refs/*`,
`logs/*`, `index`, `ORIG_HEAD`, …).

When the stream lives in a secondary worktree (the common case — oxplow
manages its own worktrees under `.oxplow/worktrees/`), the stream's
`.git` is a pointer file, not a directory. The watcher reads the
`gitdir:` line to find the per-worktree state dir (containing `HEAD`,
`index`, `logs/HEAD`) and also follows the `commondir` pointer to watch
the shared `.git` (where `refs/heads/*` actually update). Both dirs are
watched; without the commondir watch, `git fetch` / ref updates from
outside the worktree would be missed.

Fires `git-refs.changed` after each debounce. Consumed silently (no
loading spinner) by:

- `HistoryPanel` — reloads the commit log.
- `ProjectPanel` — refreshes the indexed git statuses.
- (Formerly `GitChangesPanel`, now folded into `ProjectPanel`'s filter
  modes.)

The recursive `fs.watch` falls back to per-subdir watching on platforms
that don't support recursive mode.

### 4. Notes watcher

`src/git/notes-watch.ts` — not really a git watcher, but lives next
to the others because it wraps `fs.watch` the same way. Watches
`.oxplow/notes/` for `.md` file create/change/delete, debounces
~200ms per slug, and calls `syncNoteFromDisk` → `WikiNoteStore.upsert`
(or `deleteBySlug`). Captures current HEAD (`readWorktreeHeadSha`)
and per-reference blob SHA-256 hashes as the freshness baseline.

Every write is treated identically — agent and user edits both
re-baseline freshness — so the watcher is the single sync path for
`wiki_note` metadata. See `data-model.md` → `wiki_note`.

### Why three

They watch overlapping but disjoint things:

- workspace = source files (excluding `.git`)
- root watcher = appearance/disappearance of `.git`
- refs watcher = mutations *inside* `.git`

A single recursive watcher on the root would lump them together and
either spam the UI on every internal git op or miss external changes
that don't touch source files.

## Runtime git operations

All git invocations go through `src/git/git.ts`. Notable:

- `gitBlame(projectDir, path)` — `git blame --porcelain HEAD` parsed via
  `parseBlamePorcelain`. Powers the editor blame overlay.
- `gitCommitAll(projectDir, message, options?)` — `git add -u` (or
  `git add -A` when `options.includeUntracked` is true) then
  `git commit -m message`, returning the new sha. Only used by the
  Files-panel commit dialog — the runtime never calls it elsewhere
  and no MCP tool invokes git commits. Commits not started from the
  Files dialog are user-driven via `git commit` in the terminal.
- `listBranchChanges`, `getGitLog`, `getCommitDetail`, `getChangeScopes`,
  `searchWorkspaceText`, `restorePath`, `addPath`, `appendToGitignore`,
  `listFileCommits`, `listAllRefs`,
  `readFileAtRef`, `listGitStatuses` — straight `execFileSync` wrappers
  exposed via IPC for UI consumption.
- `gitPush` / `gitPull` / `gitMerge` / `gitRebase` ship sync wrappers
  plus async siblings `gitPushAsync` / `gitPullAsync` / `gitMergeAsync` /
  `gitRebaseAsync` (and a `gitFetchAsync` helper) backed by
  `child_process.execFile` + `promisify`. The runtime IPC handlers
  use the async variants so the main process stays responsive during
  the network or merge work, and they register a row with the
  `BackgroundTaskStore` so the bottom-bar `BackgroundTaskIndicator`
  shows progress. The sync wrappers stay around for code paths that
  haven't been promoted yet (e.g. `gitCommitAll`'s internal calls,
  unit tests).
- `getGitLog` accepts an `all` option (defaults `true`). Pass
  `{ all: false }` to drop `--all` so the log only walks commits
  reachable from `HEAD`'s branch — used by the Git Dashboard's
  "Recent commits" card so the graph stays scoped to the current
  branch.
- `getAheadBehind(projectDir, base, head?)` — wraps
  `git rev-list --left-right --count base...head` and returns
  `{ ahead, behind }` relative to `base`. `head` defaults to `HEAD`.
  Powers the Git Dashboard branch header and worktree rows.
- `getCommitsAheadOf(projectDir, base, head, limit=50)` — wraps
  `git log base..head` with the same parser used by `getGitLog`, for
  pairwise commit-diff displays.
- `listRecentRemoteBranches(projectDir, limit=20)` — wraps
  `git for-each-ref --sort=-committerdate refs/remotes` and returns
  `RemoteBranchEntry[]` (filters out `<remote>/HEAD`). Drives the
  dashboard's recent-remote-branches card.
- `gitPushCurrentTo` / `gitPushCurrentToAsync(projectDir, remote, branch)`
  — runs `git push <remote> HEAD:refs/heads/<branch>`. Refspec push;
  never touches any local working dir. The runtime IPC handler uses
  the async variant + `BackgroundTaskStore`.
- `gitPullRemoteIntoCurrent(projectDir, remote, branch)` — fetches
  `<remote>/<branch>` then merges it into the current branch of
  `projectDir`. Fetch failure short-circuits the merge.

### Cross-worktree push: deliberately unsupported

There is no helper that pushes the active stream's commits *into*
another worktree's branch. Every available path mutates the other
worktree:

- `git push <other-worktree-path> <branch>` is refused by default for
  the currently-checked-out branch (`receive.denyCurrentBranch`).
- `git merge` / `git pull` inside the other worktree obviously
  mutates its working dir.
- `git update-ref` from our side advances the ref but leaves the
  other worktree's HEAD/index/working tree divergent — it then
  silently appears "dirty".

The supported direction is the inverse: from the other stream, the
Git Dashboard's worktrees card lists *our* branch with a
"Merge into current" action so a human in that stream pulls our
commits in safely. Tests pin this invariant: the gitMerge sibling-
worktree test in `src/git/git.test.ts` asserts byte-equal HEAD,
status, and file content on the sibling after merging *its* branch
into the primary.

`isGitRepo` requires the project root *itself* to be the git toplevel —
nested git repos and parent-dir lookups are explicitly refused (see
`architecture.md`'s "Workspace isolation rule"). `isGitWorktree` rejects
secondary worktrees so oxplow won't try to nest its own worktrees inside
another tool's checkout.

## UI commit affordance

The Files panel (`ProjectPanel`) shows a **Commit (N)** button in its
header toolbar whenever `gitEnabled && uncommittedPaths.length > 0`.
Clicking it opens a small `CommitDialog` with a commit-message
textarea; submitting runs `gitCommitAll` through a dedicated
`oxplow:gitCommitAll` IPC method. This is the UI entry point for
user-driven commits. The agent doesn't drive commits — the Stop-hook
emits no commit directives.

Button carries `data-testid="files-commit"`; the dialog's message
textarea is `files-commit-message` and the submit button is
`files-commit-submit`.

### Non-writer threads still cannot call git

`NON_WRITER_PROMPT_BLOCK` (`src/electron/write-guard.ts`) explicitly
forbids git mutations for non-writer threads — they share the
worktree with the writer and any ref/index change corrupts the
writer's in-progress work. The write-guard hook denies Write/Edit/
MultiEdit/NotebookEdit in those threads, and the prompt block covers
Bash (which the hook can't classify reliably).

## Related

- [data-model.md](./data-model.md) — schema overview.
- [agent-model.md](./agent-model.md) — Stop-hook pipeline (no commit
  branches; commits are user-driven).
- [editor-and-monaco.md](./editor-and-monaco.md) — blame overlay UI.
