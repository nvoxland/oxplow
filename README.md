# Oxplow

A desktop IDE for running Claude Code (and other coding agents) across many
parallel branches of the same project — built around a **stream** model
where each stream is its own branch, worktree, and agent instance.

Oxplow is a stream-aware React + Electron shell with Monaco at its core.
It layers a queue, commit/wait points, file snapshots, and a Local
History view on top of the agent so you can steer multiple agents
through real work without them clobbering each other.

## What it gives you

- **Streams + worktrees.** Each stream owns a branch and a checkout
  under `.oxplow/worktrees/`. Running agents never share a working
  tree — writes from non-writer threads are denied at the hook level.
- **Threads inside a stream.** One active "writer" thread, any number
  of read-only query threads. Agents on query threads can ask the
  writer anything but can't modify files.
- **Work queue.** A durable list of tasks with status lifecycle
  (`ready → in_progress → human_check → done`), grouped by epic,
  dragged between sections, and ordered by a single `sort_index`
  shared with commit points and wait points.
- **Commit points and wait points.** Inline markers in the queue.
  Commit points fire a git commit when the agent reaches them; wait
  points block the agent until the user releases the gate. Auto-
  commit mode replaces the commit-point queue with an every-stop
  default.
- **Local History.** Every turn produces file snapshots. Efforts are
  grouped per work item; the modal shows per-effort file-change
  counts and lets you jump back to any end snapshot.
- **Editor + file browser.** Monaco-based editor with blame overlay,
  diff tabs, and Git-aware decorations. Custom React file tree rather
  than the VS Code explorer.
- **Agent control plane.** MCP tool surface (`mcp__oxplow__…`) plus
  Claude Code hook integration — stop-hook directives feed a work-
  queue orchestrator that auto-progresses the agent without oxplow
  ever sending it a raw prompt.

## Architecture in one paragraph

Custom Electron main process owns a SQLite database
(`.oxplow/state.sqlite`), a set of stores (streams, threads, work
items, commit/wait points, snapshots, efforts), and an MCP server +
HTTP hook endpoint that Claude Code connects to per thread. The React
renderer subscribes to store events and renders the dock-shell UI
(Plan pane, file browser, editor tabs, terminal pane, Local History).
Detailed subsystem docs live under [.context/](./.context/).

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
