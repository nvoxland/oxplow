---
date: 2026-05-19
categories:
  - Releases
---

# Oxplow 0.4 -- Local History, Improving task and wiki UI

0.4 is more consolidation than expansion. The big user-visible additions: a Local History dashboard (replacing the old SnapshotsPanel) with full per-snapshot detail pages, Improved task pages backed by a better editor that the wiki uses too, and improvements to how local snapshots are managed.

<!-- more -->

> Efforts were removed on 2026-11-10. This post is kept as written; where it says "effort", read the requests and turns that replaced them.

## Local History is a page now

The old SnapshotsPanel was a bottom-rail tool window with a flat list. It's gone, replaced by a Local History page that treats snapshots as first-class content:

- **Dashboard view** with grouped rows -- by recent activity, or by the git commit each snapshot pins to. The "By git commit" view renders branch and tag chips in each group header. Uncommitted snapshots get their own group.
- **Per-snapshot detail page** with full ChangeAnalysisPanel parity -- the same Look here first card, function churn, duplication, treemap, status filters that GitCommitPage has. Snapshots aren't a poor cousin of commits anymore.
- **Effort awareness on both ends.** Local History rows show which efforts were in-flight or just-completed in a snapshot's window. SnapshotDetailPage has an "Efforts in progress at this snapshot" section. Wiki edits get a badge on the dashboard row.
- **Prev / next navigation** on the detail page. Per-snapshot history dropdown in FilePage chrome so you can jump between captures of one file without leaving the page.
- **File-page integration**: open any tracked file and the chrome dropdown lists every snapshot of that path, with a one-click jump.

Under the hood the capture pipeline was rewritten:

- **Per-stream and request-driven.** `SnapshotCaptureService` is a dirty-set manager now, not a fs-watch-fires-write loop. Concurrent `request_snapshot()` callers await the same in-flight result instead of stampeding. Captures are stream-aware (`stream_id NOT NULL`) so worktrees don't bleed history into each other.
- **Faster cold sweep.** Startup walks short-circuit on `(size, mtime)` match against the prior capture. Files that fall through get rayon-fanned through xxh3-128 hash + blob write in parallel. The phase-2 results stage and flush as a single transaction.
- **Cleaner pins.** Snapshots taken against a clean worktree get pinned to the current git commit SHA. HEAD moves re-stamp the latest snapshot. Snapshot bracket FK was wrong in V13-V16 (referenced `file_snapshot` when it stored `snapshot.id`); V17 rebuilds the table with the right target.
- **Honest about ephemera.** Transient temp files get suppressed by lifetime, not regex patterns. The settle gate makes the capture wait out the fs-watch debounce window before draining. Mermaid no longer drops its "syntax error" bomb on the outer pane.
- **Storage management.** 24-hour pruning of old snapshot rows and GC of orphaned blobs. The HUD shows sweep and cleanup runs.

## Tasks read like documents now

The old task detail view was a settings form: uppercase TITLE / DESCRIPTION / ACCEPTANCE labels above boxed inputs. That's been replaced.

- **Web-style title + description.** Large inline H1 for the title, no labels, click anywhere to type. Description is a real Tiptap editor with prose typography from the wiki, debounced save while typing, blur-commit on focus loss.
- **Right rail became pills.** Status and priority are colored pills that open a popover; category and tags are inline-edit chips. Created/Updated/By live as a compact footer.
- **Activity reads as a heading.** "Activity" renders as the same H2 styling the wiki uses, with each effort below it as a self-contained card -- header strip with timestamps and `+a ~b -c` file counts, body with clickable paths and the effort's summary rendered as markdown.
- **`acceptance_criteria` is gone.** It was set by agents, included in the dispatch prompt, but never actually scanned by `complete_task`. The single field added more surface area than it bought. Agents are nudged via the `create_task` docstring to include a `## Acceptance criteria` subsection in the description when it would help. Migration V18 folds any existing AC text into the description so nothing is lost.
- **Category and tags moved off the editable surface** for tasks attached to threads. They still exist for backlog grooming where the bulk-bucket UI needs them.

## Wiki pages use the same editor

The wiki was a textarea-vs-MarkdownView toggle. Now it's always-on Tiptap, with custom extensions to preserve everything the read view supports:

- **MermaidBlock node** -- renders the SVG when the caret is outside the block, raw editable code when the caret enters. Round-trips as a `mermaid` fenced code block.
- **InternalLink mark** allows the `file:` / `dir:` / `gitcommit:` URL schemes through Tiptap's URL sanitizer. Clicks route through `useOptionalPageNavigation` like the read view.
- **Wikilink round-trip.** `[[ ]]` syntax survives a parse + serialize cycle: `preprocessWikilinks` converts to standard markdown on load, `postprocessWikilinks` collapses `file:` / `dir:` / `gitcommit:` markdown links back to `[[ ]]` form on save. Bare vs labeled forms (`[[path]]` vs `[[path|label]]`) are preserved.
- **Page-kind icons** on every editor link so a `file:` link doesn't look like an external URL.
- **Wiki freshness signal.** Each wiki page is grounded to a content version (a git ref + a local snapshot id when applicable). The freshness badge knows when referenced files have changed since the page was last edited.
- **Wiki link text resolves to page titles** for bare `[[slug]]` links instead of showing the slug.

## Effort attribution that catches itself lying

When the agent calls `complete_task` with `touched_files`, the runtime now diffs that list against the snapshot bracket the effort actually ran against:

- **`compute_effort_file_review`** returns the set of files the agent claimed but the diff didn't see, and the set of files the diff saw but the agent didn't claim (capped at 10 to avoid wall-of-paths).
- **A Stop-hook directive fires** when there's a discrepancy, prompting the agent to either `amend_effort(add_files=..., remove_files=...)` to reconcile, or stay silent if the original claim was right (the prompt won't repeat after silent agreement).
- **`amend_effort` is a real MCP tool.** Agents can adjust an effort's file-attribution list after the fact. The acknowledged disclaim/claim markers persist so reviewers can see what was reconciled.
- **`record_effort` merges into the lifecycle effort** instead of opening a parallel one. The in_progress → done transition opens a single effort, snapshot-pinned on both ends, and `complete_task` just adds to it.
- **TaskImpact** is a new optional param on `complete_task` for the agent to record cross-page outcomes (filed a wiki page, opened a follow-up task, etc).

## Change Analysis grew architectural zones

Building on the dashboard from 0.3:

- **Zone classifier.** Files get tagged with an architectural zone (backend / frontend / config / docs / tests / build). The treemap groups cells by zone so a 40-file change reads as "mostly frontend, one config edit" instead of a wall of paths.
- **Import deltas.** Per-file added / removed import edges, classified as resolved (target exists) or unresolved (dangling).
- **Co-change surprise.** Files that historically changed together but didn't here -- and files that changed here but historically don't co-change with the rest of the diff. Both rank in a "surprise" view.
- **Theming + readability pass** on the three new cards: code chips lift off the surface, muted text darkens for dark-mode contrast.

## Generated-paths config

The hardcoded fs-watch ignore list (`node_modules`, `target`, `.next`, ...) is gone. There's a `generated` config key in `oxplow.yaml` now:

- **Single-segment entries** match anywhere in the path (`target` filters every `target/`).
- **Path entries** match exact-or-prefix (`apps/desktop/dist` filters that one directory, not unrelated `dist/` elsewhere).
- **One filter, every consumer.** Snapshot capture, the startup sweep, code-quality scans (both metrics and duplication corpus), snapshot list IPCs, effort change-paths reads -- all honor the same `WorkspaceFilter`.
- **Configurable through the file tree.** Right-click a directory -> Mark as generated. Path entries you can edit in Settings or directly in `oxplow.yaml`.
- **Read-time filtering too.** Snapshots captured before a path was added to `generated` are filtered out of the UI list views, diffs, and effort file-reviews on every read. No backfill required.

Defaults are now minimal: `.git` (segment) plus `.oxplow/` with a `.oxplow/wiki/` carve-out. Everything else is the user's call -- including build dirs, which used to be hardcoded. Existing setups will want to add `target`, `node_modules`, etc explicitly.
\