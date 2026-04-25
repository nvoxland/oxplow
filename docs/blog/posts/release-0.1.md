---
date: 2026-04-25
categories:
  - Releases
---

# Oxplow 0.1 -- First Release

The first tagged build of Oxplow is out. It's the smallest thing I'd actually use day-to-day, and it's what I've been using to build Oxplow itself for the last few months.

<!-- more -->

Oxplow is an IDE built around the way I actually work with coding agents now: not one chat window doing one thing at a time, but several agents working on parallel branches of the same project while I steer. It's a desktop app -- Electron + React on top of a local SQLite store -- and everything runs on your machine.

## What's in 0.1

The core loop works:

- **Streams and threads.** Each stream is a git worktree with its own agent thread. You can have several open at once; only one thread per stream is the writer at a time, so two agents can't fight over the same files.
- **Work queue.** File a task, send it to an agent, watch it move from `ready` → `in_progress` → `human_check` → `done`. Reordering is a drag; the queue drives what the agent picks up next.
- **Commit and wait points.** A shared timeline so a "wait until commit X lands" dependency is a real, durable thing instead of a sticky note.
- **Local History.** Every agent turn captures snapshots of what it touched. Rolling back a bad turn doesn't require `git reflog` heroics -- you pick the snapshot and restore.
- **Editor pane.** Monaco with blame, diff view, and an LSP bridge for the languages I had handy. It's not VS Code; it's the surface I needed for reviewing what an agent just did.
- **MCP tool surface.** The agent talks to Oxplow over MCP -- filing work items, recording findings, requesting commits. The toolset is small on purpose.
- **Notes.** A lightweight wiki-link space for the durable knowledge that doesn't belong in a commit message.

## Where it stands

This is a 0.1, not a 1.0. The shape is right -- I've been driving real work through it -- but a lot of the corners are rough:

- Only Claude is wired up as an agent. The control surface is meant to be agent-agnostic eventually; right now it isn't.
- Multi-repo streams aren't a thing yet. One project per workspace.
- The installer story is "grab the CI artifact or build from source." No notarized DMG, no auto-update.
- LSP support is whatever I had running locally. Adding new servers works but isn't documented.

## Why a 0.1 at all

The honest reason: I want to stop saying "oh it's not ready yet." It's ready enough that someone else can install it, drive an agent through a real task, and tell me where it falls over. That's the feedback loop I need to push the next version forward.

If you try it and something breaks -- or something is just confusing -- open an issue at [github.com/nvoxland/oxplow](https://github.com/nvoxland/oxplow). The bar for "this should be filed" is low.

## What's next

The things I want most for the next few releases, roughly in order:

- A real install path that doesn't require a checkout.
- Better review affordances around an agent's turn -- not just the diff, but *why* each edit happened.
- Multi-repo streams.
- A second agent backend, so the agent-agnostic claim is actually tested.

The [architecture vision](../../philosophy/architecture-vision.md) page has the longer version of where this is going.
