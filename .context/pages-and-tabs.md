# Pages and tabs


What this doc covers: the per-thread tab store, the shared `Page` chrome,
the page-ref id format, and the rail HUD that drives navigation. This is
the substrate the (completed) IA redesign was built on; the rail HUD +
pages are THE shell.

## Mental model

- **Streams** = parallel worktrees; **threads** = independent lines of
  thought within a stream. Both are rows in the far-left **Navigator**
  (`components/Navigator.tsx`), not tab rows: a stream glyph with its
  thread glyphs under it. Clicking a glyph switches stream / selects
  thread; the expanded panel's right-click menus add a thread (stream
  menu → Add thread), promote one (thread menu → Make writer), rename,
  open settings, close or remove; `+ Add stream` at the panel's foot
  opens the `new-stream` page. The launcher's New Thread… / New Stream…
  reach the same flows. See the Navigator row in "Modules".
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
| `apps/desktop/src/tabs/pageRefs.ts` | Every tab id, built here and nowhere else: entity refs (`fileRef`, `directoryRef`, `wikiPageRef`, `taskRef`, `gitCommitRef`, `metricRef`, `lensRef`) and shell routes (`agentRef`, `indexRef(kind)`, `diffRef(spec)`, `snapshotRef`/`effortDiffRef`/`endpointDiffRef`, `dashboardRef`, …). `refFromTabId(id)` is the inverse of every constructor (via the exhaustive `ROUTES` table for `page:` ids); `pageKindOf(id)` names the kind a tab renders as; `diskFilePath(id)` is the one way to ask "which working-tree file is this tab". |
| `apps/desktop/src/tabs/Page.tsx` | Shared page chrome: title + kind chip + status chips + actions slot, optional **browser-style nav bar** (back/forward + bookmark + backlinks dropdown — auto-mounted from `PageNavigationContext` when present), body, collapsible legacy Backlinks region. Title can be passed as a `title` prop or registered programmatically by the page via `usePageTitle`; the chrome falls back to the context title when `title` is omitted. `showNavBar` / `showHeader` flags (default true) let a page opt out — agent-style bare content sets both false. **`titleInBody`** (default false) says the page renders its OWN title as an `<h1>` in the body (use the exported `pageH1Style` for consistency), so the chrome (nav bar / header) suppresses the title to avoid a duplicate — the tab-strip label via `usePageTitle` is unaffected. Adopters: `MetricDetailPage`, `DiffViewPage` (tsk137). **Body layout** is chosen via `layout?: "full" \| "details"` (default `"full"`); details layout pairs a full-width center column with a 320px sticky right rail (`rightRail` prop). Below ~960px body width the rail doesn't disappear — `DetailsBody` relocates the **same panel** (card surface + tinted header band + ⋯ actions) inline to the top of the center column. Purely responsive, no manual toggle and no per-page logic — consumers just pass `rightRail`. Reads only semantic CSS variables (skin via theme). |
| `apps/desktop/src/tabs/PageNavBar.tsx` | Dumb nav-bar component: back/forward buttons, optional bookmark toggle, optional backlinks/outbound/snapshots dropdowns (popovers), and an optional **comment navigator** slot (`comments` ReactNode, rendered before Backlinks). Mounted by `Page` when context or explicit `navBar` prop is present. |
| `apps/desktop/src/components/Comments/CommentNavigator.tsx` | Self-contained per-page comment navigator for the nav bar. `useCommentsForTarget(kind,id)` → shows "Comments (N)", steps through anchored comments with ◀ ▶ (each `requestCommentReveal`s to scroll + open inline on the surface), and a dropdown lists all comments plus an **Orphaned (M)** section. Orphaned entries are clickable too: they `requestCommentReveal`, and the surfaces open the thread popover at a fallback position (no anchor to scroll to) so the comment is readable. The popover then offers a **"Relink to selection"** button (`CommentPopover.onRelink`, wired by RichTextField + MonacoCommentLayer) that re-attaches the comment to the editor's current selection — select the intended text first. (The right-click "Relink orphaned" path still exists.) Comment-bearing pages pass it via `Page`'s `commentsNav` prop (FilePage → file/path, WikiPage → wiki/slug, WorkItemPage → work_item/`<provider>:<id>`). Renders nothing when the page has no comments. Pure helpers (`partitionPageComments`, `stepComment`) live in `pageCommentNav.ts` (unit-tested) and are shared with the per-thread stepper: `CommentPopover` takes an `onStep(dir)` prop so an open comment thread shows ◀ Prev / Next ▶ buttons that scroll to + reopen the adjacent comment (wired by RichTextField + MonacoCommentLayer via `stepComment` + `requestCommentReveal`). |
| `apps/desktop/src/tabs/PageNavigationContext.ts` | React context exposing `{ navigate(ref, { newTab? }), goBack, goForward, canGoBack, canGoForward, setTitle, title }` to descendants of an active page tab. Wrapped around every non-agent center tab in `App.tsx`. `BacklinksList` reads it so default-click navigates in-tab. The `usePageTitle(title)` helper registers the page's current title with the host so the same string drives the chrome header AND the tab strip label — no per-page duplicate header markup. |
| `apps/desktop/src/pages/FilePage.tsx` | Thin Page wrapper around `EditorPane`. Calls `usePageTitle(basename + ● dirty)` so the file's name flows into the shared chrome title, and `useBacklinks(fileRef(path))` so wiki pages, tasks, commits, and findings that reference the file appear in the nav-bar Backlinks dropdown. EditorPane keeps owning Monaco / blame / context menus; the wrapper only provides chrome above. |
| `apps/desktop/src/pages/DiffPage.tsx` | Thin Page wrapper around `DiffPane` for diff tabs. Calls `usePageTitle(basename + (label))`. |
| `apps/desktop/src/pages/DiffViewPage.tsx` | The **explicit start→end diff view** (`diff-view` kind). Three modes via `DiffViewSpec`: `snapshot` (legacy `snapshotRef(N)` prev→N drill-in); `effort` (`effortDiffRef(effortId)` — resolves the effort's own start/end snapshot bracket via the `get_effort` IPC on load, carrying the `taskId`/`effortId`, with an "Effort is in progress" notice when the end is null); `turn` (`turnRef(turnId)` — the turn's start snapshot → end snapshot via `get_agent_turn`, start → working tree with a "Turn is still running" notice while it runs; a turn with no start snapshot shows an explanation instead of diffing the empty tree, `resolveTurnEndpoints`, tsk467 — the notice wording is `inProgressNotice(subject)`); `endpoints` (`endpointDiffRef(start, end)` — an explicit pair of revisions, `working` / `snap:<id>` / `git:<rev>`). All modes list the changed files through the `diff` RPC (`Trees`, `.context/vcs.md`) via `useChangedFiles({ kind: "endpoints" })` (`components/ChangedFiles/`), which also gives the two sides' `Revision`s for opening a file's diff. **Page shape (tsk343–346/370):** the page uses the `Page` **`layout="details"`** (center column + right rail). The `<h1>` title is `Changes: <effort title>` *when the diff is for an effort* (effort passed, OR the start/end snapshots line up exactly with an overlapping effort's bracket), else a comparison label — **`Commit comparison`** when both endpoints are git versions (commit / commit-pinned snapshot), else **`Snapshot comparison`**. The date/commit **range** is passed as the `rightRail`. When the diff **lines up with an effort** (effort passed, or a snapshot range matching an overlapping effort's bracket — `primaryTaskId`/`effortTitle`), the rail **leads with a "Task" row** (`diff-view-task-link`) naming the effort's task and linking to its `taskRef` (in-tab via `onOpenPage`); it self-hides for a non-effort range. Below it: a **date header** (`rangeDateLabel` in `diffViewModel.ts` — a single date when both endpoints fall on the same calendar day, a "start – end" range when they span multiple days, the single known date when only one endpoint is time-based, hidden when neither is) above two **selectable** fields — a "Start"/"End" caption beside an `EndpointPicker` dropdown (`apps/desktop/src/components/Diff/EndpointPicker.tsx`). The closed trigger shows a **time-only** label (so it fits on one line next to the caption); opening it lists the **20 newest snapshots on the same branch as the diffed endpoints** (`pickerBranch` / `snapshotsOnBranch` in `diffViewModel.ts` — reference branch is end snapshot's `git_branch` → start's → `stream.branch`; only snapshots *known* to be on a different branch are dropped, while unrecorded-branch snapshots (pre-V42 rows / detached HEAD) are kept so the picker stays populated on existing data — a branch switch within the stream's worktree never mixes other branches' snapshots in), **constrained per side so the range stays valid** (`rangeEndpointOptions` — the Start list only offers snapshots *before* the current End, the End list only snapshots *after* the current Start, so you can't pick an inverted range; no constraint when the opposite endpoint isn't a snapshot), plus whichever snapshot the current endpoint sits on (so the selection is always present), with the **full date+time** and, for snapshots that pinned a commit, the short commit sha. Picking a snapshot rescopes the diff in place — `onOpenPage(endpointDiffRef(newStart, end))` / `endpointDiffRef(start, newEnd)`, which `navOpen` routes as an **in-tab navigation** (Back returns to the prior range) — so the user can expand/collapse the diff range. The picker's popover is `position:fixed` (escapes the rail panel's overflow), closes on pick / Escape / outside-click, and is disabled when there are no candidate snapshots. The rail shows in the details rail (or, when the rail collapses on narrow widths, as the same panel stacked at the top of the center column — handled by the `Page` base, see below). Then an optional **Concurrent Efforts** `<ul>` (every effort overlapping the range — `listEffortsOverlappingRange`, **scoped to the diffed snapshot's own stream** so other streams'/branches' efforts whose global snapshot-id windows merely overlap don't leak in — other than the one the diff is for, each linked to its `taskRef`), a **Files Changed** `<h2>` section rendering the shared collapsible `ChangedFilesTree` (A/M/D + zone badges + Expand/Collapse all) — *all* changed files, or only the effort's claimed files (`listEffortFiles`) when the diff was opened **for** an effort. The section says so when there are no test changes. Sections are **boxless** (h2 + content, no card); `ChangedFilesTree` takes `showFileCount={false}` here. **Effort header.** Opened *for* an effort, the title is `EffortHeader` (`pages/EffortHeader.tsx`): the effort's `v_effort.title` (its own, else its item's, else its first prompt's), edited in place with `InlineEdit` (`oxplow.effort.update`; clearing restores the default); a "Linked to" line — the item (opens its page) with Unlink, or "Not linked" — and "Link to task…", an `InlinePromptStrip` taking `tsk42` or a `work_item:` ref (`oxplow.effort.link`); while open, Close effort (`oxplow.effort.close`), else how it closed (`closed_by`). It re-reads on `v_effort` changing; the tab title follows its title. An unlinked effort in Concurrent Efforts is named by its `v_effort.title`, else "Unlinked work". **Section order:** title → (effort **description**) → the **`effort.review.details` slot** → Concurrent Efforts → Files Changed. When the diff is **for an effort** (passed or lined-up): the effort's task **description** renders just under the title (`MarkdownView` via `getTask`), and the `effort.review.details` slot (`LensSlots`, params `effort_id` + the effort's `change_id`) mounts extension lenses — oxplow-bundled's decisions/claims and oxplow-bundled's change analysis (`change-review`: summary, look-here-first, churn treemap, function and test changes, co-change, duplication, cross-zone imports), coverage and tests, metric deltas and agent nudges. Both self-hide for a non-effort range. Chrome/tab title is driven by `usePageTitle` (plain-text mirror of the h1). Pure endpoint-resolution logic (`resolveEffortEndpoints`, `resolveTurnEndpoints`, `inProgressNotice`, `resolveSnapshotEndpoints`, `previousSnapshotId`, `snapshotRange`) lives React-free in `apps/desktop/src/diffViewModel.ts`. `Trees` (`.context/vcs.md`) handles mixed snapshot↔commit (normalizing the snapshot side into VCS object ids), the working-tree side, and per-file line counts (via `similar`). |
| `apps/desktop/src/tabs/RouteLink.tsx` | Browser-style link button + the `useRouteDispatch(ref, { onNavigate?, pinnedSlot? })` hook that powers it. Click semantics: left-click → in-tab navigate via `PageNavigationContext` (or `onNavigate` fallback when no context, e.g. rail / palette), Cmd/Ctrl-click + middle-click + right-click → new tab. The hook returns `{ dispatch, handlers }` so non-button rows (file tree entries, note rows, …) can adopt the same semantics without becoming a `<button>`. |
| `apps/desktop/src/components/RailHud/RailHud.tsx` | Persistent left rail HUD: a pinned search trigger at the top, then a set of **uniform collapsible panels** (every one an extension's — Go To, Uncommitted, Comments and Work from `oxplow-bundled`). Each renders through the shared `RailSection` wrapper — drag handle (⠿) + expand/collapse chevron + title + optional count badge + optional header action. (What needs the person — proposals, failed operations, undelivered events, firing badges — isn't a rail section: it's the status bar's bell, the Alerts page and toasts; see `.context/usability.md`.) Sections **drag-to-reorder** (MIME `application/x-oxplow-rail-section`, the layout — order, collapsed, hidden — persisted in `panel_layout`; see "Left-nav panels" below). An extension panel with a `collapsed` lens (Work) keeps that summary when collapsed; the rest hide their body. Every section **always renders** (stable list) with an empty-state line ("Working tree clean", "No open comments", …) when it has no content. Passive — never auto-opens tabs. Go To is the `oxplow-bundled` panel `go-to` (bookmarks, then recent or most visited pages). |
| `apps/desktop/src/components/Navigator.tsx` | Far-left combined **stream + thread navigator** (each stream's open threads — `get_thread_state` leaves closed ones out; they're on the Closed Threads page): a 40px always-visible strip of letter glyphs (`navigator-strip-stream-<id>` / `navigator-strip-thread-<id>`); clicking a glyph navigates directly — a thread glyph selects that thread, a stream glyph switches streams via `onSwitchStream`. **Hovering a glyph shows its full title as a native tooltip and nothing else (tsk269).** The `navigator-overlay` panel (~280px, covering the strip *and* the rail HUD to its right) expands only on an explicit click: the bottom-pinned `navigator-expand` chevron, dead space in the strip (`navigator-strip-empty`, `e.target === e.currentTarget`), a stream glyph (which also switches to that stream, then shows its threads), or the already-selected thread's glyph (selecting it again would do nothing). The chevron is pinned *outside* the scroll container and is the load-bearing affordance — once the list scrolls there is no dead space left, and `+ Add stream` lives only in the panel. The open/close state machine — click-to-open, geometric pointer-leave, Escape, outside-press, background-click, and the passive-vs-explicit guard — lives in the shared `apps/desktop/src/components/useSlideoutStrip.ts` hook (chevron in `SlideoutChevron.tsx`), which the Terminal page's `TerminalTabStrip` also runs on; a new slide-out strip adopts the hook rather than re-deriving it. It dismisses on a click in its own **dead background** (anything resolving to `button/input/select/textarea/a/label/[role=button]` is left alone so controls behave normally), on the pointer leaving its bounds (180ms grace), on Escape, and on any pointerdown outside it. The last two are *explicit* dismissals and beat the mid-rename / mid-new-thread form guard; the first two don't. **Pointer departure is measured geometrically** (document `pointermove` vs. the panel's rect), never `mouseleave` — the panel covers the rail and sits in the same DOM subtree as the strip, so the pointer never "leaves" the wrapper while parked over the covered region, which is what used to strand it open on top of the rail and swallow clicks meant for it (tsk131). Each overlay row (`navigator-stream-row-<id>` / `navigator-thread-row-<id>`) — and each strip glyph, with the same menu — opens its action menu on **right-click** (Menu key / Shift+F10 for keyboard), headed by the stream's or thread's name (`context-menu-header`); Rename and Add thread open the panel for their inline field. **Thread menu:** a read-only (non-writer / queued) thread leads with **"Make writer"** (`menu-item-thread.promote`) → runs the `oxplow.thread.promote` command (via `onPromoteThread` → `App.handlePromoteThread`), making it the stream's single active writer and demoting the prior one; the active writer omits the item (its accent pill already signals it). Then Rename / Settings / Close. The write guard makes every non-active thread read-only, so this is the discoverable path out of "project file edits are blocked" (tsk132). |
| `apps/desktop/src/tabs/bookmarks.ts` | The person's bookmarks, project data (data-model.md "bookmark"): `useBookmarks(threadId, streamId)` reads `v_bookmark` — a page once, at the narrowest scope the thread sees (thread / stream / project) — and `setBookmark` / `removeBookmark` run `oxplow.bookmark.set` / `oxplow.bookmark.remove`. Pages bookmark via the `PageNavigationContext.bookmark` binding (one `scope`; the star's menu moves it or, on its current scope, takes it off); the Go To page's manager re-scopes and removes (its toast's Undo is the command's undo). |
| ~~`apps/desktop/src/tabs/appPageBacklinks.ts`~~ | **Deleted.** Per-kind in-memory backlinks providers used to live here. Cross-page backlinks now come from the persisted `page_ref` graph (`crates/oxplow-db/src/page_ref_store.rs`) via the `list_backlinks` IPC; every page kind goes through the same code path. App pages that need their own provider would register a new `source_kind` writer in the backend instead. |
| `apps/desktop/src/pages/WorkItemPage.tsx` | Every work item's page (`work_item:<provider>:<id>`), whichever list is active: title and body edited in place (`TaskDetail`), the rail's state pill and the list's declared fields (`TaskDetailRail`), activity (its efforts); Parent, thread/backlog moves, Comment…, Link… and Delete only as the list's features allow (`v_capability_provider`); the `work_item.detail.body` / `.sidebar` slots and the `work_item.detail.state` replacement (beside the state pill, never instead of it). See [work-items.md](./work-items.md). |
| `apps/desktop/src/pages/GitCommitPage.tsx` | Single-commit page (`commit:<sha>`): commit metadata (collapsing a long body behind "Show more") with cherry-pick / revert, the changed files (`ChangedFilesTree`, parent → commit diffs) and the `vcs.commit.details` lens slot (`change_id` from `useChange`). Routed via `gitCommitRef(sha)`. |
| `apps/desktop/src/components/RailHud/sections.ts` | Pure helpers: `computeActiveItem`, `computeUpNext`, `computePagesDirectory`. The pages directory is a pure function so it can be unit-tested without mounting the React rail. |
| `apps/desktop/src/pages/GitDashboardPage.tsx` | Committed-history rollup: branch header (current branch + upstream + ahead/behind + push), small uncommitted mini-card that links to `UncommittedChangesPage`, recent commits rendered through the shared `CommitGraphTable` (last 5, current branch only via `getGitLog({ all: false })`; click a row → reveal in `GitHistoryPage`), worktrees row with per-row "Merge into current", a **"Merge readiness" card** (cross-stream divergence vs the integration branch via `listStreamDivergences()` — per-stream ahead/behind + a clean/will-conflict/integrated badge, naming the overlapping files on conflict, with a one-click "Merge into &lt;base&gt;" offered only while viewing the base stream), recent remote branches with per-row pull/push. Each action asks first exactly when its command's spec does (`SpecConfirm`, tsk898: merge and rebase, which are `Destructive`; pull and push are `Never`, as everywhere else they run). Routed via `gitDashboardRef()`. |
| `apps/desktop/src/components/History/CommitGraphTable.tsx` | Pure presentation of the git-log graph (branch/merge dots + lines + sha + ref badges + subject + author + relative date). Used by both `HistoryPanel` (full list with detail pane; `GitHistoryPage` wraps it, with a `vcs.history.sidebar` side column when an extension mounts there) and `GitDashboardPage`'s recent-commits card. `indexRefsBySha(log)` exported alongside groups branch heads + tags by sha so callers feed identical maps. |
| `apps/desktop/src/pages/UncommittedChangesPage.tsx` | Working-tree changes: a Commit-all form, the changed files (`ChangedFilesTree` via `useChangedFiles({ kind: "working" })`, with a `N changed · +a −d` heading), the `vcs.status.header` strip above them (`stream_id`) and the `vcs.status.details` lens slot below. Distinct from `FilesPage` which is the full project file tree. Routed via `uncommittedChangesRef()`. |
| (change analysis) | No page of its own: `crates/oxplow-app/src/change_analysis.rs` stores each change behind `v_change*`, and the oxplow-bundled `change-review` lens grid shows it in the `vcs.commit.details`, `vcs.status.details` and `effort.review.details` slots. The old per-page scope drilldown is gone; scoped tab ids reopen the unscoped page. |
| ~~`apps/desktop/src/tabs/backlinksIndex.ts`~~ | **Deleted.** The in-memory cross-kind indexer is replaced by the persisted `page_ref` table; see `data-model.md`. The `BacklinkEntry` type that renderers consume now lives in `apps/desktop/src/tabs/backlinkTypes.ts`. |
| `apps/desktop/src/tabs/useBacklinks.ts` | React hook that calls the unified `list_backlinks` IPC for a `TabRef` and maps the returned `BacklinkEdge` rows into `BacklinkEntry`s. Used by every page kind including `FilePage` (which previously rendered nothing). The sibling `usePageOutbound` hook does the same for the inverse direction. |
| `apps/desktop/src/tabs/backlinkTypes.ts` | Renderer-side `BacklinkEntry` interface (`{ ref, label, subtitle? }`). Decoupled from the SQLite `BacklinkEdge` shape. |
| `apps/desktop/src/tabs/BacklinksList.tsx` | Default renderer for the Page chrome's `backlinks` slot — buttons that route via `onOpenPage`. |
| `apps/desktop/src/pages/WikiPage.tsx` | Single-record page for a wiki page (`wiki:<slug>`), rendered through `Page` so it gets the unified chrome (title via `usePageTitle`, back/forward + star, backlinks). Edit/Save/Revert/Delete live in a thin toolbar inside the body. In-tab wikilink clicks route through `PageNavigationContext.navigate(wikiPageRef)` so they join tab history. |
| `apps/desktop/src/pages/CommentsInboxPage.tsx` | Global **Comments Dashboard** (`comments` index kind, `commentsRef()`; titled "Comments Dashboard"). Lists every comment in the current stream (`listCommentsForStream`), grouped by target. The body of a **row opens that comment's full `CommentPopover` inline** (read/reply/intent/resolve/delete — triage the whole backlog from one place); a **group-header click jumps to the target page** (file/wiki/task). Each row also has a **"Go to location" button** that navigates to the target page *and* scrolls to / opens the anchored comment via `comment-reveal-bus.ts` (disabled for orphaned comments). **Defaults to unresolved threads only**; a "Show" dropdown reveals resolved threads bucketed by recency. The bucket thresholds are a tiered ladder — one per day to a week, one per week to a month, then one per month beyond — capped at the oldest actual resolved comment so the largest option reaches all of them (helpers in `comments-filter.ts`: `resolvedWindowOptions` / `visibleThreads`, keyed on `comment.resolved_at`). The holistic "review them all" surface; the agent reaches the same data via the `list_comments` MCP tool. |
| `apps/desktop/src/pages/DashboardPage.tsx` | The **Go To** page (`dashboardRef("visits")`, titled "Go To"): bookmarks with inline scope management, recently and most visited pages. The old Planning / Review / Quality variants are oxplow-bundled lenses now. |
| `apps/desktop/src/components/Analytics/DailyBarChart.tsx` | Generic daily bar chart (`{ label, value }[]`); renders `bar` lenses. |
| `apps/desktop/src/pages/StreamSettingsPage.tsx` | Per-stream settings page (custom prompt). Replaces the in-rail StreamRail settings modal. Routed via `streamSettingsRef(streamId)`. |
| `apps/desktop/src/pages/ThreadSettingsPage.tsx` | Per-thread settings page (custom prompt). Replaces the in-rail ThreadRail settings modal. Routed via `threadSettingsRef(threadId)`. |
| `apps/desktop/src/components/CollapsibleSections.tsx` | Collapsible page sections (tsk84, tsk86). Three parts: `CollapsibleSections` (state provider), `CollapsibleSection` (chevron header inside the `<h2>` + hideable body), `SectionCollapseControls` (the "Expand all" / "Collapse all" pair). Sections register themselves on mount so the controls know what "all" means — only what's **currently rendered** counts, so a filtered-out section is never silently expanded — and the controls self-hide when nothing is registered. **The page places the controls**: Recorded Metrics renders them in the **details rail** beside its filters, which works because the provider wraps the whole `<Page>` and `rightRail` is *rendered* inside Page's subtree (context follows the render tree, not the creation site). For that to be safe the provider renders `children` **bare** — a wrapper element there would break the page chrome's `height: 100%` column. Collapsed ids persist per page in `localStorage` (`oxplow.page.sectionsCollapsed.v1`, keyed by `pageKey`); default is expanded, and stored ids are deliberately **not** reconciled against the rendered set (a section hidden by a page's search must return still collapsed). Pure state/persistence in the sibling `sectionCollapse.ts`. **Not a `Page` prop** — see `.context/usability.md` → "Collapsible page sections". Adopters: `RecordedMetricsPage`. |
| `apps/desktop/src/components/Slideover.tsx` | Right-edge panel primitive (~38vw, backdrop-click + Escape close, focus-into-panel on open) for form-shaped flows that don't justify a full page. Use instead of a centered modal. |

## Page kinds

`PageKind` (`apps/desktop/src/tabs/tabState.ts`) is the source of truth;
this list mirrors it. Grouped by what they are for. Kinds marked **(→
ext)** are analytics pages slated to move into the `oxplow-bundled`
extension as `lens:<slug>` pages ([extensions.md](./extensions.md), epic
tsk275) — don't grow them; new instruments should be lenses once the
extension host lands.

- **Agent & work:** `agent`, `work_item`, `tasks`, `done-work`, `backlog`,
  `archived`, `new-task`, `new-stream`, `stream-settings`,
  `thread-settings`, `closed-threads`, `comments`, `hook-events`,
  `alerts`
- **Code & review:** `file`, `dir`, `files`, `diff`, `diff-view`,
  `uncommitted-changes`, `commit`, `git-history`, `git-dashboard`,
  `local-history`, `local-history-full`, `local-history-by-commit-full`,
  `terminal`
- **Knowledge:** `wiki`, `wiki-index`, `wiki-freshness`
- **Lenses:** `lens` (`lens:<extension>/<slug>`, user/agent-built from
  `oxplow/extensions/`; see [extensions.md](./extensions.md))
- **System:** `settings`, `external-url`, `dashboard` (the `visits`
  variant is the Go To hub, which stays core)
- **Data (the core explorer; stays core):** `explore-data`, `metrics`,
  `metrics-recorded`, `metric`,
  `custom-dashboard`, `dashboards`
- **Analytics (moved to ext):** oxplow-bundled lenses (`usage`,
  `planning`, `review`, `quality`, `findings`, `effort-tests`).
  `duplicate-block` stays core as the side-by-side compare page (lens
  `compare` links).

`agent` is implicit per thread. There is no `change-analysis` kind.

## Tab id format

Built only by the helpers in `apps/desktop/src/tabs/pageRefs.ts` — never
hand-format an id.

**Every tab id is a canonical ref** ([refs.md](./refs.md)), parsed
through the shared grammar (`apps/desktop/src/refs/ref.ts`) so a `:`
inside an id (`work_item:oxplow:tsk42`) never splits it. Two families:

- **Entity pages** (`EntityPageKind`): the tab id of a file, directory,
  wiki page, task, commit, metric or lens *is* its ref, so a `[[…]]`
  link, a `page_ref` row, a backlink and a tab all agree on one string.
- **Shell routes** (`RoutePageKind`): a page of the shell, not a thing
  in the graph — `page:<name>[?params]` (`page:tasks`,
  `page:diff-view?effort=eff9`). The route name is the `TabRef.kind`.
  Routes never appear in `page_ref`. Params are a query string whose
  values escape only what would break the id (`&`, `=`, `+`, `%`, `@`,
  `#`, whitespace), so `path=src/a.ts&left=ref:abc` stays readable; the
  raw text after `page:` is what gets parsed, not `parseRef`'s decoded
  id.

`refFromTabId(id)` rebuilds the full ref (with payload) from an id alone
and returns `null` for text that isn't a ref, an unknown kind, or a route
whose params don't rebuild — callers (rail History, Go To, the launcher's
Recent) drop the row rather than open a blank tab. The `ROUTES` table is
`Record<RoutePageKind, …>`, so a new route can't ship without its
inverse; the round-trip test in `pageRefs.test.ts` covers every
constructor. `newTaskRef(payload)`'s defaults deliberately aren't in the
id (stable id → one form tab), so a history reopen starts empty.

There are no sentinel ids: the agent tab is `AGENT_TAB_ID`
(`page:agent`), and "is this the editor's working-tree file" is
`diskFilePath(id)` (null for a pinned revision such as
`file:src/a.rs@git:HEAD`, which is a read-only viewer tab).

**No compatibility layer for old ids.** When a kind is renamed or a page
moves into an extension, its saved tabs/bookmarks/history are dropped
(the `oxplow.layout.v2.*` keys started fresh on 2026-09-28; bookmarks
moved into the project DB and started fresh again) — see the decision in [refs.md](./refs.md).

| Kind | Id format | Example |
|---|---|---|
| file | `file:<path>[@<rev>]` — the working tree has no rev; `@git:<ref>` / `@snap:<id>` pin a version (`revisionSlot` in `revision.ts`); a literal `@`/`#`/`%` in a path is percent-encoded | `file:crates/oxplow-app/src/lib.rs`, `file:src/a.rs@git:HEAD` |
| dir | `dir:<path>` | `dir:crates/oxplow-app` |
| wiki | `wiki:<slug>` | `wiki:how-stop-hook-fires` |
| work_item | `work_item:<provider>:<id>` — every list's item (`workItemTabRef(ref)`, payload `ref`) opens `WorkItemPage` | `work_item:oxplow:tsk142`, `work_item:fake:W-1` |
| commit | `commit:<sha>` | `commit:abc1234` |
| metric | `metric:<key>` | `metric:oxplow.coverage.abs_pct` |
| lens | `lens:<extension>/<slug>[?param=value…]` (params sorted; `lensRef(id, params)`) | `lens:review/waiting-on-me`, `lens:oxplow-bundled/effort-tests?effort_id=12` |
| ext-page | `page:ext.<extension>.<page>` (`extPageRef`) — an extension's `pages:` entry, its lens full-page (`ExtensionPageView` resolves the page from the stream's extensions, so a restored tab opens too, and titles the tab with the page's manifest `title` through `LensPage`'s `title`). Not a named route: `PageKind` includes `ExtensionPageKind`, and `refFromTabId` / `pageKindOf` recognise the `ext.` head (the page id, which has no `.`, follows the last one). Params ride the id (`?ref=`) into the lens's `initialParams`: a ref of an extension's kind (`acme_pr:12`, P8.D7, extensions.md "Ref kinds") opens its kind's page this way | `page:ext.github.open-prs`, `page:ext.acme.pr?ref=acme_pr:12` |
| symbols | `page:symbols[?path=<p>]` — one file's outline, or the project's symbols (filtered) | `page:symbols?path=src/lib.rs` |
| symbol | `symbol:<path>/<name>@snap:<id>` (`v_symbol.ref`, `symbolRef(ref)`) — never a tab: opening one (`handleOpenPage`, or in-tab navigation) resolves it through `v_symbol` and opens its file at the name's line (`openSymbol`, P6.E3) | `symbol:src/lib.rs/Widget::spin@snap:12` |
| agent | `page:agent` (`AGENT_TAB_ID`) | `page:agent` |
| index routes | `page:<kind>` — `tasks`, `done-work`, `backlog`, `archived`, `wiki-index`, `files`, `comments`, `local-history(-full\|-by-commit-full)`, `git-history`, `git-dashboard`, `uncommitted-changes`, `hook-events`, `terminal`, `settings`, `metrics-recorded`, `dashboards`, `explore-data`, `catalog`, `board`, `problems`, `closed-threads`, `new-stream`, `new-task` | `page:tasks` |
| diff | `page:diff?path=<p>&left=<ver>&right=<ver>[&label=<l>]` (versions `disk` / `ref:<x>` / `snap:<id>`; `diffRef(spec)` / `computeDiffId(spec)` — `revealLine` is not in the id, so re-clicking reuses the tab) | `page:diff?path=src/a.ts&left=ref:abc&right=disk` |
| diff-view | `page:diff-view?snapshot=<N>` \| `?effort=<effortId>` \| `?start=<tok>&end=<tok>` (endpoint tokens `s<snapshotId>` / `c<sha>` / `w` / `none`) | `page:diff-view?effort=eff42` |
| duplicate-block | `page:duplicate-block?left=<p>&left_lines=<a>-<b>&left_at=<ver>&right=…` | `page:duplicate-block?left=a.rs&left_lines=1-5&left_at=disk&right=b.rs&right_lines=9-13&right_at=ref:abc` |
| wiki-freshness | `page:wiki-freshness?slug=<slug>` | `page:wiki-freshness?slug=data-model` |
| dashboard | `page:dashboard?variant=visits` (the Go To page) | `page:dashboard?variant=visits` |
| custom-dashboard | `page:custom-dashboard?id=<dashboardId>` | `page:custom-dashboard?id=dsh3` |
| stream-settings / thread-settings | `page:stream-settings?stream=<id>` / `page:thread-settings?thread=<id>` | `page:thread-settings?thread=thr3` |
| alerts | `page:alerts` | `page:alerts` |
| external-url | `page:external-url?url=<url>` (`=`, `&`, `#` in the URL are escaped) | `page:external-url?url=https://example.com/path` |

## Left-nav panels (P6.G1)

The rail's sections are **panels** (`components/Panels/panelLayout.ts`):
every enabled extension's `panels:` — core has none of its own since Go
To moved into `oxplow-bundled`; with it disabled the rail is
the search box and + Add Panel
(`ext:<extension>/<id>`, rendered by `ExtensionPanelSection`: the body
lens compact; the header's count — the `count` lens's row count (or
`number`), else the badge lens's alert count while it fires
(`panelCount`); collapsed, the `collapsed` lens compact as its summary,
else nothing; the header's ↗
opening the panel's `open` page or else its body lens, `panelOpenRef`).
A compact lens keeps its `group` headings and group actions (the
toolbar is hidden), and its linked rows drag into the agent's context.
The rail's panels are moving out of core into `oxplow-bundled`: only their UI moves, as lenses over the existing views and
commands; all three are done (`ext:oxplow-bundled/comments`, `ext:oxplow-bundled/work`, `ext:oxplow-bundled/uncommitted`). One owner
holds every panel's runs — `PanelRunsProvider`
(`components/Panels/PanelRunsContext.tsx`, around the app),
read by the rail, the status bar's bell and the Alerts page — through
`components/Panels/usePanelRuns.ts` → `useExtensionPanelRuns`: it binds each panel's scope — `stream_id` /
`thread_id` as row ids, from `panelParams` — for the stream and thread it
shows, re-runs when either changes or when a read changes, runs each
distinct lens of a panel once whatever roles it plays, and hands each
section its runs. The firing badges come from the same runs
(`panelAlerts`), so a badge never runs twice. What needs the person —
proposals, failed operations, undelivered events, firing badges — isn't
a rail panel: it's the status bar's bell and the Alerts page (see usability.md). A
layout stored before a core panel existed gets it appended at the bottom.
The set of extension panels (like extension pages, slot mounts and the
prompt catalog) reloads on `lensRerun.extensionsChanged`: a file under
`oxplow/extensions/` changed, or the config did — enabling or disabling
an extension is a `configChanged`, so its panel appears or leaves at once.

The person's layout — order (drag the ⠿ handle), collapsed (the chevron),
hidden (right-click a header → Hide Panel; **+ Add Panel** at the bottom
brings one back) — is one list in the local table `panel_layout` (V126,
never the repo), read and written through the UI RPCs `get_panel_layout`
/ `set_panel_layout`. `resolveLayout` reconciles it with the panels
available now: stored order first, ids that are gone dropped, new panels
appended expanded. The old `oxplow.rail.*`
localStorage order and expanded keys are gone. The stored layout loads
after the first render: an edit made before it arrives (a chevron, a
drag) shows at once and is queued, then replayed on the loaded layout and
saved — nothing is written over a layout not read yet, and a late load
never undoes an edit (tsk972). So every edit means the same on any base:
absolute (`setCollapsed(panel, collapsed)`, not a toggle) or relative to
a panel (`movePanelBeside(panel, target, side)`, not an index). One
`layoutSync` (`panelLayout.ts`, tsk998) owns the round trip: a **failed
load** is reported and shows the defaults, but saves nothing over the
layout it couldn't read — the next edit loads again and is replayed on
what it finds; saves go **one at a time, the latest last**; and an edit
**keeps the placement of a panel not available yet** (an extension's,
before extensions load) right after the entry it followed.

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
on `"full"`. Adopters today: see WikiPage and WorkItemPage.

## Rail HUD contract

The rail is **read-only with respect to tabs** — it never auto-opens a tab.
Every rail click goes through a single `onOpenPage(ref: TabRef)` callback
that the host wires to its own routing. Sections are a flat stack (no
zone grouping); each appears only when it has content:

1. **Search trigger** — opens the launcher (`QuickOpenOverlay`), the
   single discovery surface. Always visible.
2. **Work** — an `oxplow-bundled` panel, thread-scoped, over
   its model `thread_work` (one row per line, per thread), which reads the
   work-item interface (`v_work_item`): whichever list is active, nothing
   with none. The body lens `work` groups the lines (`group`): "In
   progress" (the active item; under its epic, with the epic's other
   children indented and the active one emphasized, when it has one, plus
   an open effort no item names), "Ready" (10, by `rank` then
   `created_at`) and "Finished" (5 done items, wiki pages and unlinked
   efforts, its heading opening the thread's activity, with a `clear`
   group action: `oxplow.work.clear_finished` records an event and the
   `finished_cleared` model hides what finished before it — per thread,
   no longer browser-local). Status and kind show as row icons. Collapsed,
   the `work-line` lens: the active item (with its epic), else the last
   finished. The header count (`work-count`) is in progress + ready, never
   an alert. Rows link (Cmd-click opens a new tab) and drag into the
   agent's context.
3. **Uncommitted** — an `oxplow-bundled` panel, stream-scoped,
   over the working change's stage-one file list (`v_change_file` /
   `v_change.conflicted` / `in_progress`, kept current by `change.analyze`
   on every move — the app no longer polls git for it). The body lens
   `uncommitted` is a folder tree: each file its A/M/D (toned) linking to
   the file, each folder the letters under it, a merge or rebase in
   progress as a red row on top. Collapsed, `uncommitted-line`: `3A 2M 1D
   +10 −4` and the conflict line, opening the Uncommitted page; the count
   (`uncommitted-count`) is the changed files.
4. **Comments** — an `oxplow-bundled` panel: the stream-scoped
   lens `comments` over `v_comment` shows two open-comment count rows split
   by intent — "For me" (`note`) and "For the agent" (`followup`) — each
   row and the header (`open: page:comments`) opening the Comments inbox.
5. **Go To** — an `oxplow-bundled` panel (`ext:oxplow-bundled/go-to`):
   the thread-scoped lens `go-to` lists the bookmarks the thread sees
   (`v_bookmark`), then its History — or, through the header's
   Recent / Most visited toggle (the lens's `mode` choice param), its most
   visited pages over 30 days with their counts — from `v_page_visit`,
   each row with its page's icon; collapsed, `go-to-bookmarks` shows the
   bookmarks alone. There is **no "Pages"
   section**: the launcher is the one discovery surface for all pages,
   and users pin what they want always-visible by starring (the ☆ on each
   page's nav bar, scoped thread / stream / project).
   `computePagesDirectory` is the launcher's page list (the old
   `RAIL_PAGE_IDS` filter is gone); every page carries a `category` (Work
   / Code / Git / Activity / Knowledge / System) so the launcher empty
   state reads like a start menu. The ↗ opens the **Go To** page
   (`dashboardRef("visits")`) for the full hub + bookmark management.
   With a query, pages rank by `pageRank` (tsk1031): every token starting
   a word of the title, then the title as a subsequence, then the id, then
   the `keywords` — where each token must start a word, since a lens's
   keywords hold its whole description and a short subsequence ("todo")
   is in almost any description.

## History: the IA redesign (complete)

The web/Linear-style IA redesign shipped in full: theme foundation, tab
store + page chrome + refs, rail HUD, every dock panel migrated to a
page, record pages + backlinks, modals replaced by inline edits /
slideovers / page forms, the selection action bar, and density polish.
Both the left and bottom docks are **gone** — the rail HUD is the only
left chrome and pages are the only center surface. One reversal stuck:
per-row actions are **right-click only** (the kebab `⋯` migration was
undone in tsk168; see [usability.md](./usability.md) → "Per-row actions").
Git history has the phase-by-phase detail if you need it.

The next structural change is not an IA change but a scope change:
analytics pages move out of core into the `oxplow-bundled` extension as
`lens:<slug>` pages mounted through slots ([extensions.md](./extensions.md)).

## The title bar

`components/TitleBar.tsx` spans the top of the window, over both the left
nav and the content (decided 2026-10-07): `stream › thread` at the
start, and at the right end the stream's branch (`BranchPicker`) with the
global search field just after it. The stream and thread names open the
navigator panel (`navigator-bus.ts`, `requestNavigatorOpen`); right-clicked,
they ask the Navigator for that stream's or thread's menu at the pointer
(`requestNavigatorMenu`) — the same menu as its rows, which the Navigator
owns. The branch opens the branch picker. On macOS it *is* the window's top (an
Overlay titlebar): its start leaves 78 px for the floating traffic lights,
and its empty space carries `data-tauri-drag-region` (Tauri drags only from
elements that carry it, so the controls stay clickable). Elsewhere it sits
under the menu bar. The bottom bar keeps only the alerts bell and the
background-task indicator (`StatusBar.tsx`). Testids: `title-bar`,
`title-bar-stream`, `title-bar-thread`, `title-bar-search`.

## One Search — the single discovery surface

There is exactly **one** way to discover and reach pages: the launcher
(`QuickOpenOverlay`). It is opened by **Cmd+P** (the `file.quickOpen`
menu command) and the title bar's **Search…** field (`title-bar-search`,
`SEARCH_TRIGGER_TESTID`; the overlay opens over it). Cmd+K and
Cmd+Shift+F are kept only as **aliases** that open the same launcher, so
the old command-palette / find-in-files reflexes still land somewhere
useful — there is no separate command palette or search overlay anymore
(`CommandPalette.tsx` and `SearchPalette.tsx` were deleted).

The launcher does everything in one box:
- **Empty input** → start-menu: a **collapsible tree** led by a **"Recent"**
  section, then the page `category` headers (Work / Code / Git / Activity /
  Knowledge / System). Categories are **collapsed by default** so the empty
  launcher is a short list of sections rather than all ~21 pages; **Recent
  is expanded by default** (it exists to *show* the recent pages). Expanding
  a section reveals its pages (indented). The tree assembly is the pure
  helper `buildLauncherTree(recent, pages, expanded)` in
  `quickOpenResults.ts`; section rows and page rows share one navigable
  `Row[]` so the keyboard cursor and render can't drift (Enter toggles a
  section, opens a page).
  - **Recent** = the 10 most recently visited pages for the active thread
    (`listRecentPageVisits({ dedupeByRef, excludeKinds:
    RAIL_HISTORY_EXCLUDE_KINDS })`, the same source as the rail History
    block), kept live via `subscribePageVisitEvents`. Visit rows don't
    persist the ref payload, so `buildRecentEntries` rebuilds each openable
    ref with `refFromTabId` and `recent:`-prefixes the id to avoid colliding
    with a static directory page. `LauncherSection = PageCategory |
    "Recent"`; recent rows are the lighter `LauncherPageEntry` (no static
    category). The section only renders when there are visits, and self-hides
    while searching (recent entries never enter `buildQuickOpenResults`).
  - **Persistence:** static category expansion persists in `localStorage`
    (`oxplow.launcher.expandedCategories`); Recent's collapse persists
    separately (`oxplow.launcher.recentCollapsed`, default un-collapsed) so
    its default-open state doesn't disturb existing category prefs. The
    effective expanded set is `expandedCategories ∪ {Recent unless
    collapsed}`.
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
testids are gone); Go To's bookmarks are the always-visible curated nav. E2e
probes that used to click `rail-page-*` should drive `title-bar-search` → the
launcher (type, then assert `page-<kind>` on the body).

**Work pages split (post-Phase-3).** The single `AllWorkPage` was
replaced by four focused pages so each has one job:

- **Tasks** (`page-tasks`) — thread-local task manager (formerly
  "Plan work", `page-plan-work`). Shows To Do + Blocked in full
  plus last-5 Done previews. The In Progress section is omitted
  because the rail HUD's "Active item" + "Up next" already surface
  it. Header link "View all done →" routes to Done Work; kebab
  carries the `hide-auto` filter and a "View backlog →"
  entry. PageKind is `"tasks"`; ref helper is `tasksRef()`.
- **Done work** (`page-done-work`) — full descending list of the
  current thread's closed items (done and canceled).
- **Backlog** (`page-backlog`) — the active list's items on no thread
  (`v_work_item.thread_id IS NULL`), for a list with the `lists`
  feature; `oxplow.work_item.move` takes one to a thread and back. The
  `backlogReadyCount` badge in the rail directory hangs off this entry.

All three read the work-item interface (`workItems.readWorkList`) and
wrap `PlanPane`, which takes the active list's profile
(`useWorkListProfile`: its features and declared fields) and offers only
what the list can do: drag reordering with `ordering`, the backlog with
`lists`, epics with `hierarchy`, Delete with `delete`. Its rows show the
list's editable enum fields (oxplow's priority), and the Tasks page's
filter bar a chip row per declared enum field. They pass filter props
(`visibleSections`, `sectionItemLimit`, `onlyStates`, `excludeStates`,
`extraSectionLinks`, `forceMode`, `hideBacklogChip`). There's no
Archived page: oxplow's archived tasks are done or canceled, as the
interface states them.

The four pages reuse the shared `<Card>` + `cardLinkButton` from
`apps/desktop/src/components/Card.tsx` for cross-page "View X →" affordances;
GitDashboardPage uses the same shell so the dashboard vocabulary is
consistent across IA.

Named ref helpers — `tasksRef()`, `doneWorkRef()`,
`backlogRef()`, `archivedRef()` — mirror the GitDashboard pattern
(`gitDashboardRef`, `uncommittedChangesRef`).

## Browser-style tab navigation

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
- `pages/RecordedMetricsPage.tsx` — each metric row passes the whole
  page's rows flattened in visual order (`metricSiblings` in
  `recordedMetricsRows.ts` — the chain continues across section
  boundaries), so a drilled-into metric detail steps through the
  metrics exactly as the page lists them (tsk119).

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
`TabRef`, regardless of kind (`wiki`, `file`, `diff`, `work_item`,
`lens`, `commit`, etc.). The page-tab loop in `centerTabs` looks the
renderer up in **`pageRenderers: Record<PageKind, (ref, nav) => CenterTab
| null>`** — one entry per kind, so a kind without a renderer (or a
renderer for a kind that no longer exists) is a compile error rather
than a blank tab — and wraps each tab in a `PageNavigationContext` so
in-tab navigation, back/forward, sibling navigation, and
bookmark/backlinks all work the same way. `nav` is the slot's
navigation (`navOpen`, `navOpenFile`, `navOpenDiff`, `navRevealCommit`,
`slotId`): closures bind to the *slot's* current ref, so a back-stack
page (still mounted, hidden) that navigates mutates the slot exactly as
the visible page would. `handleOpenPage` special-cases only `agent`,
`file` and `diff`; every other kind is "push the ref, activate it".

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

This is the architectural rule for new tab kinds: add a `PageKind`
(entity or route), add a `pageRefs.ts` constructor **and** its `ROUTES`
inverse, render through `Page`, and add the `pageRenderers` entry in
`App.tsx` (the two `Record`s tell you what's missing). **Don't** add a
parallel tab track.

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
  (`file`, `diff`, `wiki`, `work_item`, `uncommitted-changes`,
  `commit`, `tasks`, `git-history`, …) lives here. Mutated by
  `handleOpenPage`, `handleOpenFile`, `handleOpenDiff`,
  `handleNavigateInTab`, `handleStepSibling`, `closePageTab`. Read
  by the `centerTabs` builder + the `effectiveCenterActive`
  derivation.
- **`threadCenterActive: Record<string, string>`** — per-thread
  active tab id. Switching threads restores each thread's last
  active tab. Mutated by `setCenterActive` (which writes into the
  per-thread map for the current thread). `AGENT_TAB_ID` (`page:agent`)
  is the default fallback when nothing else is selected.
- **`threadPageMru: Record<string, string[]>`** — per-thread tab
  recency, most-recently-used first. In-memory (like
  `threadCenterActive`); rebuilt as tabs are activated. A `centerActive`
  effect `touchMru`s the active tab to the front; `closePageTab`
  `dropFromMru`s a closed tab. **Drives the page-tab cap:** a second
  effect enforces `MAX_PAGE_TABS` (15) on the active thread — when
  `threadPageTabs[thread]` exceeds it, `selectLruEvictions` (in
  `apps/desktop/src/tabs/tabLru.ts`, unit-tested) picks the
  least-recently-used victims and `closePageTab`s them. The **active
  tab and any dirty file tab are protected** (a soft cap never discards
  unsaved work — best-effort, so if every non-active tab is dirty the
  count can sit above 15). The pinned Agent tab is outside
  `threadPageTabs`, so it's never counted or evicted. Eviction routes
  through `closePageTab` (not the dirty-file undo-toast path) — protected
  dirty files never reach it, so evictions are silent and toast-free.

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
- **Bookmarks** — `v_bookmark` (thread / stream / project), keyed
  by `ref`; set and cleared via the tab nav-bar star button.

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
       └→ snap centerActive to AGENT_TAB_ID if it was the closed tab
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
`oxplow.layout.v2.threadPageTabs`, `oxplow.layout.v2.threadPageHistory`,
`oxplow.layout.v2.diffSpecs`.

What persists:
- The full per-thread tab list (every TabRef).
- The per-tab back/forward history + siblings record.
- Diff specs (the registry indexed by id) — except clipboard /
  selection-vs-clipboard diffs that carry inline `leftContent` /
  `rightContent`. Those are session-only.
- The active center tab id (`oxplow.layout.v2.centerActive`,
  unchanged from before).
- The per-stream open file paths (existing
  `oxplow.layout.v2.fileSessions` blob; loads file content on first
  stream activation).

Per-page state via `usePageSnapshot`:

Pages opt in via `usePageSnapshot<T>({ serialize, restore, deps })`
(see `apps/desktop/src/tabs/usePageSnapshot.ts`). On mount the hook
reads any saved blob keyed by the page's `pageKey` from
`PageNavigationContext` (`${threadId}::${tabId}`) and calls
`restore`. On each `deps` change it serializes and writes.
`closePageTab` clears the snapshot row so closed tabs don't leak.

Adopted pages (Phase 3):
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
  the rendered page tabs come from the new stream's selected
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
