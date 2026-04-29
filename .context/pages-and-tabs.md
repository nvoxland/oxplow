# Pages and tabs

What this doc covers: the per-thread tab store, the shared `Page` chrome,
the page-ref id format, and the rail HUD that drives navigation. This is
the substrate the IA redesign is being built on; it lives alongside the
existing IDE-style chrome until later phases migrate the panels into pages.

## Mental model

- **Streams** = parallel worktrees (top-level tabs). Unchanged.
- **Threads** = independent lines of thought within a stream (second-row
  tabs). Unchanged.
- **Each thread owns its own set of open tabs and an active tab.**
  Switching threads restores its tab set; switching streams swaps to the
  selected thread of the new stream. The agent terminal is always
  available per thread and survives switches.
- A **page** is anything addressable inside a tab body — file, work item,
  wiki note, finding, dashboard, settings, agent terminal. Pages share a
  common chrome (header + collapsible Backlinks panel).

## Modules

| File | Purpose |
|---|---|
| `src/ui/tabs/tabState.ts` | `createTabStore()` — per-thread tab list + active id, with `openTab`, `ensureTab`, `activate`, `closeTab`, `subscribe`. In memory; no cross-restart persistence in v1. |
| `src/ui/tabs/useTabStore.ts` | `getTabStore()` singleton + `useThreadTabs(threadId)` hook backed by `useSyncExternalStore`. |
| `src/ui/tabs/pageRefs.ts` | Stable id helpers: `agentRef()`, `fileRef(path)`, `diffRef({...})`, `noteRef(slug)`, `workItemRef(id)`, `findingRef(id)`, `indexRef(kind)`, `dashboardRef(variant)`. Centralizing the format keeps cross-component links and ⌘K open-by-id stable. |
| `src/ui/tabs/Page.tsx` | Shared page chrome: title + kind chip + status chips + actions slot, optional **browser-style nav bar** (back/forward + bookmark + backlinks dropdown — auto-mounted from `PageNavigationContext` when present), body, collapsible legacy Backlinks region. Title can be passed as a `title` prop or registered programmatically by the page via `usePageTitle`; the chrome falls back to the context title when `title` is omitted. `showNavBar` / `showHeader` flags (default true) let a page opt out — agent-style bare content sets both false. Reads only semantic CSS variables (skin via theme). |
| `src/ui/tabs/PageNavBar.tsx` | Dumb nav-bar component: back/forward buttons, optional bookmark toggle, optional backlinks dropdown (popover). Mounted by `Page` when context or explicit `navBar` prop is present. |
| `src/ui/tabs/PageNavigationContext.ts` | React context exposing `{ navigate(ref, { newTab? }), goBack, goForward, canGoBack, canGoForward, setTitle, title }` to descendants of an active page tab. Wrapped around every non-agent center tab in `App.tsx`. `BacklinksList` reads it so default-click navigates in-tab. The `usePageTitle(title)` helper registers the page's current title with the host so the same string drives the chrome header AND the tab strip label — no per-page duplicate header markup. |
| `src/ui/pages/FilePage.tsx` | Thin Page wrapper around `EditorPane`. Calls `usePageTitle(basename + ● dirty)` so the file's name flows into the shared chrome title. EditorPane keeps owning Monaco / blame / context menus; the wrapper only provides chrome above. |
| `src/ui/pages/DiffPage.tsx` | Thin Page wrapper around `DiffPane` for diff tabs. Calls `usePageTitle(basename + (label))`. |
| `src/ui/tabs/RouteLink.tsx` | Browser-style link button: left-click → in-tab navigate (or new tab when `pinnedSlot`), Cmd/Ctrl-click + middle-click + right-click → new tab. Falls back to caller-supplied `onNavigate` when used outside a `PageNavigationContext` (rail HUD, palette). |
| `src/ui/components/RailHud/RailHud.tsx` | Persistent left rail HUD: search trigger, active item, up next, **bookmarks** (when present), recent files, pages directory. Passive — never auto-opens tabs. Bookmark rows show a single-letter scope badge (T/S/G) and a per-row remove button. |
| `src/ui/tabs/bookmarks.ts` + `useBookmarks.ts` | Per-scope (thread / stream / global) bookmark store backed by localStorage. Pages bookmark via the `PageNavigationContext.bookmark` binding; the rail HUD reads the merged set. |
| `src/ui/tabs/appPageBacklinks.ts` | App-page backlinks providers — pure `(payload, ctx) → BacklinkEntry[]` functions for `git-dashboard`, `git-history`, `uncommitted-changes`, `git-commit`. `useBacklinks` dispatches to them when `target.kind` matches. Add a new app-page provider by registering it in `APP_PAGE_BACKLINKS` and extending `useBacklinks` to fetch any new data slice it needs. |
| `src/ui/pages/GitCommitPage.tsx` | Single-commit page (`git-commit:<sha>`). Reuses `CommitDetailBody` (now exported from `CommitDetailSlideover`). Routed via `gitCommitRef(sha)`. Bookmark-/history-friendly alternative to the slideover. |
| `src/ui/components/RailHud/sections.ts` | Pure helpers: `computeActiveItem`, `computeUpNext`, `sortRecentFiles`, `computePagesDirectory`. The pages directory is a pure function so it can be unit-tested without mounting the React rail. |
| `src/ui/pages/GitDashboardPage.tsx` | Committed-history rollup: branch header (current branch + upstream + ahead/behind + push), small uncommitted mini-card that links to `UncommittedChangesPage`, recent commits rendered through the shared `CommitGraphTable` (last 5, current branch only via `getGitLog({ all: false })`; click a row → reveal in `GitHistoryPage`), worktrees row with per-row "Merge into current", recent remote branches with per-row pull/push. All ref-mutating actions confirm the exact `git` command before running. Routed via `gitDashboardRef()`. |
| `src/ui/components/History/CommitGraphTable.tsx` | Pure presentation of the git-log graph (branch/merge dots + lines + sha + ref badges + subject + author + relative date). Used by both `HistoryPanel` (full list with detail pane) and `GitDashboardPage`'s recent-commits card. `indexRefsBySha(log)` exported alongside groups branch heads + tags by sha so callers feed identical maps. |
| `src/ui/pages/UncommittedChangesPage.tsx` | Stats-focused view of working-tree changes: M/A/D/R/U + total +/-, collapsible folder tree with per-folder rollup of files / +/-, Commit-all action. Distinct from `FilesPage` which is the full project file tree. Routed via `uncommittedChangesRef()`. |
| `src/ui/tabs/backlinksIndex.ts` | Pure cross-kind backlinks indexer. `computeBacklinks(target, ctx)` returns `BacklinkEntry[]` linking notes ↔ files ↔ work items ↔ findings. Inputs are plain data slices (notes, work items with `touched_files`, findings) — no IPC, no side effects, fully unit-tested. |
| `src/ui/tabs/useBacklinks.ts` | React hook that materializes a `BacklinkContext` (notes bodies + findings + work-item touched-files) from live IPC and pipes into `computeBacklinks`. Used by `WorkItemPage`, `NotePage`, `FindingPage`. |
| `src/ui/tabs/BacklinksList.tsx` | Default renderer for the Page chrome's `backlinks` slot — buttons that route via `onOpenPage`. |
| `src/ui/pages/WorkItemPage.tsx` | Single-record page for a work item — wraps `WorkItemDetail` + `ActivityTimeline`. Backlinks computed via `useBacklinks`. |
| `src/ui/pages/NotePage.tsx` | Single-record page for a wiki note — wraps `NoteTab`. The `note:<slug>` center-tab is rendered through this Page wrapper so notes get the unified chrome (title from `usePageTitle`, browser-style back/forward + star, Backlinks panel). `NoteTab` no longer renders its own header — freshness badge + Edit/Save/Revert/Delete/Create live in a thin secondary toolbar inside the body. In-tab wikilink-to-note clicks route through `PageNavigationContext.navigate(noteRef)` so they participate in tab-level history. |
| `src/ui/pages/FindingPage.tsx` | Single-record page for a code-quality finding — kind/path/line range/metric + source snippet + "Jump to source". |
| `src/ui/pages/DashboardPage.tsx` | Composite Planning / Review / Quality dashboards. Variant chosen via `dashboardRef("planning"\|"review"\|"quality")`. |
| `src/ui/pages/StreamSettingsPage.tsx` | Per-stream settings page (custom prompt). Replaces the in-rail StreamRail settings modal. Routed via `streamSettingsRef(streamId)`. |
| `src/ui/pages/ThreadSettingsPage.tsx` | Per-thread settings page (custom prompt). Replaces the in-rail ThreadRail settings modal. Routed via `threadSettingsRef(threadId)`. |
| `src/ui/components/Slideover.tsx` | Right-edge panel primitive (~38vw, backdrop-click + Escape close, focus-into-panel on open) for form-shaped flows that don't justify a full page. Use instead of a centered modal. |

## Page kinds

`PageKind` (`tabState.ts`):

```
"agent" | "file" | "diff" | "note" | "work-item" | "finding"
| "tasks" | "done-work" | "backlog" | "archived"
| "notes-index" | "files" | "code-quality"
| "local-history" | "git-history" | "git-dashboard" | "git-commit"
| "uncommitted-changes" | "hook-events" | "subsystem-docs"
| "settings" | "start" | "dashboard"
| "new-stream" | "new-work-item"
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
| file | `file:<path>` | `file:src/electron/runtime.ts` |
| diff | `diff:<path>\|<from>\|<to>\|<labelOverride>` | `diff:src/a.ts\|abc\|def\|` |
| note | `note:<slug>` | `note:how-stop-hook-fires` |
| work-item | `wi:<id>` | `wi:wi-142` |
| finding | `finding:<id>` | `finding:f-7` |
| `*-index` | the kind name | `code-quality`, `start`, `settings` |
| git-dashboard | `git-dashboard` | `git-dashboard` |
| git-commit | `git-commit:<sha>` | `git-commit:abc1234567890` |
| uncommitted-changes | `uncommitted-changes` | `uncommitted-changes` |
| dashboard | `dashboard:<variant>` | `dashboard:planning` |
| new-stream | `new-stream` | `new-stream` |
| new-work-item | `new-work-item` | `new-work-item` |
| stream-settings | `stream-settings:<streamId>` | `stream-settings:s-7` |
| thread-settings | `thread-settings:<threadId>` | `thread-settings:t-3` |
| op-error | `op-error:<errorId>` | `op-error:oe-abc123` |
| external-url | `external-url:<url>` | `external-url:https://example.com/path` |

## Rail HUD contract

The rail is **read-only with respect to tabs** — it never auto-opens a tab.
Every rail click goes through a single `onOpenPage(ref: TabRef)` callback
that the host wires to its own routing. Sections appear only when they
have content:

1. **Search trigger** — opens the ⌘K palette. Always visible.
2. **Active item** — lowest-`sort_index` non-epic item in `in_progress`
   for the current thread. Shows live `AgentStatusDot` + status label.
3. **Up next** — top 5 `ready` non-epic items.
4. **Recent files** — top 6 file paths recently opened/touched in this
   thread (today derived from `currentSession.openOrder`; eventually
   should include agent-touched files).
5. **Pages** — directory entries (computed in `computePagesDirectory`,
   exposed for unit testing): Start, Plan work, Done work, Backlog,
   Archived, Notes, Files, Code quality, Local history, Git dashboard,
   Uncommitted, Git history, Hook events, Subsystem docs, Settings,
   plus Dashboards (Planning, Review, Quality). The backlog ready
   count surfaces as a badge on the **Backlog** entry.

## Migration status

The full IA redesign ships in phases (see plan
`/Users/nvoxland/.claude/plans/the-ui-is-very-delightful-badger.md`):

- ✅ Phase 0 — Theme foundation (`.context/theming.md`).
- ✅ Phase 1 — Tab store + page chrome + page refs (this doc).
- ✅ Phase 2 — Rail HUD shell (this doc).
- ✅ Phase 3 — Page migration: every rail HUD "Pages" entry now opens
  a Page-wrapped renderer in `src/ui/pages/`:
  Start, Settings, Code quality, Local history, Git history, Files,
  Notes, All work, Subsystem docs. Both docks have since been removed
  — the rail HUD is THE left chrome and pages are THE center surface
  (see "Left dock removed" / "Bottom dock removed" notes below).
- ✅ Phase 4 — New pages + backlinks indexer:
  `WorkItemPage`, `NotePage`, `FindingPage`, three `DashboardPage`
  variants (Planning / Review / Quality), and the
  `computeBacklinks(target, ctx)` indexer. `FilePage` and `DiffPage`
  were intentionally skipped: file and diff tabs already render via
  `centerTabs` with their own chrome (Monaco editor, diff editor) and
  wrapping them in Page chrome would double-up the header. The
  legacy `note:` tab path now renders through `NotePage` so wiki
  notes get a Backlinks panel; modal-based work-item edits still work
  alongside `WorkItemPage` for callers that want the modal flow.
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
  `Slideover` primitive (`src/ui/components/Slideover.tsx`) plus the
  BranchPicker rename Slideover, ProjectPanel commit-dialog
  Slideover, and the cross-page detail wrappers
  `SnapshotDetailSlideover` (`src/ui/components/Snapshots/SnapshotDetailSlideover.tsx`)
  and `CommitDetailSlideover` (`src/ui/components/History/CommitDetailSlideover.tsx`).
  5e landed the per-stream and per-thread settings as
  `StreamSettingsPage` and `ThreadSettingsPage`, the inline-new-row
  that retired `CreateThreadModal`, and the new-stream / new-work-
  item page-form replacements (`NewStreamPage`, `NewWorkItemPage` —
  routed via `newStreamRef()` / `newWorkItemRef({...})`). The
  `PlanPane` `NewWorkItemModal` only backs the edit-double-click
  flow now; new flows route through pages.
- ✅ Phase 6 — Selection action bar + drag-to-add-context polish.
  `SelectionActionBar` (`src/ui/components/Plan/SelectionActionBar.tsx`)
  appears at the top of `PlanPane`'s work-group region whenever ≥1
  rows are marked. It owns no state; PlanPane reads its existing
  marked-set and routes Change status / Change priority / Add to
  agent context / Delete through the same paths used by single-row
  kebabs. The agent terminal now accepts multi-row work-item drags
  (decodes the `WORK_ITEM_DRAG_MIME` payload's `items` slice
  directly — see `.context/usability.md` "Add to agent context").
  Drag-to-add sources expanded: BacklinksList entries, RailHud
  recent-files / active item / up-next, CodeQualityPanel file group
  rows, plus a "Add to agent context" item on every work-item kebab
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
tabs.

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
commands that used to flip the dock (`commitFiles`, edit-work-item)
now route through `handleOpenPage(indexRef("files"))` /
`handleOpenPage(indexRef("tasks"))`. E2e probes that previously
relied on `dock-tab-plan` / `dock-tab-project` / `dock-panel-*`
testids now click `rail-page-tasks` / `rail-page-files` and
assert on `page-tasks` / `page-files`. The harness startup gate
(`waitForOxplowReady`) polls for `rail-hud`.

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
  pool with grooming affordances: free-text `category` bucket
  (default group-by), comma-separated `tags` (filter chips), and
  promote-into-thread action. Items are `work_items` rows with
  `thread_id IS NULL`; promote/demote flips `thread_id` without
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
`src/ui/components/Card.tsx` for cross-page "View X →" affordances;
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

## Per-thread active tab (today)

`App.tsx` holds a `Record<threadId, string> threadCenterActive` map and
derives `centerActive` from it. `setCenterActive` writes to the map for
the currently selected thread. Switching threads automatically restores
each thread's last active tab.

Note tabs are now per-thread (they live in `threadPageTabs` like every
other page kind, so opening or closing a note in one thread doesn't
leak into another). File and diff tab lists are still stream-scoped
(`fileSessions[stream.id]`, `diffTabs`) — file content/dirty
intentionally crosses threads within a stream; per-thread file tabs
remain a future refactor.

## When to update this doc

- Add a new page kind: extend `PageKind`, add a `pageRefs.ts` helper,
  document the id format here.
- Add a new rail HUD section: document the data source and trigger
  conditions.
- Replace a legacy panel with a Page-wrapped renderer: tick the
  migration status row above and link the new page module.
