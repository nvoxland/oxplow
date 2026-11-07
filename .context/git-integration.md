# Git integration

What this doc covers: the file watching that keeps VCS state fresh
in the UI, notes on the **git provider** (`oxplow_app::vcs::GitProvider`,
the one place that touches git — `oxplow_git` or libgit2), and the rule
that agents never call `git` directly. Core reaches version control only
through the VCS capability — see [vcs.md](./vcs.md). For the data side
of commits and their work links, see [data-model.md](./data-model.md) and
[agent-model.md](./agent-model.md).

## One watch per stream

Each stream has **one** `FsWatcher` on its worktree, owned by
`StreamWatchRegistry` (`crates/oxplow-app/src/stream_watch.rs`), which starts it
when the stream appears (boot, `oxplow.stream.create`) and ends it, and everything
hung off it, when the stream goes (archive, orphan). Off it hang the snapshot
capture, the workspace announcements and the VCS's refs watch. What follows a
few paths of the primary worktree subscribes to that same watch with a `Scope`,
which names paths the filter need not keep: the wiki (`.oxplow/wiki`), the config
(`.oxplow/project.yaml`, the personal file, `.oxplow/extensions`) and version
control appearing (what is directly at the root). The only other watch is the
user-global metrics folder, outside every project.

Boot starts the watches first. What reads the disk at boot (the startup sweep,
the wiki scan) waits for them, so a change is in what it read or heard after.

Every stream then sweeps, one at a time with the primary first (`boot::sweep_all`), so
what changed in any worktree while oxplow was off is in a snapshot soon after start.
Each stream's effort-start gate holds until its own sweep ends. A stream made after
boot needs none: nothing can have changed while oxplow was off. A worktree stream's
history is sparse, so its sweep records only real changes (`data-model.md`).

### Batches, and events as hints (the `FsWatcher` contract)

**OS events are hints, not the record.** They come late, and macOS drops them
under load. So a subscriber of `oxplow_fs_watch::FsWatcher` reads a `Subscription`:
a set of what changed (a path written a thousand times is one entry), read when
it likes, that nothing can overflow. A `Batch` holds the changed `paths` and
`rescans`: directories whose changes were not all heard. A subscriber answers
a rescan by checking the disk there. A rescan comes from:

- the OS saying it dropped events (`notify`'s rescan flag; an ignored folder's
  is no one's concern), or a watch error;
- more than 20,000 paths pending for one subscriber (they collapse into their
  top-level directories, `pending.rs`);
- a kept top-level directory appearing, or the rules stopping ignoring one.

| Subscriber | A path | A rescan |
|---|---|---|
| Snapshot capture | dirty | `enqueue_tree_diff(under)`: the startup sweep, limited to a directory |
| Workspace | `WorkspaceChanged`; one `index_moved` per batch | `WorkspaceChanged` with no path; `index_moved` |
| Wiki | `sync_page` | `scan_and_sync_all` |
| Config | reload, or restate sources | both |
| Refs | re-read the state | same |

Each picks when it reads, per consumer:

- `next_batch()`: at once. **Snapshot capture uses this** so its
  in-memory dirty set is always current the instant a snapshot is
  requested (a 250ms wait let an edit's watch event lag the
  effort-completion snapshot, so genuinely-edited files showed up as
  `claimed_but_not_changed` in the effort file-review).
- `next_batch_after(window)`: a burst as one batch. This is what the
  general-purpose, UI-facing consumers use, so a `git checkout` or save-storm
  fires one round of notifications instead of one per touched file.

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

It was fixed with a trailing debounce **plus single-flight** (one queued
follow-up, so the summary can't end stale); the helper (`coalesced-refresh.ts`)
went with the last two views that used it, which are pages now (a page re-runs
the loaders whose reads changed, once per change). Write it again for any new expensive refresh fed by more
than one event stream. And each scan is cheaper (tsk241): `get_change_scopes`
(`oxplow-git/src/branch_changes.rs`) reads the branch, its base and its
upstream in-process through `git2`, the repo opened once, so the one git
process a scan spawns is the `status` — whose untracked walk uses git's
own caches; measured, each `rev-parse` it replaced cost ~8 ms of spawn
against the status's ~10 ms (a test counts the spawns). Single-flight is the part that matters: it bounds concurrent
scans to one however slow the scan gets. Reach for it (rather than a bare
`setTimeout` debounce) for any expensive refresh fed by more than one
event stream.

### How a watch sits on the OS

**Nothing here is a hardcoded path list.** `WorkspaceFilter`
(`crates/oxplow-fs-watch`) is the single source of truth and layers, in
order: `.git` (the only hardcoded segment — `DEFAULT_IGNORED_SEGMENTS`)
and `.oxplow` defaults → `generated.include` (forces a path back in) →
`generated.exclude` → **`.gitignore`**, root + nested, with full
hierarchical semantics, under the exclude file and git's global
excludes file (`core.excludesFile`, found as git finds it).
`target/` and `node_modules/` are skipped
because a Rust/JS repo gitignores them, not because we name them.

Two questions, one filter: `ignore` is "should oxplow watch this?" and
`ignore_in_trees` is "would git see it?" (what `Trees` uses: diffs, the
working tree, the Uncommitted panel). They differ only on
`.oxplow/wiki/`, which is watched and snapshotted though git ignores it,
so it is never reported as uncommitted.

`FsWatcher::watch_workspace` registers the way the OS works (`Registration`):

- **One recursive watch on the root** (macOS, Windows). FSEvents reports
  everything under a path it is given, so pruning at registration saves nothing
  (a non-recursive root in the list still delivered all of `target/`, thrown
  away per event), and a folder that appears needs nothing from us.
- **One watch per kept directory, at any depth** (Linux). inotify has no recursive
  watch and a limit on how many it holds, so what is ignored costs nothing. A
  follower thread (registering from the OS callback thread can deadlock a
  backend) covers a kept directory that appears, or that a rule change stops
  ignoring. `mkdir d && write d/f` races the new watch and no watch sees the gap,
  so each such directory also gets a rescan, which is **not** optional: without
  one, a folder created after boot went unsnapshotted until restart. Past the kernel's limit, what is left is listed
  (`FsWatcher::uncovered`, with the `sysctl` to raise it in the log) and each
  snapshot take looks at the disk there. Slower, and correct.

Registering is one batch (`paths_mut`). On macOS `notify` stops and recreates the
whole FSEvents stream on each `watch` call, and each stop also calls `fseventsd`.
One call per top-level directory took about 7.5 s per watch on this repo and on
metabase (measured under load 10; a batch takes 30 ms to 1 s), for every watch of
every oxplow process on the machine.

**A subscriber hears a path in the root's own spelling.** The OS may use another
(macOS reports `/private/var` for `/var`, and a symlink's target); the router
rewrites it, so what a subscriber matches against its root matches.

**Ignored paths are dropped at the watch.** The router judges each path by the
filter before it passes it on (a file under an ignored directory is ignored with
it, answered from a remembered lookup with no `stat`: `ignore_change`), so a
build writing thousands of files costs next to nothing. Only a `Scope` naming a
path hears it anyway.

**An ignore-file edit takes effect.** `WorkspaceFilter` is a handle: every
clone is one filter, and it never walks the tree. A directory's `.gitignore`
is read the first time a path in it is judged and kept until the watch notes
an edit (`note_ignore_file`, which forgets that directory's rules), so what is
written next is judged by the new ones. A filter costs what is judged, not the
size of the tree. Git's exclude file counts too: a linked worktree reads its
main checkout's (`exclude_file`). What an edit stopped ignoring no one has heard
of, so it also asks for a rescan of the directory it rules (a nested `.gitignore`
its own; the root's or the exclude file, the root). An ignore file inside an
ignored folder (`target/.gitignore`) rules nothing. Each stream has its own
filter, rooted at its own worktree; its capture and its watch hold the same one,
and a `generated` change reaches them all (`set_generated`, then
`FsWatcher::refilter`, which rescans the root).

### Workspace announcements

Reads batches after a burst (`next_batch_after(250ms)`) and bridges each onto
one `OxplowEvent::WorkspaceChanged { paths, overflow }`: up to 200 paths, or
`overflow` when they are more than that or changes were lost. Consumed by:

- The Directory page's loaders (`v_file` follows `WorkspaceChanged`).
- `EditorPane` for external-file-changed prompts: each named path, or every
  buffer it holds on `overflow`.

**The last hop says when it lost something too.** The bus holds 256 events. A
socket that falls behind it has lost frames, so the daemon sends a `resync`
frame and the UI runs its reconnect handlers (`scheduleResync`, once for a
burst).

**This logic is tested with no OS.** `oxplow-fs-watch/src/router.rs` holds
what happens to an event (`Router::handle`, on the OS callback thread) and
what moves the watches (`Follower`, over the small `Registrar` trait), and
its tests hand them made-up events. `Feed` does the same for a subscriber. Only
`oxplow-fs-watch`'s top-level `tests` and one `stream_watch` test touch the real
OS: a write is heard, non-recursive holds, drop cancels, a new folder's file is
found, an edit reaches the working-files listener.

### A take's marker

A snapshot take must hold every edit made before it. Events take from 10 ms on
a calm machine to seconds under load, so no fixed wait is right. So a take writes
a marker file (`.oxplow/.take-marker-<n>`) and waits for it to come back through
the capture's own subscription (`wait_for_marker`): the OS reports changes in
order, and a batch is handled whole with the marker last, so once it is back
everything written before it is in the dirty set. It waits 2 s at most
(`DEFAULT_MARKER_WAIT`): past that the watch has stalled, and what is late reaches
the next take. With no watch running, a fixed 300 ms stands in
(`DEFAULT_PREDRAIN_DELAY`). The filter ignores the marker, so it is in no snapshot
and no announcement; a `Scope` names it for the capture alone.

### The working-files list

Every burst of edits relists the stream's working files
(`change_reactor::watch_working_files`, one run at a time), so what a relist costs
matters more than any watch. A full `Trees::diff(HEAD, Working)` lists the head's
tree and walks and stats the whole tree: 42–57 s at 500,000 files. So
`Trees::diff_from_status` looks only at the paths git status names (`entries`, and
`moved` for a rename's other end), one `object_at` each. It gives what the full
diff gives (`a_diff_from_status_is_what_a_full_diff_says`) and falls back to it for
a base other than the head, a `generated.include` (it keeps files git ignores, so
git doesn't report them), or more than 5,000 paths. A range headed at the working
tree still takes the full diff.

### Version control appearing

A `Scope` on the primary's watch: what is directly at the root
(`stream_watch::follow_project_context`). It doesn't know what a repository looks
like: after each settled (500 ms) change at the root it asks the VCS
(`Vcs::detect`) whether the project is under version control, and when the
answer changes it emits `WorkspaceContextChanged { vcs_enabled }`, so UI surfaces
(the branch picker, the stream creation form) enable or disable themselves. The
first paint reads `get_workspace_context` (`vcs_enabled`). Stream refs
watchers are bound when a stream's watch starts; a repository created after boot
is watched from the next start.

### Refs watcher

`Vcs::watch_refs(ws, on_change)` — the provider's; git's is
`crates/oxplow-git/src/refs_watch.rs` (`GitRefsWatcher`), which
`GitProvider::watch_refs` bridges to the callback until the returned
guard drops. `StreamWatchRegistry` starts one per stream, turned into
`vcsRefsChanged` on the shared `EventBus`. Git's watcher debounces ~250ms (a single `git commit` fires a
dozen events touching `HEAD`, `refs/*`, `logs/*`, `index`, `ORIG_HEAD`, …).

File events are the fast path, not the guarantee: macOS FSEvents can
deliver `refs/` events 15s late or not at all under load (the browser
suite's parallel daemons showed it — commits never reached `v_commit`).
So the watcher also checks the refs' state every second — HEAD's text and
each ref file's and `packed-refs`' size and mtime — and announces a change
no file event did. A file event restates what the check compares against,
so one move is announced once.

When the stream lives in a secondary worktree (the common case — oxplow
creates worktrees as siblings of the main repo), the stream's
`.git` is a pointer file, not a directory. The watcher asks git2 for
the per-worktree git dir (`HEAD`, `index`, `logs/HEAD`) and the common
dir (where `refs/heads/*` actually update): the file watch is on the
common dir's `refs/`, and the timer's check reads both, so `git fetch` /
ref updates from outside the worktree are heard too.

Fires `vcsRefsChanged` after each debounce. Consumed silently (no
loading spinner) by:

- The Directory page's loaders (`v_file`, `v_branch`, `v_change` follow it).
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

### Wiki

`crates/oxplow-app/src/wiki_pages_watch.rs`: a `Scope` on the primary's watch.
Each batch of `.md` changes runs `wiki_pages::sync_page` per slug; a rescan runs
`scan_and_sync_all`. Every write is treated identically — agent and user edits
both re-baseline freshness — so this is the single sync path for `wiki_page`
metadata. See `data-model.md` → `wiki_page`.

### Config

`crates/oxplow-app/src/config_watch.rs`: a `Scope` on the primary's watch. A change
to `.oxplow/project.yaml` or the personal file calls
`Services::reload_config_from_disk`, which re-runs `load_project_config`, swaps
the in-memory `Arc<RwLock<OxplowConfig>>`, gives the `generated` lists to the
stream filters (`set_generated`, as `oxplow.file.mark_generated` does), and emits
`OxplowEvent::ConfigChanged`. Exists because config is otherwise read only once
at boot — without it, an out-of-band edit (notably the agent running
`/oxplow:configure`, which writes a `testing:` block and report collectors)
wouldn't take effect until restart. The IPC setter `set_agent_prompt_append`
still mutates the in-memory config directly; this covers every other edit path.

### Orphan detection (boot + runtime)

`StreamWatchRegistry::register_all` checks `worktree_path.exists()` before
starting a stream's watch. If a non-primary stream's worktree was deleted out from
under us while oxplow was offline (e.g. external `rm -rf`, `git worktree remove`),
it calls `StreamService::archive_stream(id, false)` to take the row out of the
rail and emits `OxplowEvent::StreamOrphaned { stream_id, title }` so the renderer
can toast ("Stream X was closed: its worktree directory was deleted."). Primary
streams are exempt — a missing project root is a different failure mode (the
daemon shouldn't have booted).

Ongoing detection works the same way: on every batch the announcer cheaply
re-checks `worktree_path.exists()`. If the root is gone, it runs the same archive
+ emit path and the watch ends. The check is on every batch, not just a removal,
because macOS FSEvents surfaces a directory's own deletion as an `Updated` event
of its parent. An archive ends the stream's watch before its worktree is
deleted, so it toasts nothing.

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
`commit-cherry-pick` / `commit-revert`) run `oxplow.git.cherry_pick` /
`oxplow.git.revert` against the active stream and fold the `auto_resolved`
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
- `oxplow.vcs.commit` runs `git add -u` (or `git add -A` with
  `include_untracked`) then `git commit -m`. Only the Files panel and the
  uncommitted-changes page run it; no MCP tool commits. Its result carries an
  inverse, `oxplow.vcs.uncommit` (soft reset to the prior head, changes stay
  staged; refused, nothing changed, once pushed, built on, mid-merge, or when
  the working tree has other changes — they would be staged with the commit's),
  so the toast offers Undo: "✓ Committed <sha7>" and, after the Undo, "Commit
  undone: the changes are back in the working tree" (`commandToast.ts`
  `ranMessage` / `undoneMessage`, used by a form, a page action and a menu
  command alike; a form's Undo confirms like the others').
- Push, pull, fetch, merge and rebase are long: the desktop runs each
  command inside a `BackgroundTaskStore` row (`runAsBackgroundTask` in
  `api.ts`) so the bottom-bar `BackgroundTaskIndicator` shows progress.
- History and branch lists read the datasets (`v_commit`, `v_branch`,
  `v_tag`; P5.B5, [vcs.md](./vcs.md)); ahead/behind, divergence,
  commits-ahead and a file's history stay live on `vcs_divergence`,
  `vcs_revisions_between` and `vcs_file_history`; a local branch against its own
  upstream is `v_branch.upstream` / `ahead` / `behind`, and a stream's HEAD is
  `v_stream.head_sha`.
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
  breaks a page: oxplow-bundled's Merge Readiness composes one row per
  branch ahead (`v_branch_divergence`).
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
- `oxplow.vcs.push` / `oxplow.vcs.pull` with `remote` + `branch` push the current
  branch to `<remote>/<branch>` (`git push <remote> HEAD:refs/heads/<branch>`,
  a refspec push that never touches another working dir) and pull
  `<remote>/<branch>` into it (fetch, then merge; a failed fetch
  short-circuits the merge). The dashboard's remote-branches card runs
  both.
- `remove_workspace` is `git worktree remove --force` then `git worktree
  prune`: a removal that fails (a directory already gone, a locked
  worktree) leaves a stale admin entry in `.git/worktrees/` that a later
  `worktree add` of the same path trips on, and prune clears it.

### Merging into another stream's branch

`oxplow.vcs.merge_out` sends a branch into another stream's branch. It is
the `merge` operation run in the stream that receives the branch (`stream`
is the receiver, `rev` the branch sent), so that stream's working files
change and any conflicts stop in its checkout. An agent can't run it from
another stream (`vcs_op` keeps an agent to its own stream), and a person
confirms every run. Git Status offers it as **Merge out**, on the callout's
Ahead line, in a Streams row's expansion and in the row's right-click menu
("Merge current into this"). The target is the stream that has the branch
checked out; with none, it isn't offered. Its confirmation can't name the
target branch: the operation's input holds only `stream` and `rev`.

It runs in the receiving worktree because the ways of moving the ref from
outside leave that worktree wrong:

- `git push <other-worktree-path> <branch>` is refused by default for
  the currently-checked-out branch (`receive.denyCurrentBranch`).
- `git update-ref` from our side advances the ref but leaves the
  other worktree's HEAD/index/working tree divergent — it then
  silently appears "dirty".

Pulling in stays available from the receiving stream ("Merge into current",
**Merge in**). The gitMerge sibling-worktree test in
`crates/oxplow-git/src/lib.rs` (`#[cfg(test)] mod tests`) asserts byte-equal
HEAD, status, and file content on the sibling after merging *its* branch
into the primary.

The git provider's `detect` (over `oxplow_git::is_git_repo`) requires the
project root *itself* to be the git toplevel —
nested git repos and parent-dir lookups are explicitly refused (see
`architecture.md`'s "Workspace isolation rule"). `isGitWorktree` rejects
secondary worktrees so oxplow won't try to nest its own worktrees inside
another tool's checkout.

## UI commit affordance

The Uncommitted Changes page has an inline message box over
`oxplow.vcs.commit`, and the command's menu entry opens that page. These are
the UI entry points for user-driven commits. oxplow never steers the agent to
commit. The Directory page (the Files launcher entry) is only the tree: no commit, pull or push, and no
branch status (the Git page has those).

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

### Any thread may call git in its own stream's worktree

Every open thread of a stream shares its worktree, so any thread's agent
may run git there; the `oxplow.vcs.*` commands are `Write` commands that
need the actor's thread, and that thread open. A different stream's
worktree (the primary checkout included) is off limits: the write guard
denies edits into it (`kind: edit`). A git command run in a shell isn't
classified.

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
- Subject + body run through `oxplow_domain::uri::extract` so the
  same wikilink + inline-mention rules used by wiki bodies and
  task descriptions also apply to commit messages
  (`wi-…`, `[[architecture]]`, `[[finding:fnd-1]]`, bare 7-40 hex shas).
- **Linked to the task by its work** (`commit_links.rs`, tsk1035). An
  agent's commit rarely names its task (and agents name tasks by title),
  so each new commit is checked against the efforts that closed in the
  two weeks before it: it **holds** an effort's work when they share a
  file and the commit's object for every shared file is the object the
  effort's end-snapshot bytes would be (`Vcs::object_at` against
  `ObjectStore::id_of`; a partial commit holds it too). It's linked by a
  `(work item URI) -- committed --> (commit URI)` edge — the shape of
  a declared impact, the effort in `source_extra` — which `v_commit_work_item`
  (v2) and the commit's backlinks read. The other order (committed, then
  the task closed) is the `effort.commits` reaction to `effort.finished`,
  over the commits since the effort started. An effort whose end-snapshot
  bytes have expired from Local History (or were never kept) can't be
  shown to hold anything, so it simply doesn't match; it must never error,
  or one old effort stops the commit linking to every other.
- **Logged as `vcs.commit.indexed`** `{ commit, stream, committed_at }`,
  once per commit (dedupe key), anchored to the stream whose workspace
  found it — every commit, unlike `vcs.head.moved`, which needs a clean
  worktree. The `effort.landing` consumer (`effort_landing.rs`) compares
  it with each **open** effort on that stream by content: the effort's
  files — recorded, plus what its thread's running turn has changed so far
  (a mid-turn commit comes before those are observed) — as they are in the
  worktree now, against the commit's version of each. It holds the effort
  when it touches one of them and has every file it touches as the effort
  left it, and holds it whole (`complete`) when that's true of all of
  them. Either way it logs `effort.landed { effort, commit, complete }`
  and links a linked effort's item to the commit; the effort policy closes
  on a complete landing (`.context/work-tracking.md`, rule 3).

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
first boot (the depth the old primary-only walk had). A query that names
a commit by its full `sha` (`v_commit`, `v_commit_file`) has it indexed
first when the index lacks it (`commit_indexer::ensure_commit`,
`.context/semantic-layer.md` "Made on demand"): the workspace's scan runs
(new history, logged as always), and a commit still missing is stored
without `vcs.commit.indexed` — it is old history, and that event means
new.

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
- [agent-model.md](./agent-model.md) — commits are user-driven; the `v_page_ref`
  dataset that reads the commit indexer's output.
- [editor-and-monaco.md](./editor-and-monaco.md) — blame overlay UI.
