---
date: 2026-04-29
categories:
  - Releases
---

# Oxplow 0.2 -- A whole new UX

0.2 is mostly one big shift and a long list of follow-ons. The big shift: Oxplow is no longer a window full of docked panels and modal dialogs. Everything is a page now, the rail on the left is the persistent HUD, and the center pane is a stack of tabs you navigate like a browser.

<!-- more -->

Playing with 0.1 for a while, the "be like an IDE" was feeling very confining. The problem is that we are not the same "code first" application that an IDE was designed for with all the tools in reach.

Instead, we are more of an explorer--more of a web of data and information.

That rewrite is most of 0.2. Then a bunch of subsystems got pulled forward to match.

## Information architecture

The old shell had a left dock, a center area, a bottom dock, and a small zoo of modals (Settings, New Stream, New Task, confirms, prompts). 0.2 collapses all of that:

- **Everything is a page.** Settings, New Stream, New Task, Code Quality, the Git dashboard, individual files, individual commits, individual notes -- they're all page kinds in the same tab system. No more modals stealing focus, no more bottom dock to remember about.
- **Browser-style tabs.** Open in same tab is the default; modifier-click opens a new tab. Tabs have history (back/forward), bookmarks, and drag-reorder. File clicks, note clicks, and rail rows all go through one routing chokepoint so the behaviour is consistent everywhere.
- **A persistent rail on the left.** Current Work (the active item, with epic children inlined), Finished, History (recently visited pages), Bookmarks, and an uncommitted-changes summary. Resizable. The rail is the thing you keep glancing at; pages are the thing you act on.
- **Slideovers replaced confirm/prompt dialogs.** When something needs a bit of input but doesn't deserve a whole page, it slides in from the side. Right-click context menus replaced kebab menus for destructive actions.- **The Start page is gone.** A search overlay launcher took its place -- press the shortcut, type, jump.

The shell is simpler in the small ways too: page chrome is shared, titles register through context so the nav bar is consistent, and the tab bar handles drag-reorder in the rail and the center pane.

## Git, foregrounded

In 0.1, git was something you did *to* a stream from a settings panel. In 0.2 it's a first-class surface:

- **Git Dashboard page** with a Streams card (each stream's worktree, with per-worktree uncommitted stats and a working dot when an agent is active there) and a separate Upstream card with Push / Pull / Fetch split out.
- **Recent commits** render as a graph with click-to-reveal -- pick a commit and the standalone Git Commit page opens with the diff.
- **Uncommitted Changes page** with an inline commit form and per-file selection. No more "open a modal to commit" dance.
- **Background-task indicator** for merges, rebases, pulls. Long-running git ops kick off through a shared background-task store, the indicator shows what's running, and quitting while something is in flight asks first.
- **Sibling-aware dashboard** -- the streams card compares siblings against the current stream's branch, and Merge In / Rebase Onto buttons disable when there's nothing ahead.
- **Mid-merge edits don't trigger filing enforcement.** Conflict resolution is the merge commit; it would have deadlocked otherwise.

## Wiki notes grew up

The notes surface in 0.1 was a placeholder. In 0.2 it's a proper subsystem:

- **Wikilinks** for files and for git refs. `[[src/foo.ts]]` opens the file page; `[[abc1234]]` opens the commit page.
- **Mermaid diagrams** render inline.
- **Per-thread tabs** -- each thread has its own active note tab, so two threads aren't fighting over which note is open.
- **Per-thread attribution** in the rail's Finished list, so you can tell which thread captured what.

## Agent loop, simplified

0.1 had a four-state agent status (idle / thinking / waiting / blocked) and a `human_check` work-item state for "I think I'm done, please review." Both turned out to be busywork:

- **Two states: working / waiting.** That's all the rail needs.
- **`human_check` is gone.** Agents mark items `done` directly. The rail's Finished section is the review surface; the agent doesn't need a halfway state.
- **Wait points and commit-points are gone.** The shared timeline I shipped in 0.1 was clever and almost no one needed it -- including me. Pulled out, along with auto-commit. Commits are just commits again.
- **Filing enforcement moved to a PreToolUse hook.** It now blocks the offending edit at invocation time instead of nagging at end-of-turn. Mid-merge edits and wiki notes are exempt. Mid-turn user prompts are recognized as a new-ask boundary so a follow-up doesn't silently expand the current item.
- **Op errors no longer use `window.alert`.** They go to a per-thread op-error log and surface in the rail HUD, and clicking opens them in a new tab instead of replacing your current one.

## Smaller things worth calling out

- **Local History snapshots** anchor on effort close instead of every turn -- less noise, same rollback story.
- **Code Quality page** with lizard (complexity) and jscpd (duplication) running as background scans, findings persisted.
- **External URL tabs** -- a sandboxed webview tab type with a scheme allowlist, partition lockdown, CSP, and right-click "Open in browser." Useful for keeping a Linear ticket or a doc open next to the work without leaving the app.
- **Persisted page-visit tracking** powers the rail's History section.
- **New Stream can adopt an existing worktree**, and new worktrees get created as siblings of the repo rather than buried under `.oxplow/`.
- **Truecolor in agent ptys** (`COLORTERM=truecolor`) so agent output renders the way it does in a normal terminal.
- **Drag + right-click insert into the agent terminal** for files and selections -- you can drop a path into the prompt without retyping.
- **Backlinks indexer** so pages know who points at them.

## Where it stands

I've been dogfooding Oxplow development in Oxplow now, so I see all the places it can be improved. Still a lot, and major reworking coming for 0.3. But still an interesting release.

If you upgrade and something breaks -- or some piece of the new shell is just confusing -- file it at [github.com/nvoxland/oxplow](https://github.com/nvoxland/oxplow). The bar is still low.
