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

## Page kinds

`PageKind` (`tabState.ts`):

```
"agent" | "file" | "diff" | "note" | "work-item" | "finding"
| "all-work" | "notes-index" | "files" | "code-quality"
| "local-history" | "git-history" | "subsystem-docs"
| "settings" | "start" | "dashboard"
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
- 🚧 Phase 4 — New pages + backlinks indexer.
- 🚧 Phase 5 — Web-style interactions sweep (kill modals + right-click
  menus): inline confirm + Undo toast, kebab popovers, slideovers,
  page-form replacements.
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
