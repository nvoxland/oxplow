# Git integration


What this doc covers: the three filesystem watchers that keep git state
fresh in the UI, the runtime-side git operations, and the rule that
agents never call `git` directly. For the data side of commits (commit
points), see [data-model.md](./data-model.md) and
[agent-model.md](./agent-model.md).

## Three watchers

The runtime keeps three independent `fs.watch`-based watchers running.
Each cares about a different slice of the project state.

### Immediate vs. debounced (the `FsWatcher` contract)

`oxplow_fs_watch::FsWatcher` fires events **immediately** — there is no
built-in debounce anymore. Consumers pick the view they need:

- `subscribe()` — the raw, un-coalesced stream. Every OS-level event
  flows through as it arrives. **Snapshot capture uses this** so its
  in-memory dirty set is always current the instant a snapshot is
  requested (the old 250ms debounce let an edit's watch event lag the
  effort-completion snapshot, so genuinely-edited files showed up as
  `claimed_but_not_changed` in the effort file-review).
- `subscribe_debounced(window)` — a coalescing listener that batches a
  burst into at most one event per `(path, kind)` pair per window. This
  is what the general-purpose, UI-facing consumers watch (workspace,
  wiki, git-context, git-refs) so a `git checkout` or save-storm fires
  one round of notifications instead of one per touched file.

So debounce is a per-consumer choice, not a property of the watcher.

**Backend debounce doesn't coalesce across streams, and doesn't bound
concurrency.** Each watcher debounces its own stream, so a consumer
subscribing to *two* of them still gets two callbacks for one user action
(a `git commit` trips both the gitRefs and workspace windows). And a
debounce says nothing about how long the resulting work takes — if it
outruns the window, calls overlap.

That bit the branch-changes summary in `App.tsx` (tsk238): it subscribes
to both `gitRefsChanged` and `workspaceChanged`, and each refresh runs
`listBranchChanges` → 4+ git subprocesses including a
`status --untracked-files=all` worktree walk. Agent edit storms drove a
full rescan every 250ms, overlapping.

It now routes both subscriptions through one `coalescedRefresh`
(`apps/desktop/src/coalesced-refresh.ts`) — trailing debounce **plus
single-flight**, with exactly one queued follow-up so the summary can't
end stale. Single-flight is the part that matters: it bounds concurrent
scans to one however slow the scan gets. Reach for it (rather than a bare
`setTimeout` debounce) for any expensive refresh fed by more than one
event stream.

### 1. Workspace watcher

`crates/oxplow-app/src/workspace_watch.rs` — `WorkspaceWatchRegistry`.
One watcher per stream. Rather than registering a single recursive
watch on the worktree root (which would force `notify_debouncer_full`
to walk every subtree — including `target/` and `node_modules/` — at
boot to seed its cache), registration is **scoped**:

- A non-recursive watch on the worktree root, so top-level file
  changes and the appearance/disappearance of top-level dirs still
  fire.
- One recursive watch per top-level directory the **`WorkspaceFilter`**
  doesn't ignore (`filter.ignore(name, true)`).

**Nothing here is a hardcoded path list.** `WorkspaceFilter`
(`crates/oxplow-fs-watch`) is the single source of truth and layers, in
order: `.git` (the only hardcoded segment — `DEFAULT_IGNORED_SEGMENTS`)
and `.oxplow` defaults → `generated.include` (forces a path back in) →
`generated.exclude` → **`.gitignore`**, root + nested, with full
hierarchical semantics. `target/` and `node_modules/` are skipped
because a Rust/JS repo gitignores them, not because we name them.

A per-event filter check still runs as defence-in-depth (and to drop
swap/temp files) — it also catches *nested* ignores inside a watched
dir, which a top-level prune can't see. But the meaningful win is at
registration: we never seed cache for dirs we never care about.

Subscribes via the **debounced** view (`subscribe_debounced(250ms)`)
and bridges each batch onto `OxplowEvent::WorkspaceChanged`. Consumed by:

- `ProjectPanel` to refresh the file tree.
- `EditorPane` for external-file-changed prompts.

Source files mutate constantly; this watcher's job is to keep the
file-tree current. Note that the snapshot dirty set is **not** fed from
here — `SnapshotCaptureService` runs its own `FsWatcher` and subscribes
to the *raw* (immediate) stream so the dirty set never lags a snapshot
request (see the immediate-vs-debounced note above).

**The snapshot watcher prunes the same way (tsk206), plus one wrinkle.**
`SnapshotCaptureService::watch_paths_for` builds the identical scoped set
(root non-recursive + non-ignored top-level dirs recursive). It used to
register ONE recursive watch on the project root and filter per delivered
event, which meant every write under `target/` (345k files here) was
delivered and thrown away — any `cargo` build flooded it.

The wrinkle: unlike the workspace watcher, this service's filter can be
**swapped at runtime** by the `set_generated` IPC, whose documented
contract is that include/exclude edits apply *without an app restart*.
Since the watch SET is now derived from the filter, `set_workspace_filter`
also fires a `refilter` notify and the watcher rebuilds its registration —
otherwise a `generated.include` that un-ignores a directory would stay
unwatched until restart. Events in the rebuild gap are dropped; a config
edit is a deliberate user action and the next sweep/event covers it.

**Deriving the watch set from a one-time listing has a second edge, and it
cost real data (tsk227).** The root is non-recursive and the subdir listing
happens once per generation, so a top-level directory created *afterwards*
is covered by nothing: the root reports the `mkdir` itself and then never
reports a single write inside it. Everything under it goes unsnapshotted
until restart — absent from Local History, unrecoverable by
`restore_file_from_snapshot`, invisible to effort attribution. It was found
via that last symptom, and only after the differ and the review logic had
both been wrongly accused; the giveaway was that this repo's own
`tests-e2e/` had **0** `file_snapshot` rows while every pre-existing
directory had hundreds.

So the event loop also rebuilds when `needs_rewatch` sees a filtered-in
directory directly under the root that the generation's registered set
doesn't cover. Re-registering alone is **not** enough — `mkdir d && write
d/f` races the rebuild and no watch reports the gap — so it first calls
`mark_tree_dirty` to walk the new directory into the dirty set. If you
touch this code, keep the backfill: without it the fix silently loses
exactly the files that motivate it.

A recursive root watch would make both edges disappear, and that is what
this used to be. It is not worth it — see the 345k-file flood above. The
cost of the scoped set is that *staleness must be handled explicitly*.

### 2. Git root watcher

A non-recursive `FsWatcher` on `projectDir` itself, set up inline in
`workspace_watch::spawn_project_context`. Listens only for direntry
changes whose filename is `.git`. Non-recursive is sufficient: we only
need to know whether `.git` appears or disappears at the project root,
and a recursive watch here would re-walk the entire `.git` tree on
boot for nothing.

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

`crates/oxplow-git/src/refs_watch.rs` — `GitRefsWatcher`. The
per-stream registry lives in
`crates/oxplow-app/src/workspace_watch.rs`
(`WorkspaceWatchRegistry`), which spawns one `GitRefsWatcher` and one
`FsWatcher` per stream at boot and bridges their broadcasts onto the
shared `EventBus` as `gitRefsChanged` / `workspaceChanged`. Watchers
debounce ~250ms (a single `git commit` fires a dozen events touching
`HEAD`, `refs/*`, `logs/*`, `index`, `ORIG_HEAD`, …).

When the stream lives in a secondary worktree (the common case — oxplow
creates worktrees as siblings of the main repo), the stream's
`.git` is a pointer file, not a directory. The watcher reads the
`gitdir:` line to find the per-worktree state dir (containing `HEAD`,
`index`, `logs/HEAD`) and also follows the `commondir` pointer to watch
the shared `.git` (where `refs/heads/*` actually update). Both dirs are
watched; without the commondir watch, `git fetch` / ref updates from
outside the worktree would be missed.

Fires `gitRefsChanged` after each debounce. Consumed silently (no
loading spinner) by:

- `HistoryPanel` — reloads the commit log.
- `ProjectPanel` — refreshes the indexed git statuses.
- (Formerly `GitChangesPanel`, now folded into `ProjectPanel`'s filter
  modes.)
- `SnapshotCapture::spawn_git_refs_listener` — requests a snapshot for
  the stream, so every commit lands a snapshot row stamped with the new
  HEAD (`snapshot.git_commit`/`git_branch`) even when the worktree
  didn't change. Beyond Local History, those rows are the **anchor
  points for metric ancestry** (tsk97/tsk102): a dirty test run's code
  is placed by the *next* same-branch commit-stamped snapshot — the
  commit that absorbed it, not the fork point its `closest_git_version`
  names. The metric fold partitions per `(stream, branch)` and its
  cross-branch visibility rule (`metric_visibility.rs`) resolves from
  these anchors — see `.context/metrics.md`.

The recursive `fs.watch` falls back to per-subdir watching on platforms
that don't support recursive mode.

### 4. Notes watcher

`crates/oxplow-fs-watch/src/lib.rs` — not really a git watcher, but lives next
to the others because it wraps `fs.watch` the same way. Watches
`.oxplow/wiki/` for `.md` file create/change/delete, debounces
~200ms per slug, and calls `syncNoteFromDisk` → `WikiPageStore.upsert`
(or `deleteBySlug`). Captures current HEAD (`readWorktreeHeadSha`)
and per-reference blob SHA-256 hashes as the freshness baseline.

Every write is treated identically — agent and user edits both
re-baseline freshness — so the watcher is the single sync path for
`wiki_page` metadata. See `data-model.md` → `wiki_page`.

### 5. Config watcher

`crates/oxplow-app/src/config_watch.rs` — `ConfigWatcher`. A
non-recursive `FsWatcher` on `projectDir`, spawned once at boot from
`main.rs` (held via `Box::leak` for the process). On a debounced event
whose basename is `.oxplow/project.yaml`, it calls
`Services::reload_config_from_disk`, which re-runs
`load_project_config`, swaps the in-memory `Arc<RwLock<OxplowConfig>>`,
re-applies the snapshot `WorkspaceFilter` (mirroring `set_generated`),
and emits `OxplowEvent::ConfigChanged`. Exists because config is
otherwise read only once at boot — without it, an out-of-band edit
(notably the agent running `/oxplow:configure`, which Writes a
`collection:` block) wouldn't take effect until restart. The IPC
setters (`set_generated`, `set_agent_prompt_append`) still mutate the
in-memory config directly; this watcher covers every other edit path.

### Orphan detection (boot + runtime)

`WorkspaceWatchRegistry::spawn` checks `worktree_path.exists()` before
spawning a stream's watchers. If a non-primary stream's worktree was
deleted out from under us while oxplow was offline (e.g. external
`rm -rf`, `git worktree remove`), the registry calls
`StreamService::archive_stream(id, false)` to take the row out of the
rail and emits `OxplowEvent::StreamOrphaned { stream_id, title }` so
the renderer can toast ("Stream X was closed: its worktree directory
was deleted."). Primary streams are exempt — a missing project root
is a different failure mode (the daemon shouldn't have booted).

Ongoing detection works the same way: each per-stream fs watcher
holds a one-shot `OnOrphan` callback, and on every event it cheaply
re-checks `worktree_path.exists()`. If the root is gone, the callback
runs the same archive + emit path and the watcher loop exits (it can't
do anything useful anyway). The check is on every event, not just
`Removed`, because macOS FSEvents surfaces a directory's own deletion
as an `Updated` event of its parent — keying on the kind would miss
the case the user actually cares about.

### Why three

They watch overlapping but disjoint things:

- workspace = source files (excluding `.git`)
- root watcher = appearance/disappearance of `.git`
- refs watcher = mutations *inside* `.git`

A single recursive watcher on the root would lump them together and
either spam the UI on every internal git op or miss external changes
that don't touch source files.

### Boot is async

`WorkspaceWatchRegistry::spawn` and `WikiPagesWatcher::spawn` run as
background tasks reported through `BackgroundTaskStore` (kinds `Git`
and `NotesResync`). The desktop boot path does not block on either —
the renderer paints first, and the `BackgroundTaskIndicator` shows
"Starting workspace watchers" / "Initial wiki pages scan" rows until
each scan settles. Filesystem events start arriving once the cache
walk completes.

## GitService — the singleton

Every read of git state and every mutating git op routes through
`oxplow_app::git_service::GitService`, held on `Services` as
`Arc<GitService>`. One per app, not per stream.

**It is a thin facade.** Every read shells out live via
`tokio::task::spawn_blocking(oxplow_git::*)`; every write delegates to
the matching `oxplow_git::*` op and then emits the renderer-facing
`OxplowEvent`. There is no shared mutable cache. The only state the
service keeps is a `HashMap<StreamId, PathBuf>` routing table
(`worktrees`) maintained by `register/deregister`.

### Why no cache

The previous design cached statuses / branches / log / ahead-behind /
remote-branches and **subscribed to its own invalidation triggers**
(`WorkspaceChanged` / `GitRefsChanged`). Subscribers on the same
broadcast channel have no ordering guarantees, so any other consumer
of those events that read from the GitService cache could land on the
pre-event snapshot before the invalidation hop ran. That race silently
broke snapshot capture's commit-record path.

The wrapped `oxplow_git::*` ops are sub-10ms libgit2 calls. The cache
wasn't worth the correctness cost. If a future profile shows a real
hotspot, **add caching inside the facade** (per-method memo, request
coalescer, whatever) — never let cached state leak through the API.
Callers must not be able to tell whether anything is cached.

### Lifecycle hooks

`GitService::register(stream_id, worktree)` and `deregister(stream_id)`
are called from the stream lifecycle commands (`create_worktree`,
`adopt_worktree`, `delete_stream`, `archive_stream`) so the routing
table stays in sync with the stream list. At boot, `GitService::spawn`
seeds itself from `streams.list()` asynchronously — readers against
unseeded streams fall back to the project root via `resolve_repo_dir`.

A small bus listener subscribes to `GitRefsChanged` for one purpose
only: re-running `reconcile_branch` so the per-stream `branch` field
in the stream record follows the live HEAD. That's persistent state
in the stream record (used by the bottom-bar branch chip and agent
prompts), not a cache.

### Mutating ops emit events

`commit_all`, `add_path`, `restore_path`, `fetch`, `pull`,
`pull_remote_into_current`, `push`, `push_current_to`, `merge`,
`rebase`, `rename_branch`, `delete_branch`, `append_to_gitignore`,
plus the `*_workspace_*` write ops, all pass through to `oxplow_git::*`
and emit `OxplowEvent::WorkspaceChanged` (always) plus
`GitRefsChanged` (when the op may have moved HEAD or any ref).
Subscribers refetch on receipt; no cache is being invalidated because
there is no cache.

### Stream-scoped destructive ops require a resolvable stream

Most reads resolve their worktree via `resolve_repo_dir(stream_id)`,
which treats an absent or unparseable `stream_id` as "use the project
root" (the primary worktree). For **destructive** stream-scoped ops —
`commit_all`, `merge`, `rebase` — that silent fallback is a footgun:
a caller that meant stream B but sent a field that didn't bind (e.g.
snake_case `stream_id` where the wire field is camelCase `streamId`,
arriving as `None`) would run the op against the PRIMARY worktree and
get a misleading `{"success":true,"stdout":"Already up to date."}` on
the wrong branch.

So those three ops resolve via `resolve_stream_worktree_strict`
instead, which **errors** when `stream_id` is absent, syntactically
invalid, or names an unknown stream — never falling back to primary.
The UI is unaffected (its `api.ts` wrappers always pass a concrete
stream id, and the primary stream is itself a registered stream row);
the guard exists for MCP/scripted/future callers. To run one of these
ops against the primary worktree, pass the primary stream's id
explicitly — `None` is rejected on purpose.

### Smart conflict auto-resolution (the IntelliJ magic-wand pass)

After a long-running git op leaves conflicts, `GitService::merge`,
`rebase`, `cherry_pick`, and `revert` run a **smart-merge pass** via the
shared `with_auto_resolve` helper (only when the git op reported
`!success`): `oxplow_git::auto_resolve_conflicts(worktree)`
(`crates/oxplow-git/src/smart_merge.rs`). The number of files it
cleanly resolved is folded into `GitOpResult.auto_resolved` so the UI
can report "N conflicts auto-resolved". The pass is
**operation-agnostic** — it reads whatever unmerged paths sit in the
index regardless of which op produced them, and only resolves the
current step's conflicts; it never `--continue`s a paused
rebase/cherry-pick (the user/UI drives continuation, per the usability
rules). All four — `merge` / `rebase` / `cherry_pick` / `revert` — are
now fully wired: each has an `oxplow_git` op, a `GitService` method, an
`oxplow-rpc` core (`git_merge_into` / `git_rebase_onto` /
`git_cherry_pick` / `git_revert`) registered in the `rpc_dispatch!`
registry (which also generates the Tauri adapter) and a generated FE
binding. The
cherry-pick / revert UI entry point lives on the **commit page**
(`GitCommitPage`): two `InlineConfirm` action buttons in the commit
metadata card (`data-testid` `commit-actions`, triggers
`commit-cherry-pick` / `commit-revert`) run against the active stream's
worktree and fold the `auto_resolved` count into the success toast via
`gitOpOutcomeMessage` (`apps/desktop/src/git-op.ts`). Both are
destructive working-tree mutations, so they confirm inline per
[usability.md](./usability.md); failures record an op-error and offer a
"Show details" toast.

Why it exists: git's merge driver is **line-based**, so two edits to
*different words on the same line* (or both sides adding a different
import) collide in the same line-block and are reported as a conflict
even though they don't overlap. This is exactly what IntelliJ's "magic
wand / resolve simple conflicts" fixes by comparing at word
granularity. We reproduce it with a **token-level diff3**:

- `tokenize(s)` splits into word runs / whitespace runs / individual
  newlines / single punctuation chars. Lossless: `tokenize(s).concat()
  == s`.
- `merge3(base, ours, theirs)` runs a classic diff3 over the token
  slices (via `similar`'s Myers diff), clustering overlapping change
  regions. It returns `Ok(tokens)` only when every region is
  unambiguous, else `Err(Conflicted)`.

`auto_resolve_conflicts` reads each unmerged path's three stages
directly from the git2 index (`Index::conflicts()` →
ancestor/our/their blobs — no `git show :1:` shelling), and **only
modify/modify** conflicts (all three stages present) that are UTF-8
text under 1 MiB are eligible. For each, it runs `merge3_str`; on `Ok`
it writes the merged file and stages it with `add_path` (slot-0 add
clears the unmerged stages). On `Err` — or for add/add, delete/modify,
binary, oversized — it leaves git's markers untouched.

**Safety model (never auto-resolve a true overlap).** A divergent
region where ours and theirs both changed the same base tokens
differently (including delete-vs-modify and add/add of different text
at the same point) is `Err(Conflicted)`, so the file is left exactly as
git produced it. Tokenization is lossless, and git has already failed
its line merge by the time we run, so the pass can only ever *reduce*
the conflict count, never introduce new content. The merge stays
in-progress (MERGE_HEAD intact) with the resolved files staged — the
result is a normal, reviewable working-tree change the user commits.
`conflicted_count` (rail HUD) drops naturally as paths are staged.

Tier-1 is language-agnostic (token-level). A future Tier-2 would add a
tree-sitter AST merge (Mergiraf-style) for the highest-value commutative
cases; note Mergiraf itself is GPLv3 vs oxplow's MIT, so it can only be
invoked as a separate binary, never linked as a library.

## Runtime git operations

All git invocations go through `crates/oxplow-git/src/lib.rs`. Notable:

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
- `getCommitDetail(repo, sha)` (`src/log.rs`) resolves **both full and
  abbreviated** shas — Activity-feed commit links carry 7-char prefixes.
  Gotcha: `git2::Oid::from_str` zero-pads any ≤40-char hex string into a
  syntactically-valid-but-**nonexistent** OID and returns `Ok`, so it can
  never resolve an abbreviation. Trust it only for `sha.len() == 40`; route
  everything shorter through `repo.revparse_single`, which expands against
  the object DB. Same rule applies anywhere else a sha is turned into an OID.
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
- `compute_divergence(repo_path, base, head)` (`src/divergence.rs`) —
  cross-stream merge-readiness. Returns `Divergence { ahead, behind,
  overlapping_files, readiness }`. `ahead`/`behind` come from
  `graph_ahead_behind(head, base)`; `overlapping_files` is the set of
  paths changed on **both** sides since `merge_base(base, head)` (a
  file-overlap heuristic — it names the files a line-level merge could
  collide on, without running a trial merge). `readiness` is
  `AlreadyIntegrated` (head has no commits beyond base), `Clean` (ahead,
  no overlap), or `Conflict` (ahead, overlap). Any lookup failure
  (unresolvable branch, etc.) degrades to `AlreadyIntegrated` zeros so a
  bad row never breaks the dashboard. Exposed via
  `GitService::divergence` → the `list_stream_divergences` command (one
  row per stream vs the detected default branch), consumed by the Git
  Dashboard's "Merge readiness" card.
- `tree_at_commit(repo, rev)` / `diff_commits(repo, a, b)`
  (`src/tree.rs`) — a libgit2 tree walk that yields `path -> blob oid`
  and runs it through the **shared** `oxplow_domain::diff_trees`
  comparison (the same primitive `SqliteSnapshotStore::diff_snapshots`
  uses for snapshots). This is the source-agnostic content diff:
  before/after can come from two git commits or two snapshots and go
  through one comparison instead of `git diff`. It's a content-identity
  diff (added/modified/deleted); **rename detection is intentionally
  not done** here — views that need renames still use git's own diff.
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
worktree test in `crates/oxplow-git/src/lib.rs` (`#[cfg(test)] mod tests`) asserts byte-equal HEAD,
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

### Commits are not policed (tsk250)

Neither hook comments on a commit. The Stop hook emits no commit
directives, and the PostToolUse Bash hook watches `git commit` only to run
the revert/token-waste leg — the **commit-hygiene nudge** that used to flag
committed files outside the open effort's changed set was removed. It
second-guessed a decision the committing actor had already made (nothing to
disentangle: one commit, one actor), and it hardcoded a `docs/` warning
about this repo's own auto-deploy workflow, which oxplow can't assume of any
project. Rationale in [collection.md](./collection.md) → "Commits get no
nudge of their own".

### Non-writer threads still cannot call git

`NON_WRITER_PROMPT_BLOCK` (`crates/oxplow-runtime/src/write_guard.rs`) explicitly
forbids git mutations for non-writer threads — they share the
worktree with the writer and any ref/index change corrupts the
writer's in-progress work. The write-guard hook denies Write/Edit/
MultiEdit/NotebookEdit in those threads, and the prompt block covers
Bash (which the hook can't classify reliably).

## Commit indexer

`crates/oxplow-app/src/commit_indexer.rs` walks the most-recent
`DEFAULT_INDEX_DEPTH` (500) commits reachable from HEAD and projects
each one into the unified `page_ref` graph (see
[data-model.md](./data-model.md)):

- Diff against parent#0 → one `(git-commit:<sha>) -- touched_file -->
  (file:<path>)` edge per file.
- Subject + body run through `oxplow_domain::refs::extract` so the
  same wikilink + inline-mention rules used by wiki bodies and
  task descriptions also apply to commit messages
  (`wi-…`, `[[architecture]]`, `finding:fnd-1`, bare 7-40 hex shas).

Idempotent. Each commit is keyed by its full sha, and a one-row
existence probe before re-diffing skips already-indexed commits, so
repeated scans are cheap. No separate cursor table.

The boot path runs the initial scan in a detached task. The same
function is re-run on every `OxplowEvent::GitRefsChanged` (debounced
by `GitRefsWatcher` upstream), which catches new commits whether
they came from the in-app commit affordance or an external
`git commit` in the user's terminal.

## Snapshot capture reacts to HEAD moves

`SnapshotCaptureService::spawn_git_refs_listener` (wired from the
desktop boot in `apps/desktop/src-tauri/src/main.rs`) subscribes to
`OxplowEvent::GitRefsChanged` for its stream. On each event it drains
any pending dirty paths via `request_snapshot(SnapshotSourceKind::GitRefs)`,
then — if the worktree is clean and HEAD's sha differs from the latest
snapshot's `git_commit` — **re-stamps the latest snapshot's
`git_commit` to point at the new HEAD**. No new row is inserted: the
worktree didn't change, so the existing snapshot is already the right
representation of disk; it just now also corresponds to a new commit
(common after `git commit`, `git commit --amend`, or a fast-forward
pull that moves HEAD without altering the working tree).

After the re-stamp the service emits a 0-file
`FileSnapshotsBatchCreated` event so renderer subscribers (Local
History dashboard, change analysis) refetch and pick up the new
`git_commit`.

The cleanliness check uses `oxplow_git::list_git_statuses` directly
(via `spawn_blocking`) rather than going back through GitService.
That's a holdover from when GitService cached statuses and the cache
could race the event; now that the facade is uncached, both paths
return the same data — direct is just one less hop.

## Related

- [data-model.md](./data-model.md) — schema overview, including the
  `page_ref` table the commit indexer writes into.
- [agent-model.md](./agent-model.md) — Stop-hook pipeline (no commit
  branches; commits are user-driven), plus the `list_backlinks` /
  `list_outbound` MCP tools that read the commit indexer's output.
- [editor-and-monaco.md](./editor-and-monaco.md) — blame overlay UI.
