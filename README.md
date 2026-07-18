# Oxplow

A desktop app for guiding and tracking coding agents (Claude Code, Codex,
and others) across many parallel branches of one project. Each **stream** is
its own branch, worktree, and agent instance.

Oxplow is built around steering an agent like a pair-programming
partner. The agent does the typing; your job is to plan, review, and
understand the big picture. It's a comprehension and steering surface --
a work queue, commit/wait points, file snapshots, and a Local History
view sit on top of the agent so you can keep several agents moving
through real work without them clobbering each other. Monaco + LSP are
there for reading code and the occasional manual change, but oxplow is
not an IDE: the focus is the system, not the editor.

## What it gives you

- **Streams + worktrees.** Each stream owns a branch and its own
  checkout, created as a sibling of the main repo at
  `<parent>/<project_basename>-<slug>/`. Running agents never share
  a working tree — writes from non-writer threads are denied at the
  hook level.
- **Threads inside a stream.** One active "writer" thread, any number
  of read-only query threads. Agents on query threads can ask the
  writer anything but can't modify files.
- **Work queue.** A durable list of tasks with status lifecycle
  (`ready → in_progress → done`, plus `blocked` / `canceled` /
  `archived`), grouped by epic,
  dragged between sections, and ordered by a single `sort_index`
  shared with commit points and wait points.
- **Commit points and wait points.** Inline markers in the queue.
  Commit points fire a git commit when the agent reaches them; wait
  points block the agent until the user releases the gate. Auto-
  commit mode replaces the commit-point queue with an every-stop
  default.
- **Local History.** Every turn produces file snapshots. Efforts are
  grouped per task; the modal shows per-effort file-change
  counts and lets you jump back to any end snapshot.
- **Editor + file browser.** Monaco-based editor with blame overlay,
  diff tabs, and Git-aware decorations. Custom React file tree rather
  than the VS Code explorer.
- **Agent control plane.** MCP tool surface (`mcp__oxplow__…`) plus
  agent hook integration — stop-hook directives feed a work-
  queue orchestrator that auto-progresses the agent without oxplow
  ever sending it a raw prompt.

## Architecture in one paragraph

A Rust backend (Tauri 2 shell + a set of `oxplow-*` crates) owns the
SQLite database, the stores (streams, threads, work items, commit/wait
points, snapshots, efforts), and an MCP server + hook endpoint that
Claude Code or Codex connects to per thread. The React/Monaco/xterm frontend
subscribes to store events and renders the rail HUD + pages UI (work
queue, file browser, editor/diff pages, terminal, Local History).
This repo keeps its own internal architecture notes under
[.context/](./.context/) (a convention for *this* codebase, not an oxplow
feature).

## Running

See [DEV.md](./DEV.md) for installing, building, and running from
source. Prebuilt installers are produced by CI on every push —
download the `oxplow-<os>` artifact from the latest Actions run to
grab an unsigned `.dmg` / `.exe` / `.AppImage` / `.deb`.

## Status

Early / personal — not yet signed, not yet in a store. APIs and
on-disk schema change freely; migrations are the forward-only path.

## License

TBD.
