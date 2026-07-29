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

## The shell and its daemons

Oxplow is **one shell process owning every window**, with **one
`oxplow-daemon` child per open project**. The shell holds no project
state at all: no `Services`, no database, no watchers, no control plane.
Each project window talks to its own daemon over loopback HTTP/WS (see
[remote-daemon.md](./remote-daemon.md)); the only things the shell
serves itself are the shell surface — windowing, native menus, the OS
clipboard, external URLs, and the project lifecycle below.

This replaced process-per-window (one OS process per project, each
booting its own `Services`). That model gave every project a separate
Dock tile and no shared window menu, and it had no idea what was
actually open: "focus the window that already has this project" needed
a per-project instance lock plus a loopback focus channel, and the
session set had to be *inferred* from "recent projects whose lock is
held". The shell now simply knows — see `WindowRegistry` in
`crates/oxplow-tauri-ipc/src/windows.rs`. Local and remote also stopped
being different code paths: a local project window is a remote client
that happens to be talking to a daemon on 127.0.0.1.

The trade is a process hop on every command (~3–4 ms round trip,
measured — see [performance.md](./performance.md)) and a daemon boot
before a window can appear.

Boot flow (`apps/desktop/src-tauri/src/main.rs` — one path, no modes):

- `resolve_project_dir()` → first positional CLI arg, else
  `OXPLOW_PROJECT_DIR`, else `None`. (No cwd fallback — a bare launch
  must not silently adopt its start directory.) The resolved dir is
  **canonicalized** (`absolutize_project_dir`): a relative root like
  `oxplow .` would otherwise collapse to `""` in the workspace
  path-traversal guard (`resolve_workspace_path` in oxplow-git), making
  every subdirectory read false-positive as an escape and killing file
  listings.
- `Some(dir)` with a `.oxplow/` dir → open it (`Startup::Project`).
- `Some(dir)` **without** `.oxplow/` → the setup window
  (`Startup::Setup`): a "Create an Oxplow project here?" confirmation
  (`<ProjectSetup>`). Confirm → `setup_project` creates `.oxplow/` and
  opens it, replacing the setup window; decline → `abort_setup` closes
  the window. Nothing is recorded into recents until setup is confirmed.
  `--init` skips the question and creates the project.
- `None` → **session restore** (`Startup::Restore`): reopen every
  project the session recorded, skipping any that is no longer a project
  on disk. If nothing opened, show the launcher.

**Closing a project window stops its daemon, and therefore its
agents.** A deliberate choice: closing the window is the user saying
they're done with the project, and a backend still churning behind a
window that no longer exists is worse than a clean stop. If that ever
needs to change, the feature is reattaching to a surviving daemon — not
leaving one running by accident.

**Opening a project** (`ShellWindows::open_project`) is: focus the
window that already has it, else sweep any orphaned daemon, start a
daemon, wait for it to report its loopback endpoint, then create a
window carrying that endpoint. The order is forced — the base URL is
injected at window creation — which is why the call blocks. At startup
that happens on the main thread (nothing is on screen yet anyway);
`open_project` / `create_project` from the UI run it on a blocking
thread so open windows stay responsive.

**Creating and opening are separate doors** (tsk248). `open_project`
refuses a dir with no `.oxplow/` — it never initializes one — and the
error names File ▸ New Project…. Creating is `create_project`: it
validates the dir is *not* already a project, creates `.oxplow/`, and
opens it. Creating always opens a new window and never closes the
caller's, so it can't destroy the launcher or the window the command ran
from. The two guards are the pure `resolve_open_target` /
`resolve_create_target` helpers in `commands/launch.rs`.

**Which screen a window is** comes from the shell, not a command: the
injected `window.__OXPLOW__.kind` (`project` / `launcher` / `setup`)
that `<Root>` switches on. There is no launch-mode round trip and no
loading flash — the context is set before any page script runs. A window
the shell didn't create (a plain browser over a tunnel) has no context
and gets the app shell.

**Orphaned daemons.** A shell that dies without unwinding (crash,
SIGKILL) leaves its daemons running, holding their projects' instance
locks. Each daemon publishes `.oxplow/daemon.json` (`{base_url, pid}`),
so the next open sweeps it: kill, wait for the project lock to come free
(`oxplow_app::wait_for_project_unlock`), then start the replacement.
Orphans are killed, not adopted — a backend outliving its window is a
feature nobody has asked for, and adopting one on a guess is worse than
a clean restart.

**One instance per project** is still enforced by the lock, but now by
the *daemon*, which is the thing that would actually collide (two
`Services` on one `local.sqlite` = double watchers + a serialized
SQLite writer). A second daemon for the same project exits at once, and
the shell surfaces that as a failed open.

**Windows and the native menu.** No window is declared in
`tauri.conf.json` (`app.windows` is `[]`); every window is built at
runtime by `apps/desktop/src-tauri/src/windows.rs`, which is also the
one place their shape (size, overlay title bar, drag-drop off) is
defined. **The label is load-bearing:**

| Label | Window | Capability |
|---|---|---|
| `project-<n>` | a project | `capabilities/oxplow-windows.json` |
| `launcher` | the project picker | same |
| `setup` | "create a project here?" | same |
| `ext-url-<uuid>` | sandboxed external URL | `capabilities/external-url.json` (empty) |

`is_project_label` (strictly `project-<digits>`, tighter than the
capability glob) gates the close handler's session bookkeeping — window
events are builder-level and fire for *every* window, so without it an
external-URL webview closing dropped the project from the restore set.

The native menu bar is app-global (`AppHandle::set_menu`), but menu
state is per window, so `commands/menu.rs` keeps a `MenuRegistry` of
each window's latest snapshot and installs whichever belongs to the
focused one. A push from a background window is recorded but not
installed; a focus change re-installs; focusing a window with no
snapshot (an external-URL webview) leaves the menu alone rather than
blanking it. Activations are emitted to the focused window, not
broadcast — with two projects open, Cmd-S belongs to exactly one.

**Session restore.** A global `session.json`
(`oxplow_config::SessionProjects`) holds the set of project dirs that
were open last. A bare launch reopens them; `ShellWindows::restorable`
filters the list to dirs that are still projects and de-duplicates it
(the same project twice would open one window and then fail to start a
second daemon against its own lock). If nothing opened, the launcher
comes up — so a launch can never end with no window, which is what
tsk252 was.

The set is **owned**, not inferred: the shell writes
`session.json` = its open project windows, in open order, whenever a
window opens or closes. Two rules on top:

- **Closing one window while others are open** drops that project — you
  are done with it, don't reopen it.
- **Closing the last window, or quitting the whole app**, preserves the
  set so it's what comes back. `RunEvent::ExitRequested` calls
  `begin_quit()` first, which freezes the set so Cmd-Q with three
  windows doesn't empty it one window at a time.

**Closing the last project window opens the launcher** rather than
leaving an app with no windows — the IntelliJ/Xcode rule. Closing the
*launcher* is how you quit.

That decision is made in the window-`Destroyed` handler
(`ShellWindows::should_show_launcher`), not at `ExitRequested`. Deciding
it at exit time would mean inferring "was this a last-window close or a
Cmd-Q?" from the exit code, and a wrong guess there makes the app
unquittable. Doing it on destroy means `ExitRequested` prevents nothing:
by the time it fires, either the launcher already replaced the window,
or the launcher itself is what closed.

The earlier shape — `prevent_exit()` on macOS, leaving zero windows and
a dock icon to bring them back — is the letter of the macOS convention
without its substance: the menu bar stayed on screen showing the last
project's items, all of them dead, because the focused window they
dispatch to was gone.

**Global app state** lives under the app-config dir
(`net.voxland.oxplow`, resolved by `oxplow_config::global_config_dir()`
so non-Tauri code can find it): `recent-projects.json`
(`oxplow_config::RecentProjects`) and `session.json` — see
[data-model.md](./data-model.md).

`OXPLOW_HOME` overrides that dir (used verbatim; empty = unset), so a
dev build can run alongside an installed one without sharing session,
recents, or global metric manifests. It's read once in
`global_config_dir()`, so every caller inherits it — including the
`oxplow-daemon` children, which the shell spawns without `env_clear()`.
It moves oxplow's own state only — Tauri's `app_config_dir()` path
resolver still uses the platform location. See [DEV.md](../DEV.md).

The workspace isolation rule below is enforced **per daemon**: each
daemon is started with one project dir and treats it as the workspace
root. The shell has no workspace of its own.

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
