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
- `gitCommitAll(projectDir, message)` — `git add -A && git commit -m
  message` in one helper, returning the new sha. Used by the runtime's
  `executeApprovedCommit` (called whenever a commit point reaches the
  `approved` state, including a startup-recovery pass that drains any
  approved-but-uncommitted points left over from a crash).
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

## Agents never call git directly

A hard rule: the agent's system prompt forbids `git add`, `git commit`,
`git checkout`, etc. The write-guard prompt block
(`NON_WRITER_PROMPT_BLOCK` in `src/electron/write-guard.ts`) lists this
explicitly for non-writer batches; the standard prompt
(`buildBatchAgentPrompt` in `runtime.ts`) reinforces it via the
commit-point flow.

For commits specifically: the agent calls `mcp__newde__propose_commit`
with a drafted message, the runtime stores the proposal, and the runtime
runs `gitCommitAll` either immediately (auto mode) or after user approval
(approval mode). This keeps:

- A consistent provenance trail (commit sha is recorded on the
  commit_point row).
- A single permission boundary (the runtime, not the agent process).
- A natural place to hang the approval UI without having to interrupt
  the agent mid-shell-command.

## Related

- [data-model.md](./data-model.md) — commit_point and wait_point tables.
- [agent-model.md](./agent-model.md) — how the Stop-hook pipeline asks
  the agent to propose commits.
- [editor-and-monaco.md](./editor-and-monaco.md) — blame overlay UI.
