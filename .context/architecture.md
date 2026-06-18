# Architecture guidance: a comprehension + steering surface, not an IDE

## Goal

Oxplow is **explicitly not an IDE.** The agent does the typing, so the
app's center of gravity is **understanding the big picture and steering
the agent** — planning, reviewing, navigating, and comprehending the
system — not being an editing surface. The UI is a **web / Linear-style
shape** (rail HUD + pages; see `.context/pages-and-tabs.md`), optimized
for navigation and comprehension rather than keystroke-level code entry.

Code editing and viewing still exist and matter: **in the end the code
is the reality.** We keep Monaco + LSP (intellisense, go-to-def,
diagnostics) for drilling down and making manual changes when you need
to. But that editor is a **drill-down detail view, not the primary
surface** — it is reached *from* the comprehension/steering surface, it
does not define the app the way it defines an IDE.

We still borrow **workflow concepts** and small building blocks from VS
Code (Monaco, codicons, decorations, the URI/model patterns) — we just
don't organize the product around an editor.

This note is the default guidance for future implementation decisions
unless a later design explicitly replaces it.

## No agent automation

Oxplow steers the agent; it never *drives* it. The agent does its own
typing and makes its own tool calls — **the only source of agent
terminal input is human keystrokes / paste via the UI.** oxplow must
never programmatically generate, inject, or auto-respond with agent
terminal input (automating the agent CLI would risk violating its
license/ToS). Steering happens through hook responses the agent's own
harness injects, not by oxplow synthesizing input. This is a hard
invariant, guarded by tests — the mechanics and the human-input
transport (`forward_terminal_input`) live in
[agent-model.md](./agent-model.md#no-synthesized-agent-terminal-input-no-automation).

## Current app shape

- Custom React shell for layout and UI state
- Monaco editor used as an editor widget, not a full workbench
- Custom backend daemon that owns:
  - streams
  - per-stream worktrees
  - tmux / Claude panes
  - hook events
  - stream-scoped APIs

That means the app already has a strong custom domain model. In particular, **streams** are first-class and do not naturally map 1:1 to stock VS Code assumptions.

### Primary vs worktree streams

There is exactly one **primary** stream (`kind: "primary"`). It represents the repo itself: its `worktree_path` IS the daemon's project directory, its `title` is the project basename, and its recorded branch tracks whatever HEAD is currently checked out. The primary is the leftmost tab and cannot be deleted.

Every other stream is a **worktree** stream (`kind: "worktree"`). At creation it gets its own `git worktree add` at `<parent_of_project>/<project_basename>-<slug>/` — a sibling of the main repo. The slug is fixed at creation; the project-basename prefix prevents collisions when multiple projects share a parent directory. Pre-existing worktree streams created under the legacy `<project>/.oxplow/worktrees/<slug>/` location keep their stored `worktree_path` and continue to work unchanged; only new worktrees use the sibling layout.

Both kinds can switch branches — either via the StreamRail "Switch branch…" context menu (routed through `Services.checkoutStreamBranch()`), or by an external `git checkout` in the worktree dir (picked up by the `GitRefsWatcherRegistry` → `maybeSyncStreamBranch()`). Git's own errors (dirty tree, missing branch, already checked out elsewhere) propagate verbatim to the UI; oxplow does no pre-flight validation.

## Process-per-window & launcher

Oxplow is **one OS process per project window**. Each window boots its
own `Services` (its own `.oxplow/state.sqlite`, event bus, control
plane on an ephemeral `127.0.0.1` port, watchers, etc.). There is no
shared in-process state across windows — windows are as isolated as two
separate app launches. This was a deliberate choice over an in-process
`HashMap<window, Services>` registry, which would have required
threading window context through every IPC command and rewriting all
event-emission paths.

Boot flow (`apps/desktop/src-tauri/src/main.rs`):

- `resolve_project_dir()` → first positional CLI arg, else
  `OXPLOW_PROJECT_DIR`, else `None`. (No cwd fallback — a bare launch
  must not silently adopt its start directory.) The resolved dir is
  **canonicalized** (`absolutize_project_dir`): a relative root like
  `oxplow .` would otherwise collapse to `""` in the workspace
  path-traversal guard (`resolve_workspace_path` in oxplow-git), making
  every subdirectory read false-positive as an escape and killing file
  listings.
- `Some(dir)` with a `.oxplow/` dir → `run_project(dir, ctx)`: today's
  full boot.
- `Some(dir)` **without** `.oxplow/` → `run_setup(dir, ctx)`: **no
  `Services`** — the renderer shows a "Create an Oxplow project here?"
  confirmation (`<ProjectSetup>`). Confirm → `setup_project` creates
  `.oxplow/` and relaunches the process (which now takes the
  `run_project` branch); decline → `abort_setup` exits. Nothing is
  recorded into recents until setup is confirmed.
- `None` → **session restore**: if the global session has open
  windows from last exit, `restore_session()` spawns one process per
  still-valid project dir and we show no launcher. Otherwise →
  `run_launcher(ctx)`: **no `Services`** — just the recent-projects
  surface and a launcher window.

The renderer's `<Root>` calls `get_launch_mode` and renders
`<Launcher>` / `<ProjectSetup>` / `<App>` accordingly.
- `generate_context!()` is expanded once in `main()` and handed to
  whichever mode runs (it embeds the Info.plist and may expand only
  once per binary).

Opening / reopening a project (IntelliJ-style) goes through the
`open_project` IPC command, which **spawns a fresh process** of the
current executable with `OXPLOW_PROJECT_DIR` set. "New window" = spawn;
"replace this window" = spawn then `app.exit(0)`. The launcher and the
in-window File ▸ Open Project commands both route here — but through
`openProjectGuarded`, which first calls `project_needs_setup` and forces
an **uninitialized** dir to a *new* window. That way a declined setup
(`<ProjectSetup>` → Cancel) only closes the setup window and never
destroys the launcher or the caller's current project window.

A **per-project instance lock** (`.oxplow/instance.lock`, fs2 advisory
lock acquired in `run_project`) prevents two processes from booting on
the same project — that would double the watchers and contend on the
single SQLite writer. `open_project` probes the lock first; if the
project is already open it **focuses the existing window** instead of
spawning a duplicate. Focus uses a small loopback channel: each
`run_project` publishes `.oxplow/instance.json` (`{ focus_port, nonce }`)
and serves a background thread that raises the window on a nonce-matching
ping; `oxplow_app::request_focus` does the ping. If the running instance
can't be reached (stale state), `open_project` falls back to a clear
"already open" error. The instance lock releases on process death, so a
crashed instance's stale focus port is never used (the lock probe sees
the project as not-open and the second launch just opens it).

**Session restore.** A global `session.json`
(`oxplow_config::SessionProjects`) holds the set of project dirs that
were open last. A bare launch reopens them (skipping dirs that are gone
/ no longer have `.oxplow/`); each restore spawn carries
`OXPLOW_RESTORING`.

How the set is maintained:

- **Fresh (non-restore) boot** re-snapshots it: `run_project` writes
  `session.json` = self + every recent project whose
  `.oxplow/instance.lock` is still held (`live_project_dirs`). So the
  first window of a session resets the set to what's actually live and
  stale entries don't accumulate.
- **Restore boot** (`OXPLOW_RESTORING` set) skips the re-snapshot and
  just `add`s itself, so concurrent restores can't clobber the set
  being restored.
- **Closing one window while others are open** drops that project from
  the set (`other_window_alive` is true → `session.remove`) — you're
  done with it, don't reopen it.
- **Closing the last/only window, or quitting the whole app** preserves
  the set so it's restored. A full quit flips a `QUITTING` flag (set
  from `RunEvent::ExitRequested`) that suppresses the per-window
  removal, so Cmd-Q with several windows brings them all back; closing
  the last lone window has no other live window so it's kept too.

**Global app state** lives under the app-config dir
(`net.voxland.oxplow`, resolved by `oxplow_config::global_config_dir()`
so non-Tauri code can find it): `recent-projects.json`
(`oxplow_config::RecentProjects`) and `session.json` — see
[data-model.md](./data-model.md).

The workspace isolation rule below still holds, now **per process**:
each process treats its own project dir as the workspace root.

## Workspace isolation rule

Oxplow may write only inside (a) the daemon's start directory and its descendants, or (b) a worktree directory that an oxplow stream owns. Anywhere else is off-limits.

Specifically:

- do not look to parent directories for project data, repo state, workspace files, or configuration — even when oxplow's own worktree streams live there as siblings
- treat the daemon start directory as the workspace root, even if it lives inside some larger parent repo
- only consider Git enabled when that workspace root itself contains the repo root
- if the workspace root is not its own Git repo, oxplow should still work for file browsing/editing and agent panes, but Git features must be disabled
- when Git is disabled, alternate stream creation and other Git-dependent flows must also be disabled
- the one explicit exception: stream-creation can `git worktree add` a sibling of the project at `<parent>/<project_basename>-<slug>/`, and stream operations may read/write inside *that* directory tree (and only that). Other paths in the parent dir remain off-limits.

This rule takes priority over convenience heuristics like "find the nearest enclosing git repo."

## Core recommendation

Prefer a **hybrid architecture**:

1. **Keep the custom React shell** — organized around comprehension and
   steering (rail HUD + pages), not around an editor.
2. **Keep Monaco + LSP as the drill-down editor/viewer**, reached from
   the surface — not as the app's center of gravity.
3. **Reuse VS Code concepts heavily** (as workflow concepts, not as a workbench)
4. **Reuse small, standalone pieces where practical**
5. **Do not try to embed the full VS Code workbench or explorer implementation directly**

## What to reuse directly

These are the parts most worth reusing as actual building blocks:

- **Monaco editor**
  - editor models
  - URIs
  - decorations
  - diff editor
  - language features exposed through Monaco
- **Codicon-style iconography**
  - file/folder/action icons
  - status badges where useful
- **Monaco-centered editor patterns**
  - open file models by URI
  - editor/view state persistence
  - decorations for diagnostics, Git state, and selections

## What to reuse as concepts, not necessarily code

These should guide product and implementation design, but should usually be implemented in this codebase rather than imported from VS Code workbench internals:

- left sidebar / center editors / bottom panel layout
- activity-style navigation and tabs
- explorer tree behavior
- quick-open / command palette workflows
- command registry and keybinding concepts
- context-driven actions
- file decorations and status badges
- SCM-style mental model for changed files
- workspace-oriented editor model

## What not to adopt directly right now

Avoid directly adopting the full VS Code workbench stack unless there is an explicit architectural decision to pivot the app in that direction.

In particular, do **not** assume direct reuse of:

- VS Code explorer control
- SCM view implementation
- activity bar / panel container internals
- extension host model
- broad workbench service graph

These pieces are deeply tied to the larger workbench/runtime architecture and are not lightweight drop-ins.

## Why this hybrid approach fits this app

### Strengths

- Works cleanly with the existing **React shell**
- Preserves the app’s custom **stream/worktree** model
- Avoids a large workbench migration
- Gives a strong path toward:
  - file explorer
  - Git-aware file decorations
  - open/save file workflows
  - search/filter
  - command palette behavior
  - richer editor interactions
  - LSP integration layered on top of Monaco

### Tradeoff

- More behavior must be assembled intentionally rather than inherited from a full workbench
- Some features that VS Code gets “for free” from its internal architecture will need custom glue here

## Recommended architectural direction

### 1. Keep the app shell custom

The outer shell should remain app-specific and stream-aware.

The shell should continue to own:

- current stream selection
- stream tabs
- left sidebar modes
- bottom panel
- daemon connection state
- stream-scoped routing of UI state

### 2. Treat streams as first-class workspace contexts

Do not force streams into a fake single-workspace model too early.

Instead:

- each stream should continue to own its own worktree path
- file browsing/editing/search should be scoped to the selected stream
- future Git/LSP/file APIs should be stream-aware from the start
- all stream/workspace resolution must stay within the daemon start directory tree and never climb upward to enclosing parent projects

### 3. Build a VS Code-like file/editor architecture on top of Monaco

Future file work should follow these principles:

- represent opened files by stable URIs
- keep Monaco models keyed by URI
- preserve editor/view state per file
- support decorations for diagnostics, Git, and selection state
- make explorer selection drive editor opening

### 4. Add app primitives explicitly

Prefer adding small, composable primitives rather than importing a giant workbench dependency.

Important primitives to add over time:

- command registry
- keybinding layer
- stream-scoped file service
- stream-scoped Git status service
- explorer tree model
- editor tab model
- quick-open / search model
- diagnostics/LSP integration

## Recommendation for future file explorer work

When implementing the file explorer:

- use a **custom React tree**
- make it stream-aware
- back it with a daemon API rooted to the stream worktree
- design it for:
  - file open actions
  - lazy loading
  - Git decorations
  - file icons
  - filtering/search
  - future context actions

The goal is a navigation/comprehension layer — a way to see and reach
code well — not the foundation of an IDE. It serves understanding and
drill-down, not an editor-centric workflow.

## Recommendation for future LSP work

Prefer:

- **Monaco + LSP bridge/client integration**
- daemon-managed workspace/file context where needed
- stream-aware workspace routing

Do not assume that adopting full VS Code workbench is required to get meaningful LSP behavior.

## Recommendation for future Git integration

Git integration should likely be custom and stream-aware:

- daemon provides per-stream Git status
- explorer shows changed/added/untracked states
- open editors can show dirty/Git decorations
- future SCM panel can use VS Code-inspired concepts without needing the stock SCM view

## Decision rule for future architecture choices

When deciding whether to adopt a VS Code-originated piece, prefer it only if it is true:

1. It is reasonably modular on its own
2. It does not drag in a large hidden workbench dependency graph
3. It does not fight the stream/worktree model
4. It saves meaningful time compared to implementing the same concept cleanly in this app

If those are not true, prefer:

- reusing the **concept**
- reusing Monaco primitives
- implementing the app-specific version locally

## Default stance

Until explicitly changed, the default architecture stance is:

> **Build a custom, stream-aware comprehension + steering surface
> (web/Linear-style), where the human plans, reviews, navigates, and
> understands agent-driven work. Keep Monaco + LSP as a drill-down
> editor/viewer for reading code and making manual changes — because in
> the end the code is the reality — but do not organize the product
> around an editor, and do not import the full VS Code workbench.**
