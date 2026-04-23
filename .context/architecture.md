# Architecture guidance: VS Code-inspired workflow without full workbench adoption

## Goal

Use a lot of the **workflow concepts** and selected building blocks from VS Code without turning this app into the full VS Code IDE shell.

This note is the default guidance for future implementation decisions unless a later design explicitly replaces it.

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

## Workspace isolation rule

Oxplow usage must always be isolated to the directory where the daemon was started and that directory's descendants.

Specifically:

- do not look to parent directories for project data, repo state, workspace files, or configuration
- treat the daemon start directory as the workspace root, even if it lives inside some larger parent repo
- only consider Git enabled when that workspace root itself contains the repo root
- if the workspace root is not its own Git repo, oxplow should still work for file browsing/editing and agent panes, but Git features must be disabled
- when Git is disabled, alternate stream creation and other Git-dependent flows must also be disabled

This rule takes priority over convenience heuristics like "find the nearest enclosing git repo."

## Core recommendation

Prefer a **hybrid architecture**:

1. **Keep the custom React shell**
2. **Keep Monaco as the editor core**
3. **Reuse VS Code concepts heavily**
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

- More IDE behavior must be assembled intentionally rather than inherited from a full workbench
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

### 4. Add IDE primitives explicitly

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

The goal is not “just a file list”; it is to create the beginning of a broader IDE-style file/workspace layer.

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

> **Build a custom, stream-aware IDE shell with Monaco at the core, heavily inspired by VS Code workflows and concepts, but without importing the full VS Code workbench.**
