---
date: 2026-06-12
categories:
  - Releases
---

# Oxplow 0.5 -- Pick-your-agent, comments on anything, site-wide search

0.5 has a lot of changes. We're working toward making Oxplow an "Integrated Understanding Environment" that can work with whatever technology or stack you have.

<!-- more -->

## Pick your agent: Claude, Codex, or OpenCode

Time to move beyond just Claude. Agents are configurable now, and you choose which one drives a thread:

- **Three supported agents** -- Claude, Codex, and OpenCode -- selected per launch, with the model set via `agentModels` in `oxplow.yaml`.
- **Each ships the Oxplow skills and slash commands**, so a navigator session behaves the same whichever agent you point at it.

## Comment on anything

The goal is to have Oxplow be your "Integrated Understanding environment" and comments are being positioned as way to collect up your thoughts and ask questions throughout the system.

You can now attach a threaded comment to almost any surface in the app, and it stays anchored to the thing it's about. This is the new interaction model running through the whole release:

- **Anchored across the app** -- file lines in the editor, task rows, terminal and agent-pane buffer regions, commit-graph rows, the git dashboard, findings, and wiki text. Comments are typed and hierarchical, not just a flat note field.
- **Anchoring that survives edits.** Comments store surrounding context and relink with bounded fuzzy matching when the underlying text shifts, instead of silently detaching the moment a line moves.
- **A Comments Dashboard** that defaults to unresolved, reveals resolved by date, and gives every comment a "Go to location" jump.
- **In-page navigator** with popover stepping between comments and sane handling for orphaned ones.
- **Right-click "Comment…"** on file-tree and task rows (draggable-safe), plus a rail HUD section for open comments.
- **Referenced refs.** Comment on a selection containing `file:` / `dir:` / `gitcommit:` links and those refs are captured, so the comment knows what it points at.

## Polyglot test & coverage collection

The effort-scoped collection from 0.4 was Rust/JS-shaped. 0.5 makes it pluggable:

- **A collector plugin runtime** with jaq, Starlark, and exec transform runtimes. Plugins are namespaced (the `oxplow.` prefix is reserved) and loaded from a file, not inline YAML.
- **Multiple test/coverage reports per effort**, so a polyglot repo can report from more than one toolchain into the same effort.
- **Tests render as a tech-natural expandable tree**, and `oxplow-coverage` is now a pure-types crate so parsing has no runtime entanglement.
- Bundled jaq plugins tolerate malformed fields, jaq plugins are restricted to pure functions, and exec-based test-report plugins are tagged lower-trust like the coverage path.

## Tasks and the Work panel read clearer

- **An In Progress section above Ready** on the Tasks page, with summary counts that read the bucketed work state instead of recomputing it.
- **Task ids are visible** in the page title, the details rail, and left-nav hovers, so it's easy to reference one.
- **Backlog visibility fix** -- ready items render, with the drawer collapsing to a count and a link.
- **Effort coverage as first-class UI.** Each effort gets a compact test/coverage summary card and a detail page, with a nudge when a run produced no report.

## Site-wide search

There's a unified search across everything Oxplow knows about -- the database, wiki pages, and file contents in one ranked result list.

## Real LSP in the editor

The editor's language support was a thin custom bridge. It's now a full Monaco LSP integration riding shared backend sessions:

- **The full Monaco feature set** for every configured language -- diagnostics, completion (with snippet support), hover, go-to-definition, references, rename, formatting.
- **Sessions v2.** Real server capabilities, an event pump, a document mirror, and a structured RPC surface. The editor and any agent share the same backend session instead of standing up parallel ones.
- **Workspace edits land everywhere**, including non-open files, so a rename actually rewrites the files it should.
- **A Language Servers section in Settings**, and agents can manage servers themselves via `lsp_list_servers` / `lsp_install_server` MCP tools (Mason-backed install).

## Terminal page

- **A plain shell in the worktree directory**, with a nav bar and session restore.
- **Multiple terminals per stream** with a hover-expand tab strip.
- **File-path links resolve against the pty's live cwd**, and a missing target is a friendly not-found instead of an error.

## UI cleanup

- **Reorganized menu bar** -- dedicated Git and Tasks menus and a leaner View menu.
- **Center-tab reorder** with free drag, an insertion line, and overflow-promote, part of the ongoing tab/IA redesign.
- **The native WKWebView context menu is suppressed** app-wide so right-click is Oxplow's, plus edit-menu separators that don't throw console errors.

## Remote daemon mode

Added a daemon mode that lets you run the backend on a remote machine (such as an EC2 instance) and connect the UI from a local machine, if you like to work that way.
