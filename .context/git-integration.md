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
- `runtime.recordFsWatchChange` to attribute writes to batches in
  `batch_file_change`.

Source files mutate constantly; this watcher's job is to keep the
file-tree current and to log changes for the batch.

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

When the stream lives in a secondary worktree (the common case — newde
manages its own worktrees under `.newde/worktrees/`), the stream's
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
  `git commit -m message`, returning the new sha. The Files-commit
  dialog defaults the include-untracked toggle OFF and only renders the
  checkbox when the workspace has untracked files; the orchestrator's
  commit-point commit passes `{ includeUntracked: true }` to keep
  its historical behaviour. Used by `batchQueue.executeCommit`, which
  is called synchronously from the `newde__commit` MCP tool after the
  user approves the drafted message in chat.
- `listBranchChanges`, `getGitLog`, `getCommitDetail`, `getChangeScopes`,
  `searchWorkspaceText`, `restorePath`, `addPath`, `appendToGitignore`,
  `gitPush`, `gitPull`, `listFileCommits`, `listAllRefs`,
  `readFileAtRef`, `listGitStatuses` — straight `execFileSync` wrappers
  exposed via IPC for UI consumption.

`isGitRepo` requires the project root *itself* to be the git toplevel —
nested git repos and parent-dir lookups are explicitly refused (see
`architecture.md`'s "Workspace isolation rule"). `isGitWorktree` rejects
secondary worktrees so newde won't try to nest its own worktrees inside
another tool's checkout.

## UI commit affordance

The Files panel (`ProjectPanel`) shows a **Commit (N)** button in its
header toolbar whenever `gitEnabled && uncommittedPaths.length > 0`.
Clicking it opens a small `CommitDialog` with a commit-message
textarea; submitting runs `gitCommitAll` through a dedicated
`newde:gitCommitAll` IPC method. This is the UI entry point for the
**ad-hoc commit path** (the path `agent-model.md` distinguishes from
commit points). The commit-point flow still owns automated batches;
the button is for "I've got changes and want to land them now."

Button carries `data-testid="files-commit"`; the dialog's message
textarea is `files-commit-message` and the submit button is
`files-commit-submit`.

## Two commit paths

newde supports two paths for landing a commit, and they exist for
different reasons:

1. **Ad-hoc.** The writer-batch agent runs `git add` / `git commit`
   directly via Bash when the user tells it to commit. No commit
   point, no approval UI, no `propose_commit`. This is the default
   shape for "I've got changes, land them now."
2. **Commit-point / approval.** A `commit_point` row exists in the
   queue and the Stop-hook has blocked the agent with a directive
   telling it to call `mcp__newde__propose_commit`. The agent drafts a
   message, the user approves in chat, and `mcp__newde__commit` (or
   auto-mode) runs `gitCommitAll` through the runtime. This path
   records a commit sha on the commit_point row for provenance.

`propose_commit` is **only** for path 2 — don't ask the agent to
propose when no commit point is pending.

### Non-writer batches still cannot call git

`NON_WRITER_PROMPT_BLOCK` (`src/electron/write-guard.ts`) explicitly
forbids git mutations for non-writer batches — they share the
worktree with the writer and any ref/index change corrupts the
writer's in-progress work. The write-guard hook denies Write/Edit/
MultiEdit/NotebookEdit in those batches, and the prompt block covers
Bash (which the hook can't classify reliably).

## Related

- [data-model.md](./data-model.md) — commit_point and wait_point tables.
- [agent-model.md](./agent-model.md) — how the Stop-hook pipeline asks
  the agent to propose commits.
- [editor-and-monaco.md](./editor-and-monaco.md) — blame overlay UI.
