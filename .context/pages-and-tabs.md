# Pages and tabs


What this doc covers: the per-thread tab store, the shared `Page` chrome,
the page-ref id format, and the rail HUD that drives navigation. This is
the substrate the IA redesign was built on; the old dock chrome is gone
(see "Left dock removed" / "Bottom dock removed" below) and the rail HUD
+ pages are now THE shell.

## Mental model

- **Streams** = parallel worktrees (top-level tabs). Unchanged.
- **Threads** = independent lines of thought within a stream (second-row
  tabs). Unchanged.
- **Each thread owns its own set of open tabs and an active tab.**
  Switching threads restores its tab set; switching streams swaps to the
  selected thread of the new stream. The agent terminal is always
  available per thread and survives switches.
- A **page** is anything addressable inside a tab body — file, task,
  wiki page, finding, dashboard, settings, agent terminal. Pages share a
  common chrome (header + collapsible Backlinks panel).

## Modules

| File | Purpose |
|---|---|
| `apps/desktop/src/tabs/tabState.ts` | `createTabStore()` — per-thread tab list + active id, with `openTab`, `ensureTab`, `activate`, `closeTab`, `subscribe`. In memory; no cross-restart persistence in v1. |
| `apps/desktop/src/tabs/useTabStore.ts` | `getTabStore()` singleton + `useThreadTabs(threadId)` hook backed by `useSyncExternalStore`. |
| `apps/desktop/src/tabs/pageRefs.ts` | Stable id helpers: `agentRef()`, `fileRef(path)`, `diffRef({...})`, `wikiPageRef(slug)`, `taskRef(id)`, `findingRef(id)`, `indexRef(kind)`, `dashboardRef(variant)`. Centralizing the format keeps cross-component links and ⌘K open-by-id stable. |
| `apps/desktop/src/tabs/Page.tsx` | Shared page chrome: title + kind chip + status chips + actions slot, optional **browser-style nav bar** (back/forward + bookmark + backlinks dropdown — auto-mounted from `PageNavigationContext` when present), body, collapsible legacy Backlinks region. Title can be passed as a `title` prop or registered programmatically by the page via `usePageTitle`; the chrome falls back to the context title when `title` is omitted. `showNavBar` / `showHeader` flags (default true) let a page opt out — agent-style bare content sets both false. **Body layout** is chosen via `layout?: "full" \| "details"` (default `"full"`); details layout pairs a full-width center column with a 320px sticky right rail (`rightRail` prop) that auto-hides below ~960px body width — purely responsive, no manual toggle. Reads only semantic CSS variables (skin via theme). |
| `apps/desktop/src/tabs/PageNavBar.tsx` | Dumb nav-bar component: back/forward buttons, optional bookmark toggle, optional backlinks/outbound/snapshots dropdowns (popovers), and an optional **comment navigator** slot (`comments` ReactNode, rendered before Backlinks). Mounted by `Page` when context or explicit `navBar` prop is present. |
| `apps/desktop/src/components/Comments/CommentNavigator.tsx` | Self-contained per-page comment navigator for the nav bar. `useCommentsForTarget(kind,id)` → shows "Comments (N)", steps through anchored comments with ◀ ▶ (each `requestCommentReveal`s to scroll + open inline on the surface), and a dropdown lists all comments plus an **Orphaned (M)** section. Orphaned entries are clickable too: they `requestCommentReveal`, and the surfaces open the thread popover at a fallback position (no anchor to scroll to) so the comment is readable. The popover then offers a **"Relink to selection"** button (`CommentPopover.onRelink`, wired by RichTextField + MonacoCommentLayer) that re-attaches the comment to the editor's current selection — select the intended text first. (The right-click "Relink orphaned" path still exists.) Comment-bearing pages pass it via `Page`'s `commentsNav` prop (FilePage → file/path, WikiPage → wiki/slug, TaskPage → task/id). Renders nothing when the page has no comments. Pure helpers (`partitionPageComments`, `stepComment`) live in `pageCommentNav.ts` (unit-tested) and are shared with the per-thread stepper: `CommentPopover` takes an `onStep(dir)` prop so an open comment thread shows ◀ Prev / Next ▶ buttons that scroll to + reopen the adjacent comment (wired by RichTextField + MonacoCommentLayer via `stepComment` + `requestCommentReveal`). |
| `apps/desktop/src/tabs/PageNavigationContext.ts` | React context exposing `{ navigate(ref, { newTab? }), goBack, goForward, canGoBack, canGoForward, setTitle, title }` to descendants of an active page tab. Wrapped around every non-agent center tab in `App.tsx`. `BacklinksList` reads it so default-click navigates in-tab. The `usePageTitle(title)` helper registers the page's current title with the host so the same string drives the chrome header AND the tab strip label — no per-page duplicate header markup. |
| `apps/desktop/src/pages/FilePage.tsx` | Thin Page wrapper around `EditorPane`. Calls `usePageTitle(basename + ● dirty)` so the file's name flows into the shared chrome title, and `useBacklinks(fileRef(path))` so wiki pages, tasks, commits, and findings that reference the file appear in the nav-bar Backlinks dropdown. EditorPane keeps owning Monaco / blame / context menus; the wrapper only provides chrome above. |
| `apps/desktop/src/pages/DiffPage.tsx` | Thin Page wrapper around `DiffPane` for diff tabs. Calls `usePageTitle(basename + (label))`. |
| `apps/desktop/src/tabs/RouteLink.tsx` | Browser-style link button + the `useRouteDispatch(ref, { onNavigate?, pinnedSlot? })` hook that powers it. Click semantics: left-click → in-tab navigate via `PageNavigationContext` (or `onNavigate` fallback when no context, e.g. rail / palette), Cmd/Ctrl-click + middle-click + right-click → new tab. The hook returns `{ dispatch, handlers }` so non-button rows (file tree entries, note rows, …) can adopt the same semantics without becoming a `<button>`. |
| `apps/desktop/src/components/RailHud/RailHud.tsx` | Persistent left rail HUD: search trigger, active item, up next, **bookmarks** (when present), recent files, pages directory. Passive — never auto-opens tabs. Bookmark rows show a single-letter scope badge (T/S/G) and a per-row remove button. |
| `apps/desktop/src/components/Navigator.tsx` | Far-left combined **stream + thread navigator**: a 40px always-visible strip of letter glyphs; hovering slides a `navigator-overlay` (~280px) over it (and over the rail HUD to its right) re-rendering the same rows with full titles. The overlay dismisses on mouseleave (180ms grace), Escape, **or any pointerdown outside it** — the last rule is load-bearing because the open overlay covers the rail and the wrapper's `mouseleave` can't fire while the pointer sits over the rail-covered region (tsk131). Each overlay row (`navigator-stream-row-<id>` / `navigator-thread-row-<id>`) opens its action menu on **right-click** (Menu key / Shift+F10 for keyboard). **Thread menu:** a read-only (non-writer / queued) thread leads with **"Make writer"** (`menu-item-thread.promote`) → calls the `promote_thread` IPC (via `onPromoteThread` → `App.handlePromoteThread`), making it the stream's single active writer and demoting the prior one; the active writer omits the item (its accent pill already signals it). Then Rename / Settings / Close. The write guard makes every non-active thread read-only, so this is the discoverable path out of "project file edits are blocked" (tsk132). |
| `apps/desktop/src/tabs/bookmarks.ts` + `useBookmarks.ts` | Per-scope (thread / stream / global) bookmark store backed by localStorage. Pages bookmark via the `PageNavigationContext.bookmark` binding; the rail HUD reads the merged set. |
| ~~`apps/desktop/src/tabs/appPageBacklinks.ts`~~ | **Deleted.** Per-kind in-memory backlinks providers used to live here. Cross-page backlinks now come from the persisted `page_ref` graph (`crates/oxplow-db/src/page_ref_store.rs`) via the `list_backlinks` IPC; every page kind goes through the same code path. App pages that need their own provider would register a new `source_kind` writer in the backend instead. |
| `apps/desktop/src/pages/GitCommitPage.tsx` | Single-commit page (`git-commit:<sha>`). Reuses `CommitDetailBody` (now exported from `CommitDetailSlideover`). Routed via `gitCommitRef(sha)`. Bookmark-/history-friendly alternative to the slideover. |
| `apps/desktop/src/components/RailHud/sections.ts` | Pure helpers: `computeActiveItem`, `computeUpNext`, `sortRecentFiles`, `computePagesDirectory`. The pages directory is a pure function so it can be unit-tested without mounting the React rail. |
| `apps/desktop/src/pages/GitDashboardPage.tsx` | Committed-history rollup: branch header (current branch + upstream + ahead/behind + push), small uncommitted mini-card that links to `UncommittedChangesPage`, recent commits rendered through the shared `CommitGraphTable` (last 5, current branch only via `getGitLog({ all: false })`; click a row → reveal in `GitHistoryPage`), worktrees row with per-row "Merge into current", a **"Merge readiness" card** (cross-stream divergence vs the integration branch via `listStreamDivergences()` — per-stream ahead/behind + a clean/will-conflict/integrated badge, naming the overlapping files on conflict, with a one-click "Merge into &lt;base&gt;" offered only while viewing the base stream), recent remote branches with per-row pull/push. All ref-mutating actions confirm the exact `git` command before running. Routed via `gitDashboardRef()`. |
| `apps/desktop/src/components/History/CommitGraphTable.tsx` | Pure presentation of the git-log graph (branch/merge dots + lines + sha + ref badges + subject + author + relative date). Used by both `HistoryPanel` (full list with detail pane) and `GitDashboardPage`'s recent-commits card. `indexRefsBySha(log)` exported alongside groups branch heads + tags by sha so callers feed identical maps. |
| `apps/desktop/src/pages/UncommittedChangesPage.tsx` | Stats-focused view of working-tree changes: M/A/D/R/U + total +/-, collapsible folder tree with per-folder rollup of files / +/-, Commit-all action. Distinct from `FilesPage` which is the full project file tree. Routed via `uncommittedChangesRef()`. |
| `apps/desktop/src/pages/ChangeAnalysisPage.tsx` | Two-mode page: **dashboard** (no scope) shows summary + clickable file pivots (extension / directory / status); pivot rows route to a focused **drilldown** (`changeAnalysisRef(target, scope)` where `scope` is `{ kind: 'ext'\|'dir'\|'status', value }`). Drilldown reuses the same hook with the scope applied and renders the scoped summary, a semantic / file-list view toggle, an added/modified/deleted/all status filter, plus duplication + tests cards (relocated off the dashboard). Shared chrome: `ChangeAnalysisHeader` (Parent vs … / Refresh / Open commit) renders on every variant. The drilldown body lives in `components/ChangeAnalysis/ChangeAnalysisDrilldown.tsx`. All data assembled on demand via `useChangeAnalysis({ streamId, target, scope? })`; no new tables. |
| ~~`apps/desktop/src/tabs/backlinksIndex.ts`~~ | **Deleted.** The in-memory cross-kind indexer is replaced by the persisted `page_ref` table; see `data-model.md`. The `BacklinkEntry` type that renderers consume now lives in `apps/desktop/src/tabs/backlinkTypes.ts`. |
| `apps/desktop/src/tabs/useBacklinks.ts` | React hook that calls the unified `list_backlinks` IPC for a `TabRef` and maps the returned `BacklinkEdge` rows into `BacklinkEntry`s. Used by every page kind including `FilePage` (which previously rendered nothing). The sibling `usePageOutbound` hook does the same for the inverse direction. |
| `apps/desktop/src/tabs/backlinkTypes.ts` | Renderer-side `BacklinkEntry` interface (`{ ref, label, subtitle? }`). Decoupled from the SQLite `BacklinkEdge` shape. |
| `apps/desktop/src/tabs/BacklinksList.tsx` | Default renderer for the Page chrome's `backlinks` slot — buttons that route via `onOpenPage`. |
| `apps/desktop/src/pages/TaskPage.tsx` | Single-record page for a task — wraps `TaskDetail` + `ActivityTimeline`. Backlinks computed via `useBacklinks`. |
| `apps/desktop/src/pages/NotePage.tsx` | Single-record page for a wiki page — wraps `NoteTab`. The `note:<slug>` center-tab is rendered through this Page wrapper so notes get the unified chrome (title from `usePageTitle`, browser-style back/forward + star, Backlinks panel). `NoteTab` no longer renders its own header — freshness badge + Edit/Save/Revert/Delete/Create live in a thin secondary toolbar inside the body. In-tab wikilink-to-note clicks route through `PageNavigationContext.navigate(wikiPageRef)` so they participate in tab-level history. |
| `apps/desktop/src/pages/FindingPage.tsx` | Single-record page for a code-quality finding — kind/path/line range/metric + source snippet + "Jump to source". |
| `apps/desktop/src/pages/CommentsInboxPage.tsx` | Global **Comments Dashboard** (`comments` index kind, `commentsRef()`; titled "Comments Dashboard"). Lists every comment in the current stream (`listCommentsForStream`), grouped by target. The body of a **row opens that comment's full `CommentPopover` inline** (read/reply/intent/resolve/delete — triage the whole backlog from one place); a **group-header click jumps to the target page** (file/wiki/task). Each row also has a **"Go to location" button** that navigates to the target page *and* scrolls to / opens the anchored comment via `comment-reveal-bus.ts` (disabled for orphaned comments). **Defaults to unresolved threads only**; a "Show" dropdown reveals resolved threads bucketed by recency. The bucket thresholds are a tiered ladder — one per day to a week, one per week to a month, then one per month beyond — capped at the oldest actual resolved comment so the largest option reaches all of them (helpers in `comments-filter.ts`: `resolvedWindowOptions` / `visibleThreads`, keyed on `comment.resolved_at`). The holistic "review them all" surface; the agent reaches the same data via the `list_comments` MCP tool. |
| `apps/desktop/src/pages/DashboardPage.tsx` | Composite Planning / Review / Quality dashboards. Variant chosen via `dashboardRef("planning"\|"review"\|"quality")`. |
| `apps/desktop/src/pages/StreamSettingsPage.tsx` | Per-stream settings page (custom prompt). Replaces the in-rail StreamRail settings modal. Routed via `streamSettingsRef(streamId)`. |
| `apps/desktop/src/pages/ThreadSettingsPage.tsx` | Per-thread settings page (custom prompt). Replaces the in-rail ThreadRail settings modal. Routed via `threadSettingsRef(threadId)`. |
| `apps/desktop/src/components/Slideover.tsx` | Right-edge panel primitive (~38vw, backdrop-click + Escape close, focus-into-panel on open) for form-shaped flows that don't justify a full page. Use instead of a centered modal. |

## Page kinds

`PageKind` (`tabState.ts`):

```
"agent" | "file" | "diff" | "duplicate-block" | "note" | "task" | "finding"
| "tasks" | "done-work" | "backlog" | "archived"
| "wiki-index" | "files" | "comments" | "code-quality" | "terminal"
| "local-history" | "git-history" | "git-dashboard" | "git-commit"
| "uncommitted-changes" | "change-analysis" | "hook-events"
| "settings" | "start" | "dashboard"
| "new-stream" | "new-task"
| "stream-settings" | "thread-settings"
| "op-error"
| "external-url"
```

`agent` is implicit per thread. The `*-index` kinds are full-page
versions of what today are left-rail or bottom-drawer panels.

## Tab id format

| Kind | Id format | Example |
|---|---|---|
| agent | `agent` | `agent` |
| file | `file:<path>` | `file:crates/oxplow-app/src/lib.rs` |
| diff | `diff:<path>\|<from>\|<to>\|<labelOverride>` | `diff:src/a.ts\|abc\|def\|` |
| duplicate-block | `dup:<leftPath>:<lstart>-<lend>::<rightPath>:<rstart>-<rend>` | `dup:src/a.ts:10-40::src/b.ts:55-85` |
| note | `note:<slug>` | `note:how-stop-hook-fires` |
| task | `wi:<id>` | `wi:wi-142` |
| finding | `finding:<id>` | `finding:f-7` |
| `*-index` | the kind name | `code-quality`, `comments`, `start`, `settings` |
| git-dashboard | `git-dashboard` | `git-dashboard` |
| git-commit | `git-commit:<sha>` | `git-commit:abc1234567890` |
| uncommitted-changes | `uncommitted-changes` | `uncommitted-changes` |
| change-analysis | `change-analysis:<target>` (dashboard) or `change-analysis:<target>:<scopeKind>:<scopeValue>` (drilldown — `scopeKind` is `ext` / `dir` / `status`) | `change-analysis:working`, `change-analysis:abc1234:ext:rs` |
| dashboard | `dashboard:<variant>` | `dashboard:planning` |
| new-stream | `new-stream` | `new-stream` |
| new-task | `new-task` | `new-task` |
| stream-settings | `stream-settings:<streamId>` | `stream-settings:s-7` |
| thread-settings | `thread-settings:<threadId>` | `thread-settings:t-3` |
| op-error | `op-error:<errorId>` | `op-error:oe-abc123` |
| external-url | `external-url:<url>` | `external-url:https://example.com/path` |

## Body layouts

`<Page>` takes a `layout?: "full" | "details"` prop (default `"full"`).
The tab-level chrome (nav bar, header, legacy backlinks footer) is the
same for both layouts — only the body region differs.

- **`"full"`** — today's behavior. Body region is `flex: 1; overflow:
  auto` with no width cap and no padding; children own their own
  padding. Dashboards, lists, history tables, the agent terminal,
  file/diff editors, and anything that wants every available pixel
  stay full.
- **`"details"`** — two-column CSS grid: a **bounded reading-width
  center column** plus a `320px` sticky right rail (`position: sticky;
  top: 0`). Outer padding `24px`, gap `24px`. The center column carries
  the `.oxplow-reading-column` class (defined in `index.html`), which
  owns the measure: `max-width: 78ch; margin-inline: auto`. The layout
  owns the width so **children just fill it and don't manage their own**
  — that class also cancels the per-leaf self-cap on `.oxplow-md` /
  `.oxplow-rt-field` (which otherwise self-center at `78ch` for
  unbounded contexts), so the title, description, and activity share one
  left edge and width instead of looking scattered (title flush-left,
  body floating centered). The page provides rail content via the
  `rightRail` prop; the layout owns padding and sticky positioning.

  Reuse `.oxplow-reading-column` for any future bounded reading area —
  it's the single primitive that expresses "a fixed-measure column that
  everything inside lives within."

The rail is **purely responsive — no user toggle**. A `ResizeObserver`
on the body container watches its width; below `960px` the rail is
unmounted entirely and the grid collapses to one column. There is no
persistent collapsed state. Pages that want a rail but have no rail content for some
condition just pass `rightRail={undefined}`; the layout treats that
identically to a sub-threshold viewport.

**Opt-in rule:** detail pages (single record — wiki page, task,
finding, …) use `layout="details"`. Lists / dashboards / editors stay
on `"full"`. Adopters today: see WikiPage and TaskPage migrations.

## Rail HUD contract

The rail is **read-only with respect to tabs** — it never auto-opens a tab.
Every rail click goes through a single `onOpenPage(ref: TabRef)` callback
that the host wires to its own routing. Sections are a flat stack (no
zone grouping); each appears only when it has content:

1. **Search trigger** — opens the launcher (`QuickOpenOverlay`), the
   single discovery surface. Always visible.
2. **Work** — `WorkSection`. A single section, header always **"Work"**;
   item status is conveyed by the leading glyph rather than the title — a
   `◐` in-progress icon when working, a `✓` done check on the last
   finished item. The expand **chevron lives on the header** (styled like
   the other `SectionHeading`s); default collapsed, state persisted in
   `localStorage` (`oxplow.rail.workExpanded` via `useRailWorkExpanded`).
   Collapsed and expanded are two distinct render branches (not one row
   set restyled):
   - **Collapsed:** a compact one-liner — the live Active item
     (`ActiveItemSection`, with its epic-children expander when the
     active item belongs to an epic) when working; otherwise just the
     most recent finished item (`SingleFinishedRow`, ✓ for a done task /
     wiki icon for a note).
   - **Expanded:** the full work picture as labeled sections —
     "In progress" (the active item, when present) + `UpNextSection`
     ("Ready") + `FinishedSection` ("Finished"), inline; no jump to the
     Tasks page.
3. **Uncommitted** — working-tree summary; hidden when clean.
4. **Comments** — `CommentsSection` self-fetches
   `listCommentsForStream(streamId)` (kept live via
   `subscribeCommentEvents`) and shows two open-comment count rows split
   by intent — "For me" (`note`) and "For the agent" (`followup`) — each
   opening the Comments inbox (`commentsRef()`).
5. **Errors** — `OpErrorsSection`; hidden when empty.
6. **Recent files** — top 6 file paths recently opened/touched in this
   thread.
7. **Bookmarks** — the user-curated pinned set. There is **no "Pages"
   section**: the launcher is the one discovery surface for all pages,
   and users pin what they want always-visible by starring (the ☆ on each
   page's nav bar, scoped thread / stream / global).
   `computePagesDirectory` is the launcher's page list (the old
   `RAIL_PAGE_IDS` filter is gone); every page carries a `category` (Work
   / Code / Git / Activity / Knowledge / System) so the launcher empty
   state reads like a start menu.
8. **History** — recent / most-visited pages.

## Migration status

The full IA redesign ships in phases (see plan
`/Users/nvoxland/.claude/plans/the-ui-is-very-delightful-badger.md`):

- ✅ Phase 0 — Theme foundation (`.context/theming.md`).
- ✅ Phase 1 — Tab store + page chrome + page refs (this doc).
- ✅ Phase 2 — Rail HUD shell (this doc).
- ✅ Phase 3 — Page migration: every rail HUD "Pages" entry now opens
  a Page-wrapped renderer in `apps/desktop/src/pages/`:
  Start, Settings, Code quality, Local history, Git history, Files,
  Notes, All work. Both docks have since been removed
  — the rail HUD is THE left chrome and pages are THE center surface
  (see "Left dock removed" / "Bottom dock removed" notes below).
- ✅ Phase 4 — New pages + backlinks indexer:
  `TaskPage`, `NotePage`, `FindingPage`, three `DashboardPage`
  variants (Planning / Review / Quality), and the
  `computeBacklinks(target, ctx)` indexer. `FilePage` and `DiffPage`
  were intentionally skipped: file and diff tabs already render via
  `centerTabs` with their own chrome (Monaco editor, diff editor) and
  wrapping them in Page chrome would double-up the header. The
  legacy `note:` tab path now renders through `NotePage` so wiki
  notes get a Backlinks panel; modal-based task edits still work
  alongside `TaskPage` for callers that want the modal flow.
- ↩️ **Reversal (tsk168, 2026-06):** phase 5c's kebab `⋯` migration was
  undone — per-row actions are **right-click only** again (right-click is
  discoverable enough and the kebab ate row space). `Kebab.tsx` is
  deleted; the shared hook is now `useRowContextMenu.tsx`
  (`useRowContextMenu(items)` / `useContextMenu()`). The global
  suppressor in `context-menu.ts` stays as a backstop; editing surfaces
  keep their native/own menus. See `.context/usability.md` →
  "Per-row actions (right-click menus)". The 5c log below is historical.
- ✅ Phase 5 — Web-style interactions sweep (kill modals + right-click
  menus). 5a (`InlineConfirm` + `UndoToast` queue) and 5b (`InlineEdit`
  + `InlinePromptStrip` for new-X flows) shipped: `ConfirmDialog.tsx`
  and `PromptDialog.tsx` are deleted. 5c (Kebab popovers) shipped on
  the high-traffic surfaces (StreamRail, ThreadRail, CenterTabs,
  WorkGroupList rows, Notes pane rows, FileTree rows) plus the
  remaining holdouts: BranchPicker manage rows (chevron-led row click,
  no `onContextMenu`), EditorPane git-blame margin (hover-revealed
  per-row kebab), MarkdownView links (inline hover-revealed kebab
  next to each link), WikiActivityBar entry pills + overflow rows
  (per-row kebab), TerminalPane (xterm `contextmenu` listener
  removed; header-bar kebab with Copy/Paste/Clear). 5d landed the
  `Slideover` primitive (`apps/desktop/src/components/Slideover.tsx`) plus the
  BranchPicker rename Slideover, ProjectPanel commit-dialog
  Slideover, and the cross-page detail wrappers
  `SnapshotDetailSlideover` (`apps/desktop/src/components/Snapshots/SnapshotDetailSlideover.tsx`)
  and `CommitDetailSlideover` (`apps/desktop/src/components/History/CommitDetailSlideover.tsx`).
  5e landed the per-stream and per-thread settings as
  `StreamSettingsPage` and `ThreadSettingsPage`, the inline-new-row
  that retired `CreateThreadModal`, and the new-stream / new-work-
  item page-form replacements (`NewStreamPage`, `NewTaskPage` —
  routed via `newStreamRef()` / `newTaskRef({...})`). The
  `PlanPane` `NewTaskModal` only backs the edit-double-click
  flow now; new flows route through pages.
- ✅ Phase 6 — Selection action bar + drag-to-add-context polish.
  `SelectionActionBar` (`apps/desktop/src/components/Plan/SelectionActionBar.tsx`)
  appears at the top of `PlanPane`'s work-group region whenever ≥1
  rows are marked. It owns no state; PlanPane reads its existing
  marked-set and routes Change status / Change priority / Add to
  agent context / Delete through the same paths used by single-row
  kebabs. The agent terminal now accepts multi-row task drags
  (decodes the `WORK_ITEM_DRAG_MIME` payload's `items` slice
  directly — see `.context/usability.md` "Add to agent context").
  Drag-to-add sources expanded: BacklinksList entries, RailHud
  recent-files / active item / up-next, CodeQualityPanel file group
  rows, plus a "Add to agent context" item on every task kebab
  (single-row and group menus).
- ✅ Phase 7 — Density + visual polish. Body font bumped to 14px;
  list rows (Plan / Files / Notes / Code quality / Snapshots /
  History) raised from ~24–28px to ~36–40px; section headers use
  `--surface-app` + 10px padding; CenterTabs strip is 36px min-height;
  Page chrome header is 56px with a 17px / 600-weight title; legacy
  unknown `--color-*` fallback hexes (NotesPane, NoteTab,
  WikiActivityBar, MarkdownView, TerminalPane drag overlay) migrated
  to the semantic tokens; selection/marked rows use a 3px stripe +
  `--accent-soft-bg`. Monaco editors are pinned to `vs-dark` (oxplow
  is dark-only). See `.context/theming.md` Density section.

Phase 3 is shipped: rail HUD "Pages" entries open as full center-area
tabs. (The rail "Pages" section itself was later removed — see
"One Search" below — but the underlying page-as-tab routing is unchanged;
the launcher and Bookmarks now drive navigation instead.)

**Left dock removed.** The left-side `DockShell` that previously
carried four toolwindows (HUD / Work / Files / Notes) is gone.
`<RailHud>` is now mounted directly as a 260px-wide left aside in
`App.tsx` — the component owns its own width / `borderRight` /
`var(--surface-rail)` background, so no host wrapper is needed. The
rail HUD is THE persistent left chrome; the legacy `Plan` / `Project` /
`Notes` left-rail tabs were duplicates of the existing
the work pages (`TasksPage` / `DoneWorkPage` / `BacklogPage` /
`ArchivedPage`) / `FilesPage` / `NotesIndexPage` content and have
been deleted along with the `leftDockActivate` plumbing. Menu
commands that used to flip the dock (`commitFiles`, edit-task)
now route through `handleOpenPage(indexRef("files"))` /
`handleOpenPage(indexRef("tasks"))`. The harness startup gate
(`waitForOxplowReady`) polls for `rail-hud`.

## One Search — the single discovery surface

There is exactly **one** way to discover and reach pages: the launcher
(`QuickOpenOverlay`). It is opened by **Cmd+P** (the `file.quickOpen`
menu command) and the rail **Search…** button (`rail-search`). Cmd+K and
Cmd+Shift+F are kept only as **aliases** that open the same launcher, so
the old command-palette / find-in-files reflexes still land somewhere
useful — there is no separate command palette or search overlay anymore
(`CommandPalette.tsx` and `SearchPalette.tsx` were deleted).

The launcher does everything in one box:
- **Empty input** → start-menu: a **collapsible tree** of the page
  `category` headers (Work / Code / Git / Activity / Knowledge / System),
  **collapsed by default** so the empty launcher is a short list of
  sections rather than all ~21 pages. Expanding a category reveals its
  pages (indented). Expansion state persists per user in `localStorage`
  (`oxplow.launcher.expandedCategories`). The tree assembly is the pure
  helper `buildLauncherTree(pages, expanded)` in `quickOpenResults.ts`;
  category rows and page rows share one navigable `Row[]` so the keyboard
  cursor and render can't drift (Enter toggles a category, opens a page).
- **Typing** → ranked results, pages → **commands** → files → body hits.
  Commands come from `buildMenuGroups` (passed in as `menuGroups`) and are
  flattened by `flattenCommands`; the same registry still feeds the native
  menu. Body hits come from `searchSite` (BM25 over tasks / comments /
  wiki / notes / file contents). The ordering is a pure helper —
  `buildQuickOpenResults` in `components/quickOpenResults.ts` — so it's
  unit-tested without mounting React.

Because the launcher lists every page (including Code Quality, Hook
Events, Local History), no page needs a bespoke `CommandId` to be
reachable — that supersedes the old "wire each page as a menu command"
approach (tsk147).

The rail no longer has a "Pages" section (the `rail-page-*` / `rail-pages`
testids are gone); **Bookmarks** is the always-visible curated nav. E2e
probes that used to click `rail-page-*` should drive `rail-search` → the
launcher (type, then assert `page-<kind>` on the body).

**Work pages split (post-Phase-3).** The single `AllWorkPage` was
replaced by four focused pages so each has one job:

- **Tasks** (`page-tasks`) — thread-local task manager (formerly
  "Plan work", `page-plan-work`). Shows To Do + Blocked in full
  plus last-5 Done previews. The In Progress section is omitted
  because the rail HUD's "Active item" + "Up next" already surface
  it. Header link "View all done →" routes to Done Work; kebab
  carries the legacy `hide-auto` filter and a "View backlog →"
  entry. PageKind is `"tasks"`; ref helper is `tasksRef()`.
  `planWorkRef()` is kept as a deprecated alias for one release.
- **Done work** (`page-done-work`) — full descending list of done +
  canceled items for the current thread. Excludes archived; header
  link "View archived →" routes to the Archived page.
- **Backlog** (`page-backlog`) — global (cross-stream) candidate
  pool with a promote-into-thread action. Items are `tasks` rows
  with `thread_id IS NULL`; promote/demote flips `thread_id` without
  copying. The `backlogReadyCount` badge in the rail directory
  hangs off this entry.
- **Archived** (`page-archived`) — full descending list of archived
  items only.

All four wrap `PlanPane` and pass filter props
(`visibleSections`, `sectionItemLimit`, `onlyStatuses`,
`excludeStatuses`, `sectionLabelOverrides`, `extraSectionLinks`,
`forceMode`, `hideBacklogChip`, `hideArchiveToggle`). The
inline "Show archived (N)" toggle on the Done section header is
suppressed across all four — archive flow is owned by the
dedicated Archived page link.

The four pages reuse the shared `<Card>` + `cardLinkButton` from
`apps/desktop/src/components/Card.tsx` for cross-page "View X →" affordances;
GitDashboardPage uses the same shell so the dashboard vocabulary is
consistent across IA.

Named ref helpers — `tasksRef()`, `doneWorkRef()`,
`backlogRef()`, `archivedRef()` — mirror the GitDashboard pattern
(`gitDashboardRef`, `uncommittedChangesRef`). `planWorkRef()`
remains as a deprecated alias of `tasksRef()`.

**Bottom dock removed.** The bottom-drawer `DockShell` that previously
hosted Hook events / Git history / Local history / Code quality is
gone. Every panel it carried has a Page equivalent
(`HookEventsPage`, `GitHistoryPage`, `LocalHistoryPage`,
`CodeQualityPage`); menu commands like "Open history" / "Open
snapshots", and the cross-pane "show in history" reveal hooks
(`handleRevealCommit`, `handleShowSnapshotInHistory`), all route
through `handleOpenPage(indexRef("git-history"))` /
`indexRef("local-history")`. The `StatusBar` (background-task
indicator + branch chip) used to live as the bottom dock's `railExtra`;
it's now mounted directly at the bottom of `App.tsx` inside its own
status-bar wrapper. The `BottomPanel`, `HistoryPanel`, `SnapshotsPanel`,
and `CodeQualityPanel` modules are no longer imported from `App.tsx` —
the only callers left are inside their own page wrappers.

## Browser-style tab navigation (Phase 1)

Page tabs now carry **per-tab back/forward history**. `App.tsx` keeps
a parallel `threadPageHistory: Record<threadId, Record<tabId, { back; forward }>>`
state alongside `threadPageTabs`. When a page-tab descendant calls
`navigate(ref)` via `PageNavigationContext`, the active tab's current
ref is replaced with `ref` and the prior ref is pushed onto its back
stack — the tab id changes to `ref.id`, `centerActive` follows, and
the history entry is migrated. `goBack` / `goForward` swap the
current ref with the top of the back / forward stack.

`navigate(ref, { newTab: true })`, Cmd/Ctrl-click on a `BacklinksList`
entry, middle-click, and right-click all bypass in-tab navigation and
fall through to `handleOpenPage` (the legacy "open as new page tab"
path). Notes participate in tab-level history (they live in
`threadPageTabs` like every other page kind); diffs and files have
their own list state but still get the shared chrome wrap, so back/
forward is no-op for them but the title row + nav bar UI is the same
as everywhere else.

The bookmark toggle and backlinks dropdown affordances on
`PageNavBar` are scaffolded but currently inert; Phases 2 and 3 wire
them.

## Sibling navigation (list → page prev/next)

When a page is opened from a list (notes index, file tree, backlinks,
task list, …), `PageNavBar` renders **up/down sibling buttons**
next to back/forward. They step through the originating list without
touching back/forward — Back still goes to the page that listed the
items, never to the previously-viewed sibling.

Mechanics:

- A list registers its rows by passing `siblings: NavSiblings` into
  `useRouteDispatch(ref, { siblings })`. `NavSiblings` is
  `{ entries: Array<{ ref, label }>; index: number }`. The `label`
  shows up in the prev/next button hover tooltip.
- The dispatcher forwards `siblings` to `PageNavigationContext.navigate`
  on in-tab navigation only (new-tab escape paths drop it — sibling
  context is in-tab only).
- Per-tab history entries (`threadPageHistory`) gained a `siblings`
  field. `handleNavigateInTab` calls `resolveSiblings` to snap the
  destination's index against `ref.id`, so a stale list still lands on
  the right row.
- `handleStepSibling` swaps the active tab to a sibling at the target
  index, mutating only `siblings.index` — back/forward stacks are
  preserved.
- Back/forward navigation **clears** siblings on the destination
  entry: the back target predates the list-originated chain.
- `Page` reads `ctxNav.siblings` and constructs the nav-bar config
  (`prevLabel` / `nextLabel` from `entries[index ± 1]`, callbacks from
  `goPrevSibling` / `goNextSibling` which are only set when not at the
  edge). The `1 of N` indicator renders between the buttons.
- The indicator is itself a toggle (`page-nav-sibling-indicator`) —
  clicking it opens a popover (`page-nav-sibling-list`) listing every
  sibling entry numbered 1..N with the active row highlighted, so the
  user can jump straight to any sibling instead of stepping through
  them. Mirrors the CenterTabs overflow ▾ dropdown pattern. Wired via
  `goSibling(index)` on `PageNavigation`, which delegates to the same
  `handleStepSibling` used by the up/down buttons. Escape and
  outside-click close the popover.

Adopted lists:

- `tabs/BacklinksList.tsx` — every backlink entry passes its index in
  the merged list (snapshot/commit slideover entries are excluded).
- `components/Wiki/WikiPane.tsx` — `NoteRow` (Recently visited /
  Recently modified sections, each independent) and `SearchRow`
  (search results) accept a `siblings` prop wired through the
  pre-computed entries-with-labels.
- `components/LeftPanel/FileTree.tsx` — `TreeEntries` exposes file-row
  siblings within each directory level (excluding directories and
  deleted files).
- `components/History/CommitGraphTable.tsx` — each row dispatches via
  `useRouteDispatch(gitCommitRef(sha), { siblings, onNavigate: onSelect })`
  through a `CommitRowDispatcher` adapter, so the legacy `onSelect`
  callback survives as the rail-side fallback while the in-page path
  picks up siblings.
- `pages/DashboardPage.tsx` — `RowButton` now optionally takes
  `navRef` + `siblings` + `onNavigate`; the Planning dashboard wires
  ready / backlog / recent-notes lists.

`WorkGroupList` rows (Plan / Tasks / Backlog pages) intentionally do
NOT adopt: clicking opens the edit modal, not a page. Sibling nav
applies only to lists whose rows navigate to a page.

Future lists adopt by passing `siblings` to `useRouteDispatch` /
`RouteLink`. Lists that wrap their own click callback can either
migrate to `useRouteDispatch` directly (preferred) or use the
adapter-component pattern from `CommitGraphTable`'s
`CommitRowDispatcher`.

## Linking between tabs — single chokepoint rule

**Browser-tab semantics are non-negotiable: plain-click navigates
in-tab; only Cmd/Ctrl-click + middle-click + right-click open a new
tab.** This rule has regressed several times — every regression has
the same root cause: a list/tree row called `onOpenFile` /
`onOpenPage` directly instead of dispatching through the page-
context chokepoint, and the host's callback always opens a new tab.

**Any clickable row that targets another `TabRef` MUST go through
`RouteLink` or `useRouteDispatch`.** Don't write raw
`onClick={() => onOpenPage(...)}` / `onClick={() => onOpenFile(...)}`
on rows: that path always opens a new tab and never gets right-click
or modifier-click semantics.

When `useRouteDispatch` isn't structurally available (e.g. the row
is built inside a `useMemo` where hooks can't be called), grab the
nav context once at the top of the host component
(`const ctxNav = useOptionalPageNavigation()`) and have the row's
`onClick` call `ctxNav.navigate(ref, { newTab })`. Falling back to
the host's `onOpenFile` / `onOpenPage` callback is **only**
acceptable when no PageNavigationContext is present (rail HUD,
palette) — those callbacks always open new tabs and that's correct
for those surfaces.

The pattern, depending on the row's markup:

- Plain link (a button): use `<RouteLink ref={someRef(...)}>`.
- Existing `<div>`-based row that needs to keep its other event
  handlers (drag, double-click, right-click menu): call
  `const { handlers } = useRouteDispatch(someRef(...), { onNavigate });`
  and spread `onClick={handlers.onClick}`,
  `onAuxClick={handlers.onAuxClick}`,
  `onContextMenu={handlers.onContextMenu}` onto the row.

The hook reads `useOptionalPageNavigation()` and **prefers the
context** when it's present — that's how plain-click does in-tab
navigation inside a page. The optional `onNavigate` prop is a
**fallback** used only when no context exists (rail HUD, palette,
non-page surfaces). Rail callers pass `onNavigate` so the same row
keeps its "always-new-tab" behavior outside a page.

Reference implementations:

- `apps/desktop/src/components/LeftPanel/FileTree.tsx` — tree row dispatches
  via `useRouteDispatch(fileRef(path), { onNavigate: (_, opts) => onOpenFile(path, opts) })`.
- `apps/desktop/src/components/Notes/NotesPane.tsx` — `NoteRow` and `SearchRow`
  use the hook with `wikiPageRef(slug)` and a `() => onOpenNote(slug)`
  fallback.
- `apps/desktop/src/tabs/BacklinksList.tsx` — the older pattern (manual
  `ctxNav.navigate` + per-event new-tab branches). Both forms are
  acceptable; `useRouteDispatch` is preferred for new code.

If you're adding a new index/list page and find yourself threading an
`onOpenPage` / `onOpenFile` / `onOpenNote` callback all the way down
to a row's `onClick`, **stop and use the hook instead**. The callback
should only survive as the rail-side fallback.

## Per-thread active tab (today)

`App.tsx` holds a `Record<threadId, string> threadCenterActive` map and
derives `centerActive` from it. `setCenterActive` writes to the map for
the currently selected thread. Switching threads automatically restores
each thread's last active tab.

## Unified tab list — every tab holds a Page

Every per-thread tab lives in `threadPageTabs[threadId]` as a
`TabRef`, regardless of kind (`note`, `file`, `diff`, `task`,
`change-analysis`, `git-commit`, etc.). The page-tab loop in
`centerTabs` builds the renderer by switching on `ref.kind` and
wrapping each tab in a `PageNavigationContext` so in-tab navigation,
back/forward, sibling navigation, and bookmark/backlinks all work
the same way.

- `fileSessions[stream.id]` is now a **content + dirty-state cache**
  only. Tab membership / order is driven by `threadPageTabs`.
  `handleOpenFile` populates fileSessions and pushes a `kind: "file"`
  ref into `threadPageTabs` for the active thread.
- `diffTabs` is now a **spec registry indexed by id**. `handleOpenDiff`
  registers the spec and pushes a `kind: "diff"` ref into
  `threadPageTabs`. The page-tab renderer's `ref.kind === "diff"`
  branch looks up the spec from `diffTabs` to render `DiffPage`.
- `closePageTab` is the unified close path; it removes the ref from
  `threadPageTabs`, drops history + page-title state, and (for
  file tabs) closes the entry in `fileSessions`.
- The agent tab is the only special-case at the centerTabs level —
  it sits at slot 0, is `closable: false`, and uses `AgentPage`
  (`apps/desktop/src/pages/AgentPage.tsx`) which wraps `TerminalPane`
  inside `Page` chrome configured with `showNavBar={false}` and
  `showHeader={false}`. A future cleanup may move the agent ref into
  `threadPageTabs` too; today centerTabs prepends it deterministically.

This is the architectural rule for new tab kinds: add a `PageKind`,
add a `pageRefs.ts` helper, render through `Page`, and dispatch in
the `centerTabs` page-tab loop. **Don't** add a parallel tab track.

Files were stream-scoped historically; lifting them into the
per-thread list means each thread has its own open-file list within
a stream. The file content + dirty state continues to be shared
across threads via fileSessions, so closing a file in one thread
doesn't lose unsaved edits if it's still open in another.

## Inventory: where each piece of state lives

The unified tab store lives in `App.tsx`. Each slot has a single
owner; this list captures who reads / writes what so future tab
kinds slot in without re-discovering the layout.

### Tab membership + order
- **`threadPageTabs: Record<string, TabRef[]>`** — per-thread tab
  list, keyed by `threadId`. The single source of truth for "what
  tabs exist in this thread, in what order." Every tab kind
  (`file`, `diff`, `note`, `task`, `change-analysis`,
  `git-commit`, `tasks`, `git-history`, …) lives here. Mutated by
  `handleOpenPage`, `handleOpenFile`, `handleOpenDiff`,
  `handleNavigateInTab`, `handleStepSibling`, `closePageTab`. Read
  by the `centerTabs` builder + the `effectiveCenterActive`
  derivation.
- **`threadCenterActive: Record<string, string>`** — per-thread
  active tab id. Switching threads restores each thread's last
  active tab. Mutated by `setCenterActive` (which writes into the
  per-thread map for the current thread). The agent's `"agent"`
  literal is the default fallback when nothing else is selected.

### Per-tab navigation history
- **`threadPageHistory: Record<string, Record<string, { back: TabRef[], forward: TabRef[], siblings: NavSiblings | null }>>`**
  — keyed by `threadId` then by the tab's current id. `back` is
  pushed when `handleNavigateInTab` swaps the tab's ref; `forward`
  is populated by `handleGoBack`. `siblings` carries the list-
  originated prev/next list (cleared on back/forward; preserved on
  sibling-step). New tab kinds participate automatically — the
  history entry is created the first time the tab is navigated.

### Spec / content registries (look-aside)
- **`fileSessions: Record<string, FileSessionState>`** —
  **stream-scoped** (`streamId → session`) content + dirty-state
  cache for open files. Each `FileSessionState` carries
  `files: Record<path, { savedContent, draftContent, isLoading,
  loadError }>` plus `selectedPath` and a legacy `openOrder`. After
  the unification:
  - **Tab membership** is driven by `threadPageTabs` (each open
    file is a `kind: "file"` ref there). `openOrder` is no longer
    consulted by the renderer.
  - **Content** stays in `fileSessions` because the same buffer
    needs to survive thread switches within a stream — closing a
    file in thread A while it's open + dirty in thread B must not
    drop the draft.
  - `closePageTab` clears the file from `fileSessions[stream.id]`
    only when the file isn't open in any other tab in the same
    stream. (Today closePageTab unconditionally closes — see the
    "known follow-ups" below.)
- **`diffTabs: Array<{ id, spec: DiffSpec }>`** — a spec registry
  indexed by id. `handleOpenDiff` and `handleCompareWithClipboard`
  register specs here; the page-tab renderer's `ref.kind === "diff"`
  branch looks up the spec by id to render `DiffPage`. Specs persist
  across tab close/reopen (cheap; the array is small) so navigating
  back to a previously-closed diff via history works without
  re-registering.

### Per-tab metadata
- **`pageTitles: Record<string, string>`** — per-tab title
  registered via `usePageTitle(...)` from the page body. Drives
  both the chrome header and the tab strip label.
- **Bookmarks** — separate `bookmarksStore` (per-scope: thread /
  stream / global), keyed by `ref.id`. Cleared via the tab nav-bar
  star button.

### Renderers
- **`AgentPage`** — `apps/desktop/src/pages/AgentPage.tsx`. Wraps
  `TerminalPane` inside `Page` with `showNavBar={false}` /
  `showHeader={false}`. Currently mounted directly as `tabs[0]` in
  the `centerTabs` builder (always present, not in `threadPageTabs`).
- **Page-tab loop** (`for (const ref of pageTabsForThread)`) —
  switches on `ref.kind` to render the appropriate `*Page`
  component. Wraps each tab in `PageNavigationContext.Provider`
  with `navigate` / `goBack` / `goForward` / `siblings` / `setTitle`
  bindings keyed to the tab's id.

## Data flow: opening, navigating, closing

For every tab kind the path is:

**Open from a list / palette / rail / menu:**
```
caller
  └→ handleOpenPage(ref) | handleOpenFile(path) | handleOpenDiff(spec)
       ├→ register payload (fileSessions / diffTabs) if needed
       ├→ push ref into threadPageTabs[selectedThreadId]
       └→ setCenterActive(ref.id)
```

**Navigate in-tab (browser-tab semantic):**
```
in-page row click
  └→ useRouteDispatch / ctxNav.navigate(ref, { newTab: false, siblings? })
       └→ handleNavigateInTab(currentTabId, ref, siblings?)
            ├→ register payload if needed (file: handleOpenFile;
            │   diff: handleOpenDiffInTab — both register before
            │   calling handleNavigateInTab)
            ├→ push prior ref onto back stack
            ├→ replace tab's current ref
            └→ setCenterActive(ref.id)
```

**Sibling step (no history mutation):**
```
nav bar ↑/↓ click
  └→ ctxNav.goPrevSibling() | goNextSibling()
       └→ handleStepSibling(currentTabId, targetIndex)
            ├→ swap tab ref to siblings[targetIndex]
            └→ update siblings.index in place; back/forward untouched
```

**Close:**
```
tab × / right-click menu close
  └→ closePageTab(id)
       ├→ if id starts with "file:": close in fileSessions
       ├→ remove from threadPageTabs
       ├→ drop threadPageHistory entry
       ├→ drop pageTitles entry
       └→ snap centerActive to "agent" if it was the closed tab
```

## Persistence across restart

`threadPageTabs`, `threadPageHistory`, `diffTabs` (the spec registry)
and the per-thread active-tab pointer are owned by the
`useThreadPageTabs` hook (`apps/desktop/src/tabs/useThreadPageTabs.ts`),
which persists them to `localStorage` on change and restores them in
the `useState` initializers on boot. The read/write functions —
including the corrupt-JSON fallbacks and the legacy-blob coercions
(bare-TabRef history stacks, pre-versioning diff specs) — live in
`apps/desktop/src/tabs/pageTabsPersistence.ts` and are unit-tested in
`pageTabsPersistence.test.ts`. Storage keys:
`oxplow.layout.v1.threadPageTabs`, `oxplow.layout.v1.threadPageHistory`,
`oxplow.layout.v1.diffSpecs`.

What persists:
- The full per-thread tab list (every TabRef).
- The per-tab back/forward history + siblings record.
- Diff specs (the registry indexed by id) — except clipboard /
  selection-vs-clipboard diffs that carry inline `leftContent` /
  `rightContent`. Those are session-only.
- The active center tab id (`oxplow.layout.v1.centerActive`,
  unchanged from before).
- The per-stream open file paths (existing
  `oxplow.layout.v1.fileSessions` blob; loads file content on first
  stream activation).

Per-page state via `usePageSnapshot`:

Pages opt in via `usePageSnapshot<T>({ serialize, restore, deps })`
(see `apps/desktop/src/tabs/usePageSnapshot.ts`). On mount the hook
reads any saved blob keyed by the page's `pageKey` from
`PageNavigationContext` (`${threadId}::${tabId}`) and calls
`restore`. On each `deps` change it serializes and writes.
`closePageTab` clears the snapshot row so closed tabs don't leak.

Adopted pages (Phase 3):
- `ChangeAnalysisDrilldown` — view toggle (Semantic / File list) +
  status filter (All / Added / Modified / Deleted).
- `WikiPageTab` — body scroll position. Reapplied when the body
  re-renders so brief layout shifts during markdown load don't
  reset the scroll.
- `EditorPane` — Monaco view-state (cursor, scroll, folds,
  selection) via `editor.saveViewState()` / `restoreViewState()`.
  If a snapshot arrives before the editor mounts, the hook stashes
  it in a ref and the post-mount block applies it.

Other pages mount fresh after restart. The `display:none`
mounted-stack approach used for in-session back/forward (perfect
fidelity, free) cannot survive a restart — no DOM, no React state.
Snapshots are the only path for cross-restart fidelity.

## Known follow-ups + invariants

- **Agent ref doesn't live in threadPageTabs yet.** It's the only
  special case in `centerTabs`. Lifting it would need a
  closable=false flag on the unified-tab record (or a
  per-PageKind table).
- **fileSessions close on closePageTab is not refcounted.** If the
  same file is open in two threads of the same stream and one
  thread closes its tab, the buffer is dropped for both. Should be
  a refcount or a stream-scoped check that another thread still
  holds the path.
- **Diff specs are not GC'd.** `diffTabs` grows monotonically per
  session. Cleanup hook on closePageTab could prune.
- **Stream switch consequences.** When the user changes streams,
  the rendered thread tabs come from the new stream's selected
  thread. `fileSessions` is per-stream, so file content is correct;
  `threadPageTabs` is per-thread, so the *list* is correct. Diff
  specs are per-session (single global registry) — works because
  diff ids embed the spec details, but theoretically a bug if two
  streams produced colliding ids; today this doesn't happen because
  diff ids include the leftRef which is stream-specific in practice.

## Adding a new tab kind: checklist

1. Extend `PageKind` in `tabs/tabState.ts`.
2. Add a `pageRefs.ts` helper (`fooRef(...)`) and document the id
   format in the table above.
3. Build a `*Page` component in `apps/desktop/src/pages/` that
   wraps the body in `<Page>` and (typically) registers a title
   via `usePageTitle`.
4. Add the kind to **`handleOpenPage`'s `switch (ref.kind)` allowlist**
   in `App.tsx` (the per-thread-page-tab `case` group). This is easy to
   miss: it has a `default: return`, so a kind that isn't listed is
   silently dropped — the rail/search entry appears but clicking does
   nothing. An index kind also needs to be in `INDEX_KINDS`
   (`pageKinds.tsx`) and the `indexRef(...)` param union (`pageRefs.ts`).
5. Add a `ref.kind === "..."` branch in the `centerTabs` page-tab
   loop in `App.tsx` to render the page.
6. Plumb any list rows that target the new kind through
   `useRouteDispatch` / `RouteLink` so plain-click navigates in-tab.
7. If the page exposes data that other pages should backlink to,
   register a provider in `appPageBacklinks.ts`.

Do **not** introduce a parallel state slot for the new kind's tab
list (no new `fooTabs` array). Look-aside registries are fine if
the kind needs runtime data the ref payload can't carry (like
diff specs), but tab membership goes in `threadPageTabs`.

## When to update this doc

- Add a new page kind: extend `PageKind`, add a `pageRefs.ts` helper,
  document the id format here.
- Add a new rail HUD section: document the data source and trigger
  conditions.
- Replace a legacy panel with a Page-wrapped renderer: tick the
  migration status row above and link the new page module.
