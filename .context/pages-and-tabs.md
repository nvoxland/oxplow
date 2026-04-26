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
| `src/ui/tabs/Page.tsx` | Shared page chrome: title + kind chip + status chips + actions slot, body, collapsible Backlinks region. Reads only semantic CSS variables (skin via theme). |
| `src/ui/components/RailHud/RailHud.tsx` | Persistent left rail HUD: search trigger, active item, up next, recent files, pages directory. Passive — never auto-opens tabs. |
| `src/ui/components/RailHud/sections.ts` | Pure helpers: `computeActiveItem`, `computeUpNext`, `sortRecentFiles`. |
| `src/ui/tabs/backlinksIndex.ts` | Pure cross-kind backlinks indexer. `computeBacklinks(target, ctx)` returns `BacklinkEntry[]` linking notes ↔ files ↔ work items ↔ findings. Inputs are plain data slices (notes, work items with `touched_files`, findings) — no IPC, no side effects, fully unit-tested. |
| `src/ui/tabs/useBacklinks.ts` | React hook that materializes a `BacklinkContext` (notes bodies + findings + work-item touched-files) from live IPC and pipes into `computeBacklinks`. Used by `WorkItemPage`, `NotePage`, `FindingPage`. |
| `src/ui/tabs/BacklinksList.tsx` | Default renderer for the Page chrome's `backlinks` slot — buttons that route via `onOpenPage`. |
| `src/ui/pages/WorkItemPage.tsx` | Single-record page for a work item — wraps `WorkItemDetail` + `ActivityTimeline`. Backlinks computed via `useBacklinks`. |
| `src/ui/pages/NotePage.tsx` | Single-record page for a wiki note — wraps `NoteTab`. The `note:<slug>` center-tab is rendered through this Page wrapper so notes get a Backlinks panel. |
| `src/ui/pages/FindingPage.tsx` | Single-record page for a code-quality finding — kind/path/line range/metric + source snippet + "Jump to source". |
| `src/ui/pages/DashboardPage.tsx` | Composite Planning / Review / Quality dashboards. Variant chosen via `dashboardRef("planning"\|"review"\|"quality")`. |
| `src/ui/pages/StreamSettingsPage.tsx` | Per-stream settings page (custom prompt). Replaces the in-rail StreamRail settings modal. Routed via `streamSettingsRef(streamId)`. |
| `src/ui/pages/ThreadSettingsPage.tsx` | Per-thread settings page (custom prompt). Replaces the in-rail ThreadRail settings modal. Routed via `threadSettingsRef(threadId)`. |
| `src/ui/components/Slideover.tsx` | Right-edge panel primitive (~38vw, backdrop-click + Escape close, focus-into-panel on open) for form-shaped flows that don't justify a full page. Use instead of a centered modal. |

## Page kinds

`PageKind` (`tabState.ts`):

```
"agent" | "file" | "diff" | "note" | "work-item" | "finding"
| "all-work" | "notes-index" | "files" | "code-quality"
| "local-history" | "git-history" | "subsystem-docs"
| "settings" | "start" | "dashboard"
| "new-stream" | "new-work-item"
| "stream-settings" | "thread-settings"
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
| dashboard | `dashboard:<variant>` | `dashboard:planning` |
| new-stream | `new-stream` | `new-stream` |
| new-work-item | `new-work-item` | `new-work-item` |
| stream-settings | `stream-settings:<streamId>` | `stream-settings:s-7` |
| thread-settings | `thread-settings:<threadId>` | `thread-settings:t-3` |

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
5. **Pages** — directory entries: Start, All work, Notes, Files, Code
   quality, Local history, Git history, Subsystem docs, Settings, plus
   Dashboards (Planning, Review, Quality).

## Migration status

The full IA redesign ships in phases (see plan
`/Users/nvoxland/.claude/plans/the-ui-is-very-delightful-badger.md`):

- ✅ Phase 0 — Theme foundation (`.context/theming.md`).
- ✅ Phase 1 — Tab store + page chrome + page refs (this doc).
- ✅ Phase 2 — Rail HUD shell (this doc).
- ✅ Phase 3 — Page migration: every rail HUD "Pages" entry now opens
  a Page-wrapped renderer in `src/ui/pages/`:
  Start, Settings, Code quality, Local history, Git history, Files,
  Notes, All work, Subsystem docs. Legacy left/bottom dock entries
  (Plan, Project, Notes, Hook events, Git history, Local history,
  Code quality) are still mounted alongside the pages so the app
  stays usable mid-migration; phase 5/7 cleans them up.
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
- 🟡 Phase 5 — Web-style interactions sweep (kill modals + right-click
  menus). 5a (`InlineConfirm` + `UndoToast` queue) and 5b (`InlineEdit`
  + `InlinePromptStrip` for new-X flows) shipped: `ConfirmDialog.tsx`
  and `PromptDialog.tsx` are deleted. 5c (Kebab popovers) shipped on
  the high-traffic surfaces (StreamRail, ThreadRail, CenterTabs,
  WorkGroupList rows, Notes pane rows, FileTree rows); BranchPicker
  manage menu, EditorPane git-blame margin, NotesPane/MarkdownView
  link menus, WikiActivityBar entry menus, and TerminalPane right-
  click are tracked as a continuation task. 5d landed `Slideover`
  primitive (`src/ui/components/Slideover.tsx`); BranchPicker /
  commit-dialog / snapshot-detail / commit-detail migrations to
  Slideover are still pending. 5e landed the per-stream and
  per-thread settings as `StreamSettingsPage` and `ThreadSettingsPage`
  (the in-rail settings overlay falls back to the legacy modal when
  the rail isn't given an `onOpenStreamSettings` /
  `onOpenThreadSettings` handler — App.tsx wires both). New-stream /
  New-work-item page-form replacements and the inline-new-row that
  retires `CreateThreadModal` are still pending.
- 🚧 Phase 6 — Selection action bar + drag-to-add-context polish.
- 🚧 Phase 7 — Density + visual polish.

Phase 3 is shipped: rail HUD "Pages" entries open as full center-area
tabs. The existing left rail toolwindows (Work, Files, Notes, plus the
HUD tab) and bottom drawer (Hook events, Git history, Local history,
Code quality) remain in place during phases 4–6 so existing
keyboard/menu paths keep working; phase 5/7 trims them.

## Per-thread active tab (today)

`App.tsx` holds a `Record<threadId, string> threadCenterActive` map and
derives `centerActive` from it. `setCenterActive` writes to the map for
the currently selected thread. Switching threads automatically restores
each thread's last active tab.

The full tab-list is still stream-scoped (`fileSessions[stream.id]`,
`noteTabs`, `diffTabs`). Per-thread tab-list scoping is a follow-on; the
active-tab pointer is the bigger UX win and lands first.

## When to update this doc

- Add a new page kind: extend `PageKind`, add a `pageRefs.ts` helper,
  document the id format here.
- Add a new rail HUD section: document the data source and trigger
  conditions.
- Replace a legacy panel with a Page-wrapped renderer: tick the
  migration status row above and link the new page module.
