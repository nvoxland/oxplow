# Git integration

What this doc covers: the filesystem watchers that keep VCS state fresh
in the UI, notes on the **git provider** (`oxplow_app::vcs::GitProvider`,
the one place that touches git — `oxplow_git` or libgit2), and the rule
that agents never call `git` directly. Core reaches version control only
through the VCS capability — see [vcs.md](./vcs.md). For the data side
of commits (commit points), see [data-model.md](./data-model.md) and
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
to both `vcsRefsChanged` and `workspaceChanged`, and each refresh runs
`listBranchChanges` → 4+ git subprocesses including a
`status --untracked-files=all` worktree walk. Agent edit storms drove a
full rescan every 250ms, overlapping.

It now routes both subscriptions through one `coalescedRefresh`
(`apps/desktop/src/coalesced-refresh.ts`) — trailing debounce **plus
single-flight**, with exactly one queued follow-up so the summary can't
end stale. And each scan is cheaper (tsk241): `get_change_scopes`
(`oxplow-git/src/branch_changes.rs`) reads the branch, its base and its
upstream in-process through `git2`, the repo opened once, so the one git
process a scan spawns is the `status` — whose untracked walk uses git's
own caches; measured, each `rev-parse` it replaced cost ~8 ms of spawn
against the status's ~10 ms (a test counts the spawns). Single-flight is the part that matters: it bounds concurrent
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
hierarchical semantics, under `.git/info/exclude` and git's global
excludes file (`core.excludesFile`, found as git finds it — tsk1083).
`target/` and `node_modules/` are skipped
because a Rust/JS repo gitignores them, not because we name them.

Two questions, one filter: `ignore` is "should oxplow watch this?" and
`ignore_in_trees` is "would git see it?" (what `Trees` uses: diffs, the
working tree, the Uncommitted panel). They differ only on
`.oxplow/wiki/`, which is watched and snapshotted though git ignores it,
so it is never reported as uncommitted.

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

**Both watchers register through one call: `FsWatcher::watch_workspace(root,
filter)`** (`oxplow-fs-watch`, tsk1051) — the root non-recursively plus each
top-level directory the filter keeps, recursively (`workspace_watch_paths`).
The snapshot service used to register ONE recursive watch on the project
root and filter per delivered event, which meant every write under
`target/` (345k files here) was delivered and thrown away — any `cargo`
build flooded it (tsk206).

The snapshot service's filter can be **swapped at runtime** by the
`set_generated` IPC, whose documented contract is that include/exclude
edits apply *without an app restart*. Since the watch set is derived from
the filter, `set_workspace_filter` also fires a `refilter` notify and the
watcher starts a new `watch_workspace` with the new filter — otherwise a
`generated.include` that un-ignores a directory would stay unwatched until
restart. Events in the rebuild gap are dropped; a config edit is a
deliberate user action and the next sweep/event covers it.

**Deriving the watch set from a one-time listing has a second edge, and it
cost real data (tsk227, tsk1051).** The root is non-recursive and the
subdir listing happens once, so a top-level directory created *afterwards*
is covered by nothing: the root reports the `mkdir` itself and then never
reports a single write inside it. Everything under it went unsnapshotted
until restart — absent from Local History, unrecoverable by
`snapshot.restore_file`, invisible to effort attribution (this repo's own
`tests-e2e/` had **0** `file_snapshot` rows while every pre-existing
directory had hundreds). And the workspace watcher, which had no fix of
its own, never saw a project's first `oxplow/` — the extension catalog
never signalled, so the first kept lens or new extension stayed out of
the launcher.

So `watch_workspace` **follows the tree**: the event of a kept directory
appearing directly under the root goes to a follower thread (registering a
watch from the OS callback thread can deadlock a backend), which adds its
recursive watch and then reports every kept file already inside it, as
`Other` events. The report is **not** optional — `mkdir d && write d/f`
races the new watch and no watch sees the gap — so consumers get the
files that motivated the fix. A directory that disappears is unwatched, so
a later one of the same name is new again. Followed directories are keyed
by name: the OS may report either spelling of the root (`/private/var`
for `/var`).

A recursive root watch would make both edges disappear, and that is what
this used to be. It is not worth it — see the 345k-file flood above. The
cost of the scoped set is that *staleness must be handled explicitly*,
once, in the watcher.

### 2. Project root watcher

A non-recursive `FsWatcher` on `projectDir` itself
(`workspace_watch::spawn_project_context`). Non-recursive is enough: a
repository appears or goes at the root, and a recursive watch would
re-walk its metadata on boot for nothing. It doesn't know what a
repository looks like: after each settled (500 ms) change at the root it
asks the VCS (`Vcs::detect`) whether the project is under version
control, and when the answer changes it emits
`WorkspaceContextChanged { vcs_enabled }`, so UI surfaces (the branch
picker, the stream creation form) enable or disable themselves. The
first paint reads `get_workspace_context` (`vcs_enabled`). Stream refs
watchers are bound at boot per stream; a repository created after boot
is watched from the next start.

This is the only watcher that lives at the project-root level rather
than per-stream.

### 3. Refs watcher

`Vcs::watch_refs(ws, on_change)` — the provider's; git's is
`crates/oxplow-git/src/refs_watch.rs` (`GitRefsWatcher`), which
`GitProvider::watch_refs` bridges to the callback until the returned
guard drops. The per-stream registry lives in
`crates/oxplow-app/src/workspace_watch.rs` (`WorkspaceWatchRegistry`),
which starts one refs watch and one `FsWatcher` per stream at boot and
turns them into `vcsRefsChanged` / `workspaceChanged` on the shared
`EventBus`. Git's watcher debounces ~250ms (a single `git commit` fires a
dozen events touching `HEAD`, `refs/*`, `logs/*`, `index`, `ORIG_HEAD`, …).

When the stream lives in a secondary worktree (the common case — oxplow
creates worktrees as siblings of the main repo), the stream's
`.git` is a pointer file, not a directory. The watcher reads the
`gitdir:` line to find the per-worktree state dir (containing `HEAD`,
`index`, `logs/HEAD`) and also follows the `commondir` pointer to watch
the shared `.git` (where `refs/heads/*` actually update). Both dirs are
watched; without the commondir watch, `git fetch` / ref updates from
outside the worktree would be missed.

Fires `vcsRefsChanged` after each debounce. Consumed silently (no
loading spinner) by:

- `HistoryPanel` — reloads the commit log.
- `ProjectPanel` — refreshes the indexed git statuses.
- (Formerly `GitChangesPanel`, now folded into `ProjectPanel`'s filter
  modes.)
- `SnapshotCapture::spawn_git_refs_listener` — requests a snapshot for
  the stream, so every commit lands a snapshot row stamped with the new
  HEAD (`snapshot.revision`/`branch`) even when the worktree
  didn't change. Beyond Local History, those rows are the **anchor
  points for metric ancestry** (tsk97/tsk102): a dirty test run's code
  is placed by the *next* same-branch commit-stamped snapshot — the
  commit that absorbed it, not the fork point its `closest_vcs_rev`
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
(notably the agent running `/oxplow:configure`, which writes a
`testing:` block and report collectors) wouldn't take effect until restart. The IPC
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

## The git provider

`GitProvider` (`crates/oxplow-app/src/vcs/git.rs`) implements the `Vcs`
trait over `oxplow_git`, stateless and path-based, each call under
`spawn_blocking`. Beyond the trait it has git's own operations: the
`git.*` commands' rebase, cherry-pick, revert and `.gitignore`, and the
reads behind the `git_*` RPCs — `change_scopes` (staged / unstaged /
branch changes against base and upstream), `commit_ref_labels` (the
branches and tags at each sha) and `recent_remote_branches`.
`Services.git` is the same provider, typed, for those. The ratchet
`only_the_git_provider_touches_git` (`vcs/mod.rs`) keeps every other
production file off `oxplow_git` and `git2`.

What left the old `GitService` facade (P5.B1–B7), and where it went:

- **Routing** — stream → worktree is `WorktreeRouter`
  (`Services.worktrees`), a memo over the stream store.
- **File I/O and text search** — `WorkspaceFiles`
  (`Services.workspace_files`): list/read/write/create/rename/delete and
  `search_text` (a fixed-string scan over the files the workspace filter
  keeps, the quick-open walk — it was `git grep`); none of it is git.
- **Branch reconciliation** — `BranchReconciler`
  (`Services.branch_reconciler`, spawned at boot).
- **Mutations** — the `vcs.*` / `git.*` bus commands
  (`commands/vcs.rs`, P5.B6).
- **Worktrees** — `Vcs::create_workspace` / `remove_workspace` /
  `detect` (primary or secondary checkout), which `StreamService`
  (`oxplow-session`) calls.
- **Co-change** — `crate::co_change`, over the commit index (below).
- **Ancestry for metric visibility** — `Vcs::revision_graph`, a
  synchronous `RevisionGraph` the visibility oracle caches over.

### Why no cache

The previous design cached statuses / branches / log / ahead-behind /
remote-branches and **subscribed to its own invalidation triggers**
(`WorkspaceChanged` / `VcsRefsChanged`). Subscribers on the same
broadcast channel have no ordering guarantees, so any other consumer
of those events that read from that cache could land on the
pre-event snapshot before the invalidation hop ran. That race silently
broke snapshot capture's commit-record path.

The provider's `oxplow_git::*` ops are sub-10ms libgit2 calls. The cache
wasn't worth the correctness cost. If a future profile shows a real
hotspot, **add caching inside the provider** — never let cached state
leak through the API. Callers must not be able to tell whether anything
is cached.

### Mutations announce what they changed

A `vcs.*` / `git.*` command emits `OxplowEvent::WorkspaceChanged` for
its stream (always) plus `VcsRefsChanged` when it may have moved HEAD
or a ref — for every stream when the refs are shared (fetch, push,
branch rename or delete). `WorkspaceFiles`' writes emit
`WorkspaceChanged` the same way. Subscribers refetch on receipt; no
cache is being invalidated because there is no cache.

### Stream-scoped mutations require a resolvable stream

Reads resolve their worktree via `WorktreeRouter::resolve(stream_id)`,
which treats an absent or unparseable `stream_id` as "use the project
root" (the primary worktree). For a **mutation** that silent fallback is
a footgun: a caller that meant stream B but sent a field that didn't
bind would run a merge against the PRIMARY worktree and get a
misleading "Already up to date." on the wrong branch.

So every VCS command takes a required `stream` and resolves it via
`WorktreeRouter::resolve_strict`, which **errors** when it is invalid or
names an unknown stream — never falling back to primary. To act on the
primary worktree, pass the primary stream's id.

### Smart conflict auto-resolution (the IntelliJ magic-wand pass)

After a git op leaves conflicts, the git provider's `merge`, `pull`,
`rebase`, `cherry_pick` and `revert` run a **smart-merge pass** via
`with_auto_resolve` (`crates/oxplow-app/src/vcs/git.rs`; only when git
reported `!success`): `oxplow_git::auto_resolve_conflicts(worktree)`
(`crates/oxplow-git/src/smart_merge.rs`). The number of files it
cleanly resolved is `OpOutcome.auto_resolved`, and the paths still
conflicted are `OpOutcome.conflicts`, so the UI can report "N conflicts
auto-resolved". The pass is **operation-agnostic** — it reads whatever
unmerged paths sit in the index regardless of which op produced them,
and only resolves the current step's conflicts; it never `--continue`s
a paused rebase/cherry-pick (the user/UI drives continuation, per the
usability rules). The provider's tests in `vcs/git.rs` pin it for a
merge, a rebase and a cherry-pick, and that a true overlap stays
conflicted. The cherry-pick / revert UI entry point lives on the
**commit page** (`GitCommitPage`): two `InlineConfirm` action buttons in
the commit metadata card (`data-testid` `commit-actions`, triggers
`commit-cherry-pick` / `commit-revert`) run `git.cherry_pick` /
`git.revert` against the active stream and fold the `auto_resolved`
count into the success toast via `gitOpOutcomeMessage`
(`apps/desktop/src/git-op.ts`); failures record an op-error and offer a
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

## Provider notes

- `get_commit_detail(repo, sha)` (`src/log.rs`, behind `Vcs::revision`)
  resolves **both full and abbreviated** shas — Activity-feed commit
  links carry 7-char prefixes. Gotcha: `git2::Oid::from_str` zero-pads
  any ≤40-char hex string into a syntactically-valid-but-**nonexistent**
  OID and returns `Ok`, so it can never resolve an abbreviation. Trust it
  only for `sha.len() == 40`; route everything shorter through
  `repo.revparse_single`, which expands against the object DB. Same rule
  applies anywhere else a sha is turned into an OID.
- `vcs.commit` runs `git add -u` (or `git add -A` with
  `include_untracked`) then `git commit -m`. Only the Files panel and the
  uncommitted-changes page run it; no MCP tool commits.
- Push, pull, fetch, merge and rebase are long: the desktop runs each
  command inside a `BackgroundTaskStore` row (`runAsBackgroundTask` in
  `api.ts`) so the bottom-bar `BackgroundTaskIndicator` shows progress.
- History and branch lists read the models (`v_commit`, `v_branch`,
  `v_tag`; P5.B5, [vcs.md](./vcs.md)); ahead/behind, divergence,
  commits-ahead and a file's history stay live on `vcs_divergence`,
  `vcs_revisions_between` and `vcs_file_history`.
- `compute_divergence(repo_path, base, head)` (`src/divergence.rs`,
  behind `Vcs::divergence`) — cross-stream merge-readiness:
  `Divergence { ahead, behind, overlapping_files, readiness }`.
  `ahead`/`behind` come from `graph_ahead_behind(head, base)`;
  `overlapping_files` is the set of paths changed on **both** sides since
  `merge_base(base, head)` (a file-overlap heuristic — it names the files
  a line-level merge could collide on, without running a trial merge).
  `readiness` is `AlreadyIntegrated` (head has no commits beyond base),
  `Clean` (ahead, no overlap), or `Conflict` (ahead, overlap). Any lookup
  failure degrades to `AlreadyIntegrated` zeros so a bad row never
  breaks the dashboard, whose "Merge readiness" card composes one row per
  stream against the default branch (`v_branch.is_default`).
- `tree_at_commit(repo, rev)` / `diff_commits(repo, a, b)`
  (`src/tree.rs`, behind `Vcs::files_at` / `diff`) — a libgit2 tree walk
  yielding `path -> blob oid`, run through the **shared**
  `oxplow_domain::diff_trees` comparison (the primitive
  `SqliteSnapshotStore::diff_snapshots` uses for snapshots), so two
  commits or two snapshots diff through one comparison. It's a
  content-identity diff; **rename detection is intentionally not done**.
- `recent_remote_branches` wraps `git for-each-ref
  --sort=-committerdate refs/remotes` (filters out `<remote>/HEAD`) for
  the dashboard's recent-remote-branches card.
- `vcs.push` / `vcs.pull` with `remote` + `branch` push the current
  branch to `<remote>/<branch>` (`git push <remote> HEAD:refs/heads/<branch>`,
  a refspec push that never touches another working dir) and pull
  `<remote>/<branch>` into it (fetch, then merge; a failed fetch
  short-circuits the merge). The dashboard's remote-branches card runs
  both.
- `remove_workspace` is `git worktree remove --force` then `git worktree
  prune`: a removal that fails (a directory already gone, a locked
  worktree) leaves a stale admin entry in `.git/worktrees/` that a later
  `worktree add` of the same path trips on, and prune clears it.

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

The git provider's `detect` (over `oxplow_git::is_git_repo`) requires the
project root *itself* to be the git toplevel —
nested git repos and parent-dir lookups are explicitly refused (see
`architecture.md`'s "Workspace isolation rule"). `isGitWorktree` rejects
secondary worktrees so oxplow won't try to nest its own worktrees inside
another tool's checkout.

## UI commit affordance

The Files panel (`ProjectPanel`) shows a **Commit (N)** button in its
header toolbar whenever `vcsEnabled && uncommittedPaths.length > 0`.
Clicking it opens a small `CommitDialog` with a commit-message
textarea; submitting runs the `vcs.commit` command (`vcsCommit`). This
is the UI entry point for
user-driven commits. oxplow never steers the agent to commit.

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

`crates/oxplow-app/src/commit_indexer.rs` walks the commits reachable
from **every stream's** head as deep as `IndexDepth::DEFAULT` says — a
window of 500, doubled while the oldest it read was new (a pull longer
than the window is indexed whole, no gap), up to 5000, which is also the
horizon: a first boot indexes the newest 5000 (tsk568) — through the VCS capability (`Vcs::log`, `Vcs::revision`;
[vcs.md](./vcs.md)) — stores them (`v_commit`, `v_commit_file`),
restates branches and tags (`v_branch`, `v_tag`), and projects each
commit into the unified `page_ref` graph (see
[data-model.md](./data-model.md)):

- Diff against parent#0 → one `(commit:<sha>) -- touched_file -->
  (file:<path>)` edge per file.
- Subject + body run through `oxplow_domain::refs::extract` so the
  same wikilink + inline-mention rules used by wiki bodies and
  task descriptions also apply to commit messages
  (`wi-…`, `[[architecture]]`, `finding:fnd-1`, bare 7-40 hex shas).
- **Linked to the task by its work** (`commit_links.rs`, tsk1035). An
  agent's commit rarely names its task (and agents name tasks by title),
  so each new commit is checked against the efforts that closed in the
  two weeks before it: it **holds** an effort's work when they share a
  file and the commit's object for every shared file is the object the
  effort's end-snapshot bytes would be (`Vcs::object_at` against
  `ObjectStore::id_of`; a partial commit holds it too). It's linked by a
  `(work_item:<id>) -- committed --> (commit:<sha>)` edge — the shape of
  a declared impact, the effort in `source_extra` — which `v_commit_task`
  (v2) and the commit's backlinks read. The other order (committed, then
  the task closed) is the `effort.commits` reaction to `effort.finished`,
  over the commits since the effort started. An effort whose end-snapshot
  bytes have expired from Local History (or were never kept) can't be
  shown to hold anything, so it simply doesn't match; it must never error,
  or one old effort stops the commit linking to every other (tsk1078).

Idempotent. Each commit is keyed by its full sha, and a one-row
existence probe before re-diffing skips already-indexed commits, so
repeated scans are cheap. No separate cursor table.

The index also feeds **co-change** (`crate::co_change`): change analysis
aggregates the last 180 days of `git_commit_file`
(`SqliteGitStore::changesets_since`) into which files usually move
together and when each was last touched, and flags a change's files
whose usual co-changers are missing or that were long dormant. The
history is cached until the index changes (its size or newest commit).
The index spans every stream's head, up to 5000 commits deep from a
first boot (the depth the old primary-only walk had).

The boot path runs the initial scan in a detached task. The same
function is re-run on every ref move (`Services.ref_moves`, the VCS
watcher's own channel — not the in-memory event bus, P7.B6; debounced
by the refs watch upstream), which catches new commits whether
they came from the in-app commit affordance or an external
`git commit` in the user's terminal.

## Snapshot capture reacts to HEAD moves

`SnapshotCaptureService::spawn_git_refs_listener` (wired from the
desktop boot in `apps/desktop/src-tauri/src/main.rs`) subscribes to
the ref moves (`RefMoves`) of its stream. On each event it drains
any pending dirty paths via `request_snapshot(SnapshotSourceKind::GitRefs)`,
then — if the worktree is clean and HEAD differs from the latest
snapshot's `revision` — **re-stamps the latest snapshot's
`revision` to point at the new HEAD** (`git:<sha>`). No new row is inserted: the
worktree didn't change, so the existing snapshot is already the right
representation of disk; it just now also corresponds to a new commit
(common after `git commit`, `git commit --amend`, or a fast-forward
pull that moves HEAD without altering the working tree).

After the re-stamp the service emits a 0-file
`FileSnapshotsBatchCreated` event so renderer subscribers (Local
History dashboard, change analysis) refetch and pick up the new
`revision`.

The cleanliness check and the head read go through the VCS capability
(`Vcs::status` / `Vcs::head`, `SnapshotCaptureService::clean_head`,
`.context/vcs.md`).

## Related

- [vcs.md](./vcs.md) — the VCS capability core reads git through.
- [data-model.md](./data-model.md) — schema overview, including the
  `page_ref` table the commit indexer writes into.
- [agent-model.md](./agent-model.md) — commits are user-driven; the `list_backlinks` /
  `list_outbound` MCP tools that read the commit indexer's output.
- [editor-and-monaco.md](./editor-and-monaco.md) — blame overlay UI.
