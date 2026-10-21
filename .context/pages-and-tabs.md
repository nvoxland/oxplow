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
  menu → Add thread), mark one (thread menu → Mark with…), rename,
  open settings, close or remove; `+ Add stream` at the panel's foot
  opens the `new-stream` page. The launcher's New Thread… / New Stream…
  reach the same flows. See the Navigator row in "Modules".
- **Each thread owns its own set of open tabs and an active tab.**
  Switching threads restores its tab set; switching streams swaps to the
  selected thread of the new stream.
- **A thread's agents are its agent sessions, one tab each.** Every open
  `agent_session` row has a pinned tab (`agent_session:ses3`) leading the
  thread's tabs, in the order the sessions opened its terminal. The tabs follow the rows — `useThreadSessions` reads
  `v_agent_session`, and `reconcileSessionTabs` (`tabs/sessionTabs.ts`)
  keeps each open session's tab first and drops a closed one's — a
  session opened later still goes ahead of every other tab. **A new
  thread** (one with no tab list yet; they persist, so it's one never
  opened here) with no session opens as the person's `newThreadSession`
  says (`newThreadTabs` in `tabs/sessionTabs.ts`, applied by App's
  `openNewThread`): `ask` (the default) opens the **session picker**
  (`page:new-session`, `NewSessionPage`), `none` opens nothing, and an
  agent starts its session — or asks, if it can't start any more. The
  picker is an ordinary tab after that: its × closes it for good, and
  closing a thread's last session leaves its other tabs (or home), not
  the picker. It also lists the thread's **earlier sessions** (closed
  ones and every agent's saved conversations for the worktree), one
  column each for agent, session, status, last active, turns and size,
  each on one line (last active is relative, the exact time on hover; a
  value it doesn't have is a dash). A key under the filter names the
  status icons in use, and the rows that can't be taken (open in another
  thread) come last. The page is headed **New session**, as its tab is.
  Taking one open in the thread shows its tab and keeps the picker;
  reopening or resuming one, like **New Session**, closes the picker and
  activates that session's tab. A closed session's ref (a search hit)
  opens its transcript lens, not a missing tab. The picker says nothing
  of the thread's open sessions (its tabs show them) and has no way to
  decline one (its × closes it); its **Remember this for new threads**
  checkbox saves the agent started as `newThreadSession`; Settings → Agents has
  the same choice ("New threads start with",
  `components/NewThreadSessionSetting.tsx`). The launcher's New Agent
  Session… opens the picker, and each agent's command starts a session
  directly. A process survives switches; closing a session's tab closes
  the session.
- A **page** is anything addressable inside a tab body — file, task,
  wiki page, finding, dashboard, settings, agent session. Pages share a
  common chrome (header + collapsible Backlinks panel).

## Modules

| File | Purpose |
|---|---|
| `apps/desktop/src/tabs/tabState.ts` | `createTabStore()` — per-thread tab list + active id, with `openTab`, `ensureTab`, `activate`, `closeTab`, `subscribe`. In memory; no cross-restart persistence in v1. |
| `apps/desktop/src/tabs/useTabStore.ts` | `getTabStore()` singleton + `useThreadTabs(threadId)` hook backed by `useSyncExternalStore`. |
| `apps/desktop/src/tabs/pageRefs.ts` | Every tab id, built here and nowhere else: entity refs (`fileRef`, `directoryRef`, `wikiPageRef`, `taskRef`, `gitCommitRef`, `metricRef`, `lensRef`, `agentSessionRef`) and declared pages (`pageTabRef(id, params?, state?)`, `extPageRef`, typed wrappers such as `newSessionRef`, `diffRef(spec)`, `endpointDiffRef`, `settingsRef`; `pagePayload`, `isPage`, `recordedPageKind`). `refFromTabId(id)` is the inverse of every constructor; `pageKindOf(id)` names the kind a tab renders as; `diskFilePath(id)` is the one way to ask "which working-tree file is this tab". |
| `apps/desktop/src/tabs/Page.tsx` | Shared page chrome: title + kind chip + status chips + actions slot, optional **browser-style nav bar** (back/forward + bookmark + backlinks dropdown — auto-mounted from `PageNavigationContext` when present), body, collapsible legacy Backlinks region. Title can be passed as a `title` prop or registered programmatically by the page via `usePageTitle`; the chrome falls back to the context title when `title` is omitted. Every page shows the nav bar when it has a navigation context (a page outside one — a test — gets the plain header instead); a page doesn't opt out. `titleIcon` draws before the title (an agent's logo). **`titleInBody`** (default false) says the page renders its OWN title as an `<h1>` in the body (use the exported `pageH1Style` for consistency), so the chrome (nav bar / header) suppresses the title to avoid a duplicate — the tab-strip label via `usePageTitle` is unaffected. Adopters: `MetricDetailPage`, `DiffViewPage` (tsk137). **The body** takes `width?: "full" \| "reading"` and an independent `rightRail` (a 320px sticky control panel beside a body of either width); below ~960px the rail moves inline to the top of the body (`RailBody`) — "Body layouts" below. **Actions** are `PageAction[]` drawn in the nav bar, and `pin` adds Pin to Dashboard — "The nav bar is core's". Reads only semantic CSS variables (skin via theme). |
| `apps/desktop/src/tabs/PageNavBar.tsx` | Dumb nav-bar component: back/forward buttons, optional bookmark toggle, optional backlinks/outbound/snapshots dropdowns (popovers), and an optional **comment navigator** slot (`comments` ReactNode, rendered before Backlinks). Mounted by `Page` when context or explicit `navBar` prop is present. |
| `apps/desktop/src/components/Comments/CommentNavigator.tsx` | Self-contained per-page comment navigator for the nav bar. `useCommentsForTarget(kind,id)` → shows "Comments (N)", steps through anchored comments with ◀ ▶ (each `requestCommentReveal`s to scroll + open inline on the surface), and a dropdown lists all comments plus an **Orphaned (M)** section. Orphaned entries are clickable too: they `requestCommentReveal`, and the surfaces open the thread popover at a fallback position (no anchor to scroll to) so the comment is readable. The popover then offers a **"Relink to selection"** button (`CommentPopover.onRelink`, wired by RichTextField + MonacoCommentLayer) that re-attaches the comment to the editor's current selection — select the intended text first. (The right-click "Relink orphaned" path still exists.) Comment-bearing pages pass it via `Page`'s `commentsNav` prop (FilePage → file/path, WikiPage → wiki/slug, WorkItemPage → work_item/`<provider>:<id>`). Renders nothing when the page has no comments. Pure helpers (`partitionPageComments`, `stepComment`) live in `pageCommentNav.ts` (unit-tested) and are shared with the per-thread stepper: `CommentPopover` takes an `onStep(dir)` prop so an open comment thread shows ◀ Prev / Next ▶ buttons that scroll to + reopen the adjacent comment (wired by RichTextField + MonacoCommentLayer via `stepComment` + `requestCommentReveal`). |
| `apps/desktop/src/tabs/PageNavigationContext.ts` | React context exposing `{ navigate(ref, { newTab? }), goBack, goForward, canGoBack, canGoForward, setTitle, title }` to descendants of an active page tab. Wrapped around every non-agent center tab in `App.tsx`. `BacklinksList` reads it so default-click navigates in-tab. The `usePageTitle(title)` helper registers the page's current title with the host so the same string drives the chrome header AND the tab strip label — no per-page duplicate header markup. |
| `apps/desktop/src/pages/FilePage.tsx` | Thin Page wrapper around `EditorPane`. Calls `usePageTitle(basename + ● dirty)` so the file's name flows into the shared chrome title, and `useBacklinks(fileRef(path))` so wiki pages, tasks, commits, and findings that reference the file appear in the nav-bar Backlinks dropdown. EditorPane keeps owning Monaco / blame / context menus; the wrapper only provides chrome above. |
| `apps/desktop/src/pages/DiffPage.tsx` | Thin Page wrapper around `DiffPane` for diff tabs. Calls `usePageTitle(basename + (label))`. |
| `apps/desktop/src/pages/DiffViewPage.tsx` | The **explicit start→end diff view** (`diff-view` kind). Three modes via `DiffViewSpec`: `snapshot` (legacy `snapshotRef(N)` prev→N drill-in); `effort` (`effortDiffRef(effortId)` — resolves the effort's own start/end snapshot bracket via the `get_effort` IPC on load, carrying the `taskId`/`effortId`, with an "Effort is in progress" notice when the end is null); `turn` (`turnRef(turnId)` — the turn's start snapshot → end snapshot via `get_agent_turn`, start → working tree with a "Turn is still running" notice while it runs; a turn with no start snapshot shows an explanation instead of diffing the empty tree, `resolveTurnEndpoints`, tsk467 — the notice wording is `inProgressNotice(subject)`); `endpoints` (`endpointDiffRef(start, end)` — an explicit pair of revisions, `working` / `snap:<id>` / `git:<rev>`). All modes list the changed files through the `diff` RPC (`Trees`, `.context/vcs.md`) via `useChangedFiles({ kind: "endpoints" })` (`components/ChangedFiles/`), which also gives the two sides' `Revision`s for opening a file's diff. **Page shape (tsk343–346/370):** the page uses the `Page` **`width="reading"`** with a right rail. The `<h1>` title is `Changes: <effort title>` *when the diff is for an effort* (effort passed, OR the start/end snapshots line up exactly with an overlapping effort's bracket), else a comparison label — **`Commit comparison`** when both endpoints are git versions (commit / commit-pinned snapshot), else **`Snapshot comparison`**. The date/commit **range** is passed as the `rightRail`. When the diff **lines up with an effort** (effort passed, or a snapshot range matching an overlapping effort's bracket — `primaryTaskId`/`effortTitle`), the rail **leads with a "Task" row** (`diff-view-task-link`) naming the effort's task and linking to its `taskRef` (in-tab via `onOpenPage`); it self-hides for a non-effort range. Below it: a **date header** (`rangeDateLabel` in `diffViewModel.ts` — a single date when both endpoints fall on the same calendar day, a "start – end" range when they span multiple days, the single known date when only one endpoint is time-based, hidden when neither is) above two **selectable** fields — a "Start"/"End" caption beside an `EndpointPicker` dropdown (`apps/desktop/src/components/Diff/EndpointPicker.tsx`). The closed trigger shows a **time-only** label (so it fits on one line next to the caption); opening it lists the **20 newest snapshots on the same branch as the diffed endpoints** (`pickerBranch` / `snapshotsOnBranch` in `diffViewModel.ts` — reference branch is end snapshot's `git_branch` → start's → `stream.branch`; only snapshots *known* to be on a different branch are dropped, while unrecorded-branch snapshots (detached HEAD) are kept — a branch switch within the stream's worktree never mixes other branches' snapshots in), **constrained per side so the range stays valid** (`rangeEndpointOptions` — the Start list only offers snapshots *before* the current End, the End list only snapshots *after* the current Start, so you can't pick an inverted range; no constraint when the opposite endpoint isn't a snapshot), plus whichever snapshot the current endpoint sits on (so the selection is always present), with the **full date+time** and, for snapshots that pinned a commit, the short commit sha. Picking a snapshot rescopes the diff in place — `onOpenPage(endpointDiffRef(newStart, end))` / `endpointDiffRef(start, newEnd)`, which `navOpen` routes as an **in-tab navigation** (Back returns to the prior range) — so the user can expand/collapse the diff range. The picker's popover is `position:fixed` (escapes the rail panel's overflow), closes on pick / Escape / outside-click, and is disabled when there are no candidate snapshots. The range shows in the rail (or, when the rail collapses on narrow widths, as the same panel stacked at the top of the center column — handled by the `Page` base, see below). Then an optional **Concurrent Efforts** `<ul>` (every effort overlapping the range — `listEffortsOverlappingRange`, **scoped to the diffed snapshot's own stream** so other streams'/branches' efforts whose global snapshot-id windows merely overlap don't leak in — other than the one the diff is for, each linked to its `taskRef`), a **Files Changed** `<h2>` section rendering the shared collapsible `ChangedFilesTree` (A/M/D + zone badges + Expand/Collapse all) — *all* changed files, or only the effort's claimed files (`listEffortFiles`) when the diff was opened **for** an effort. The section says so when there are no test changes. Sections are **boxless** (h2 + content, no card); `ChangedFilesTree` takes `showFileCount={false}` here. **Effort header.** Opened *for* an effort, the title is `EffortHeader` (`pages/EffortHeader.tsx`): the effort's `v_effort.title` (its own, else its item's, else its first prompt's), edited in place with `InlineEdit` (`oxplow.effort.update`; clearing restores the default); a "Linked to" line — the item (opens its page) with Unlink, or "Not linked" — and "Link to task…", an `InlinePromptStrip` taking `tsk42` or a `work_item:` ref (`oxplow.effort.link`); while open, Close effort (`oxplow.effort.close`), else how it closed (`closed_by`). It re-reads on `v_effort` changing; the tab title follows its title. An unlinked effort in Concurrent Efforts is named by its `v_effort.title`, else "Unlinked work". **Section order:** title → (effort **description**) → the **`effort.review.details` slot** → Concurrent Efforts → Files Changed. When the diff is **for an effort** (passed or lined-up): the effort's task **description** renders just under the title (`MarkdownView` via `getTask`), and the `effort.review.details` slot (`LensSlots`, params `effort_id` + the effort's `change_id`) mounts extension lenses — oxplow-bundled's decisions/claims and oxplow-bundled's change analysis (`change-review`: summary, look-here-first, churn treemap, function and test changes, co-change, duplication, cross-zone imports), coverage and tests, metric deltas and agent nudges. Both self-hide for a non-effort range. Chrome/tab title is driven by `usePageTitle` (plain-text mirror of the h1). Pure endpoint-resolution logic (`resolveEffortEndpoints`, `resolveTurnEndpoints`, `inProgressNotice`, `resolveSnapshotEndpoints`, `previousSnapshotId`, `snapshotRange`) lives React-free in `apps/desktop/src/diffViewModel.ts`. `Trees` (`.context/vcs.md`) handles mixed snapshot↔commit (normalizing the snapshot side into VCS object ids), the working-tree side, and per-file line counts (via `similar`). |
| `apps/desktop/src/tabs/RouteLink.tsx` | Browser-style link button + the `useRouteDispatch(ref, { onNavigate?, pinnedSlot? })` hook that powers it; `ref` / `to` is a `TabRef` or a ref string (opened through `openRef`). Click semantics: left-click → in-tab navigate via `PageNavigationContext` (or `onNavigate` fallback when no context, e.g. rail / palette), Cmd/Ctrl-click + middle-click + right-click → new tab. The hook returns `{ dispatch, handlers }` so non-button rows (file tree entries, note rows, …) can adopt the same semantics without becoming a `<button>`. |
| `apps/desktop/src/components/RailHud/RailHud.tsx` | Persistent left rail HUD: a pinned search trigger at the top, then a set of **uniform collapsible panels** (every one an extension's — Go To, Uncommitted, Comments and Work from `oxplow-bundled`). Each renders through the shared `RailSection` wrapper — drag handle (⠿) + expand/collapse chevron + title + optional count badge + optional header action. (What needs the person — proposals, failed operations, undelivered events, firing badges — isn't a rail section: it's the status bar's bell, the Alerts page and toasts; see `.context/usability.md`.) Sections **drag-to-reorder** (MIME `application/x-oxplow-rail-section`, the layout — order, collapsed, hidden — persisted in `panel_layout`; see "Left-nav panels" below). An extension panel with a `collapsed` lens (Work) keeps that summary when collapsed; the rest hide their body. Every section **always renders** (stable list) with an empty-state line ("Working tree clean", "No open comments", …) when it has no content. Passive — never auto-opens tabs. Go To is the `oxplow-bundled` panel `go-to` (bookmarks, then recent or most visited pages). |
| `apps/desktop/src/components/Navigator.tsx` | Far-left combined **stream + thread navigator** (each stream's open threads — `get_thread_state` leaves closed ones out; they're on the Closed Threads page): a 40px always-visible strip of letter glyphs (`navigator-strip-stream-<id>` / `navigator-strip-thread-<id>`) — a stream is a tab in the guide line's grey (`--text-muted`) with dark text (`--text-on-stream-tile`), coming out of the window's left edge and rounded on the right only, with its threads as tabs under it, all flush with the left edge and butted up against each other, each filling its shorter row and stopping short of the right (in the centre tab strip's header tint, `--panel-header-bg` over `--surface-card`, like a regular tab; the selected thread's accent bar is inside its tab, down its left, not beside it; a marked thread shows its mark as a corner fold on its top-left in `var(--thread-mark-<colour>)` (`navigator-thread-mark`, title "Marked <colour>"), not an outline; stacked tabs are set apart by a 1px divider along each one's bottom but the last's, in the guide line's grey (`--text-muted`), so none falls under the stream's tab, and the strip's column is `--surface-app`); a grey guide line runs down the column's right, flush with the stream tab's right edge, from under the stream's tab, with a tick from each thread's tab into it, ending at the last, each thread's agent status dot a node where the tick meets the tab; a stack's corners are rounded only where no tab sits next to it (the stream's top, the last tab's bottom), the rest flat; the strip draws no line against the HUD rail; in the expanded panel each row's tab runs on across the row carrying the title, set off from its abbreviation by a divider line through every row and a gap (`IconColumn wide`, `IconCell label`), so the tab's colour flows into the label, the glyph kept at its strip width so the two line up; the panel's rows draw the same icon column (`IconColumn`) so the two line up; clicking a glyph navigates directly — a thread glyph selects that thread, a stream glyph switches streams via `onSwitchStream`. **Hovering a glyph shows its full title as a native tooltip and nothing else (tsk269).** The `navigator-overlay` panel (~280px, covering the strip *and* the rail HUD to its right) expands only on an explicit click: the bottom-pinned `navigator-expand` chevron, dead space in the strip (`navigator-strip-empty`, `e.target === e.currentTarget`), a stream glyph (which also switches to that stream, then shows its threads), or the already-selected thread's glyph (selecting it again would do nothing). The chevron is pinned *outside* the scroll container and is the load-bearing affordance — once the list scrolls there is no dead space left, and `+ Add stream` lives only in the panel. The open/close state machine — click-to-open, geometric pointer-leave, Escape, outside-press, background-click, and the passive-vs-explicit guard — lives in the shared `apps/desktop/src/components/useSlideoutStrip.ts` hook (chevron in `SlideoutChevron.tsx`), which the Terminal page's `TerminalTabStrip` also runs on; a new slide-out strip adopts the hook rather than re-deriving it. It dismisses on a click in its own **dead background** (anything resolving to `button/input/select/textarea/a/label/[role=button]` is left alone so controls behave normally), on the pointer leaving its bounds (180ms grace), on Escape, and on any pointerdown outside it. The last two are *explicit* dismissals and beat the mid-rename / mid-new-thread form guard; the first two don't. **Pointer departure is measured geometrically** (document `pointermove` vs. the panel's rect), never `mouseleave` — the panel covers the rail and sits in the same DOM subtree as the strip, so the pointer never "leaves" the wrapper while parked over the covered region, which is what used to strand it open on top of the rail and swallow clicks meant for it (tsk131). Each overlay row (`navigator-stream-row-<id>` / `navigator-thread-row-<id>`) — and each strip glyph, with the same menu — opens its action menu on **right-click** (Menu key / Shift+F10 for keyboard), headed by the stream's or thread's name (`context-menu-header`); Rename and Add thread open the panel for their inline field. **Thread menu:** **"Mark with…"** (`menu-item-thread.mark`) opens a submenu headed by the line "A colour on the thread's tab, for grouping threads your way. It changes nothing else." (`context-submenu-header`, the generic `MenuItem.submenuHeader`) and lists `amber`, `red`, `green`, `blue`, `violet` (each with a colour chip, `MenuItem.swatch`) and None as `menu-item-thread.mark.<colour|none>`, the current one checked; picking one runs `oxplow.thread.mark` (via `onMarkThread` → `App.handleMarkThread`). Then Rename / Settings / Close. A mark only groups threads the person's way; every open thread of a stream shares its worktree. |
| `apps/desktop/src/tabs/bookmarks.ts` | The person's bookmarks, project data (data-model.md "bookmark"): `useBookmarks(threadId, streamId)` reads `v_bookmark` — a page once, at the narrowest scope the thread sees (thread / stream / project) — and `setBookmark` / `removeBookmark` run `oxplow.bookmark.set` / `oxplow.bookmark.remove`. Pages bookmark via the `PageNavigationContext.bookmark` binding (one `scope`; the star's menu moves it or, on its current scope, takes it off); the Go To lens page's bookmark row actions re-scope and remove. |
| ~~`apps/desktop/src/tabs/appPageBacklinks.ts`~~ | **Deleted.** Per-kind in-memory backlinks providers used to live here. Cross-page backlinks now come from the persisted `page_ref` graph (`crates/oxplow-db/src/page_ref_store.rs`) via the `list_backlinks` IPC; every page kind goes through the same code path. App pages that need their own provider would register a new `source_kind` writer in the backend instead. |
| `apps/desktop/src/pages/WorkItemPage.tsx` | Every work item's page (`work_item:<provider>:<id>`), whichever list is active: title and body edited in place (`TaskDetail`), the rail's state pill and the list's declared fields (`TaskDetailRail`), activity (its efforts); Parent, thread/backlog moves, Comment…, Link… and Delete only as the list's features allow (`v_capability_provider`); the `work_item.detail.body` / `.sidebar` slots and the `work_item.detail.state` replacement (beside the state pill, never instead of it). See [work-items.md](./work-items.md). |
| `apps/desktop/src/pages/GitCommitPage.tsx` | Single-commit page (`commit:<sha>`): commit metadata (collapsing a long body behind "Show more") with cherry-pick / revert, the changed files (`ChangedFilesTree`, parent → commit diffs) and the `vcs.commit.details` lens slot (`change_id` from `useChange`). Routed via `gitCommitRef(sha)`. |
| `apps/desktop/src/components/RailHud/sections.ts` | The launcher's categories (`PAGE_CATEGORY_ORDER`, `PageDirectoryEntry`) and `settingsAreaEntries` (the Settings areas as launcher entries); every other launcher page is a declared page (`components/extensionLauncher.ts`). |
| `apps/desktop/src/pages/GitDashboardPage.tsx` | Committed-history rollup: branch header (current branch + upstream + ahead/behind + push), small uncommitted mini-card that links to `UncommittedChangesPage`, recent commits rendered through the shared `CommitGraphTable` (last 5, current branch only via `getGitLog({ all: false })`; click a row → reveal in `GitHistoryPage`), worktrees row with per-row "Merge into current", a **"Merge readiness" card** (cross-stream divergence vs the integration branch via `listStreamDivergences()` — per-stream ahead/behind + a clean/will-conflict/integrated badge, naming the overlapping files on conflict, with a one-click "Merge into &lt;base&gt;" offered only while viewing the base stream), recent remote branches with per-row pull/push. Each action asks first exactly when its command's spec does (`SpecConfirm`, tsk898: merge and rebase, which are `Destructive`; pull and push are `Never`, as everywhere else they run). Routed via `gitDashboardRef()`. |
| `apps/desktop/src/components/History/CommitGraphTable.tsx` | Pure presentation of the git-log graph (branch/merge dots + lines + sha + ref badges + subject + author + relative date). Used by both `HistoryPanel` (full list with detail pane; `GitHistoryPage` wraps it, with a `vcs.history.sidebar` side column when an extension mounts there) and `GitDashboardPage`'s recent-commits card. `indexRefsBySha(log)` exported alongside groups branch heads + tags by sha so callers feed identical maps. |
| `apps/desktop/src/pages/UncommittedChangesPage.tsx` | Working-tree changes: a Commit-all form, the changed files (`ChangedFilesTree` via `useChangedFiles({ kind: "working" })`, with a `N changed · +a −d` heading), the `vcs.status.header` strip above them (`stream_id`) and the `vcs.status.details` lens slot below. Distinct from `FilesPage` which is the full project file tree. Routed via `uncommittedChangesRef()`. |
| (change analysis) | No page of its own: `crates/oxplow-app/src/change_analysis.rs` stores each change behind `v_change*`, and the oxplow-bundled `change-review` lens grid shows it in the `vcs.commit.details`, `vcs.status.details` and `effort.review.details` slots. The old per-page scope drilldown is gone; scoped tab ids reopen the unscoped page. |
| ~~`apps/desktop/src/tabs/backlinksIndex.ts`~~ | **Deleted.** The in-memory cross-kind indexer is replaced by the persisted `page_ref` table; see `data-model.md`. The `BacklinkEntry` type that renderers consume now lives in `apps/desktop/src/tabs/backlinkTypes.ts`. |
| `apps/desktop/src/tabs/useBacklinks.ts` | React hook that calls the unified `list_backlinks` IPC for a `TabRef` and maps the returned `BacklinkEdge` rows into `BacklinkEntry`s. Used by every page kind including `FilePage` (which previously rendered nothing). The sibling `usePageOutbound` hook does the same for the inverse direction. |
| `apps/desktop/src/tabs/backlinkTypes.ts` | Renderer-side `BacklinkEntry` interface (`{ ref: TabRef \| null, target, label, subtitle?, dead?, linkType? }`): `ref` is the page the row opens, null for a dead link (nothing resolves the ref; `BacklinkEdge.resolved`) or a resolved ref with no page. Decoupled from the SQLite `BacklinkEdge` shape. |
| `apps/desktop/src/tabs/BacklinksList.tsx` | Default renderer for the Page chrome's `backlinks` slot — buttons that route via `onOpenPage`; a row with no page renders muted as its raw ref (dead link). `entryMenu` supplies a row's right-click actions (a work item's link rows: Unlink). |
| `apps/desktop/src/pages/WikiPage.tsx` | Single-record page for a wiki page (`wiki:<slug>`), rendered through `Page` so it gets the unified chrome (title via `usePageTitle`, back/forward + star, backlinks). Edit/Save/Revert/Delete live in a thin toolbar inside the body. In-tab wikilink clicks route through `PageNavigationContext.navigate(wikiPageRef)` so they join tab history. |
| `apps/desktop/src/pages/CommentsInboxPage.tsx` | Global **Comments Dashboard** (`comments` index kind, `commentsRef()`; titled "Comments Dashboard"). Lists every comment in the current stream (`listCommentsForStream`), grouped by target. The body of a **row opens that comment's full `CommentPopover` inline** (read/reply/intent/resolve/delete — triage the whole backlog from one place); a **group-header click jumps to the target page** (file/wiki/task). Each row also has a **"Go to location" button** that navigates to the target page *and* scrolls to / opens the anchored comment via `comment-reveal-bus.ts` (disabled for orphaned comments). **Defaults to unresolved threads only**; a "Show" dropdown reveals resolved threads bucketed by recency. The bucket thresholds are a tiered ladder — one per day to a week, one per week to a month, then one per month beyond — capped at the oldest actual resolved comment so the largest option reaches all of them (helpers in `comments-filter.ts`: `resolvedWindowOptions` / `visibleThreads`, keyed on `comment.resolved_at`). The holistic "review them all" surface; the agent reaches the same data via the `oxplow_list_comments` MCP tool. |
| `apps/desktop/src/components/Analytics/DailyBarChart.tsx` | Generic daily bar chart (`{ label, value }[]`); renders `bar` lenses. |
| `apps/desktop/src/pages/StreamSettingsPage.tsx` | Per-stream settings page (custom prompt). Replaces the in-rail StreamRail settings modal. Routed via `streamSettingsRef(streamId)`. |
| `apps/desktop/src/pages/ThreadSettingsPage.tsx` | Per-thread settings page (custom prompt). Replaces the in-rail ThreadRail settings modal. Routed via `threadSettingsRef(threadId)`. |
| `apps/desktop/src/components/CollapsibleSections.tsx` | Collapsible page sections (tsk84, tsk86). Two parts plus a hook: `CollapsibleSections` (state provider), `SectionCollapseControls` (the "Expand all" / "Collapse all" pair), and `useSectionCollapse`, which the kit `Section` (`collapsible`, `defaultCollapsed`; the toggle is a button inside the `<h2>`) reads. Sections register themselves on mount so the controls know what "all" means — only what's **currently rendered** counts, so a filtered-out section is never silently expanded — and the controls self-hide when nothing is registered. **The page places the controls**: Recorded Metrics renders them in the **details rail** beside its filters, which works because the provider wraps the whole `<Page>` and `rightRail` is *rendered* inside Page's subtree (context follows the render tree, not the creation site). For that to be safe the provider renders `children` **bare** — a wrapper element there would break the page chrome's `height: 100%` column. Collapsed ids are the tab entry's state (`useHistoryState`); default is expanded, and stored ids are deliberately **not** reconciled against the rendered set (a section hidden by a page's search must return still collapsed). Pure state/persistence in the sibling `sectionCollapse.ts`. **Not a `Page` prop** — see `.context/usability.md` → "Collapsible page sections". Adopters: `MetricsPage`. |
| `apps/desktop/src/components/Slideover.tsx` | Right-edge panel primitive (~38vw, backdrop-click + Escape close, focus-into-panel on open) for form-shaped flows that don't justify a full page. Use instead of a centered modal. |

## Page kinds

`PageKind` (`apps/desktop/src/tabs/tabState.ts`) is the source of truth;
this list mirrors it. Grouped by what they are for. Kinds marked **(→
ext)** are analytics pages slated to move into the `oxplow-bundled`
extension as `lens:<slug>` pages ([extensions.md](./extensions.md), epic
tsk275) — don't grow them; new instruments should be lenses once the
extension host lands.

- **Agent & work:** `agent_session`, `new-session`, `work_item`, `tasks`, `backlog`,
  `archived`, `new-task`, `new-stream`, `stream-settings`,
  `thread-settings`, `comments`,
  `alerts`
- **Code & review:** `file`, `dir`, `files`, `diff`, `diff-view`,
  `uncommitted-changes`, `commit`, `git-history`, `git-dashboard`,
  `local-history`, `terminal`
- **Knowledge:** `wiki`
- **Lenses:** `lens` (`lens:<extension>/<slug>`, user/agent-built from
  `.oxplow/extensions/`; see [extensions.md](./extensions.md))
- **System:** `settings`, `external-url`
- **Data (the core explorer; stays core):** `explore-data`, `metrics`,
  `metrics-recorded`, `metric`,
  `custom-dashboard`
- **Analytics (moved to ext):** oxplow-bundled lenses (`usage`,
  `planning`, `review`, `quality`, `findings`, `effort-tests`).
  `duplicate-block` stays core as the side-by-side compare page (lens
  `compare` links).

An agent session's tab follows its row (`reconcileSessionTabs`). There is no `change-analysis` kind.

## Tab id format

Built only by the helpers in `apps/desktop/src/tabs/pageRefs.ts` — never
hand-format an id.

**Every tab id is a canonical ref** ([refs.md](./refs.md)), parsed
through the shared grammar (`apps/desktop/src/refs/ref.ts`) so a `:`
inside an id (`work_item:oxplow:tsk42`) never splits it. Two families:

- **Entity pages** (`EntityPageKind`): the tab id of a file, directory,
  wiki page, task, commit, metric or lens *is* its ref, so a `[[…]]`
  link, a `page_ref` row, a backlink and a tab all agree on one string.
- **Declared pages** (kind `page`): a page an extension's manifest
  declares under `pages:` — not a thing in the graph. Its id is
  `page:<id>[?params]` for oxplow's own (oxplow-foundation has
  `namespace: oxplow`, so its pages are core-named as its commands are:
  `page:tasks`, `page:settings?area=agents`) and
  `page:<extension>/<id>[?params]` for any other extension's. Every
  page tab's `TabRef.kind` is `page`; its payload is `PagePayload
  { extension, page, params }` (`extension` null for oxplow's). Nothing
  rides beside the id: what a page shows is its params (a diff's spec,
  Settings' landing section, a new item's parent), and what only one step
  of a tab keeps (a clipboard compare's two texts) is that entry's history
  state. Pages never appear
  in `page_ref`. Params are a query string whose values escape only
  what would break the id (`&`, `=`, `+`, `%`, `@`, `#`, whitespace),
  so `path=src/a.ts&left=git:abc` stays readable.

`refFromTabId(id)` rebuilds the full ref (with payload) from an id alone
and returns `null` for text that isn't a ref or an unknown kind —
callers (rail History, Go To, the launcher's Recent) drop the row rather
than open a blank tab. Any `page:` id rebuilds (`pageTabRef` /
`extPageRef`); whether a page by that id exists is the declaration's
business, and an undeclared one draws "No such page". The round-trip
test in `pageRefs.test.ts` covers every constructor (a page's `state`
aside). constructor. `newTaskRef(payload)`'s defaults deliberately aren't in the
id (stable id → one form tab), so a history reopen starts empty.

There are no sentinel tab ids (`HOME_TAB` in `App.tsx` means "nothing
chosen", never a tab), and "is this the editor's working-tree file" is
`diskFilePath(id)` (null for a pinned revision such as
`file:src/a.rs@git:HEAD`, which is a read-only viewer tab).

**No compatibility layer for old ids.** When a kind is renamed or a page
moves into an extension, its saved tabs/bookmarks/history are dropped
(the tab-layout keys started fresh again as `oxplow.layout.v3.*` when
pages became declared, 2026-10-17; bookmarks
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
| page | `page:<id>[?params]` (oxplow's) / `page:<extension>/<id>[?params]` (`pageTabRef(id, params?, state?)`, `extPageRef(ext, id, params?)`, plus typed wrappers: `diffRef`, `endpointDiffRef`, `duplicateBlockRef`, `settingsRef`, `newTaskRef`, …) — a declared page; see "Declared pages" below | `page:tasks`, `page:symbols?path=src/lib.rs`, `page:github/open-prs`, `page:acme/pr?ref=acme_pr:12` |
| symbol | `symbol:<path>/<id>` (`v_symbol.ref`, `symbolRef(ref)`) — no revision in the id; never a tab: opening one (`handleOpenPage`, or in-tab navigation) resolves it (`resolve_symbol`) and opens its file at the name's line (`openSymbol`). A pinned one (`@snap:<id>` / `@git:<sha>`) opens the file viewer at that revision; if the version is gone it shows a notice and opens the current symbol | `symbol:src/lib.rs/Widget::spin` |
| agent_session | `agent_session:<ses id>` (`agentSessionRef(id)`, pinned) — an open agent session's terminal; its tab draws its harness's declared logo (`CenterTab.icon` from `HarnessListing.icon`, `AgentIcon`; the generic robot when it declares none) | `agent_session:ses3` |

## Declared pages

Every page that isn't an entity's is a `pages:` entry in an extension's
manifest (`.context/extensions.md` "Pages"); oxplow's own are in
oxplow-foundation's. The declaration gives the title, icon, launcher
`category` (none: not in the launcher — a page only opened from
elsewhere), `keywords`, the `params` its id carries, a `count:` lens for
the launcher badge, and what draws it: a `lens:` (most pages) or, for
oxplow's own until the kit can draw them, `component: oxplow:<name>`, a
**built-in page**. The built-ins are pinned by
`crates/oxplow-app/src/built_in_pages.rs` and its golden
`fixtures/built_in_pages.json`, which only shrinks (a page leaves when it
becomes a lens); `pages/declaredPages.ts`'s `BUILT_IN_PAGES` mirrors it
and a test checks they match.

The desktop draws a `page` tab in one renderer (`App.tsx`,
`pageRenderers.page`): it finds the declaration (`declaredPage`, by tab
id among the enabled extensions), draws a built-in from the
`builtInPages` table (`pages/builtInPages.tsx`) by name, a lens page through
`DeclaredPageView` (`LensPage` with the id's params), and a custom page
(`body.kind === "component"`) through `CustomPageView`: the component's
frame filling the body under the nav bar, told its stream and thread
and given the whole entry as its view (`useHostedView`), its title and
actions the frame's (`setTitle`, `setActions`). oxplow's Timeline (`page:timeline`, params `from`, `to`,
`thread`) is one: its window is its view params, written on every change
(not its opening default) and followed on Back/Forward. A frame's
`setViewParams` reaches the tab as `patchViewParams` (null drops a key). Each built-in body is a
`(ctx) => CenterTab | null` over `{ ref, params, state, nav, scope }`;
`scope` is the `AppScope` (`pages/appScope.ts`), the explicit list of
what core offers them (selection, work state, handlers) that `App.tsx`
builds per render, so a body never reaches into App's locals. While the
extensions load, an oxplow page whose id is a built-in's name draws the
built-in straight away. Icons and labels come from the declaration too:
`kindForTabId` gives a page's id (`tasks`, `gh/open-prs`) as its kind,
which is also what visits and bookmarks record (`recordedPageKind`),
and `PageKindIcon` / `lensIcon` / `pageKindLabel` resolve it to the
page's icon and title. The launcher lists the declared pages with a
category (`launcherPages`), each `count:` lens run when it opens
(`useLauncherCounts`), plus the Settings areas (`settingsAreaEntries`).

oxplow's pages and their params:

- Lens pages: `problems`, `symbols` (`path`, `filter`), `closed-threads`,
  `wiki-index`, `wiki-freshness` (`slug`), `go-to`, `dashboards`,
  `hook-events`, `done-work`, `catalog`.
- Built-in pages: `tasks`, `backlog` (badge: `backlog-count`), `board`,
  `files`, `git-dashboard`, `git-history`, `uncommitted-changes`,
  `local-history` (view: `mode` `recent` | `by-commit`, `all` — "View
  all" pushes it), `comments`, `explore-data`, `metrics-recorded`,
  `terminal`, `alerts`, `settings` (view: `area`, `section` — another
  area is a pushed view change, never a page opened over it),
  `custom-dashboard` (`id`), `new-session`, `new-stream`, `new-task`
  (`parent`), `stream-settings` (`stream`), `thread-settings` (`thread`),
  `external-url` (`url`), `diff` (`path`, `left`, `right`, `label`,
  `base` — the left side's name when it isn't the version's own — and
  `line`, a view param: the same diff opened at another line moves the
  tab showing it; `diffSpecOf` reads the spec back; a clipboard compare's
  texts are history state, `diffTextOf`), `diff-view` (`start`, `end`:
  `none` / `working` / `snap:<id>` / `git:<rev>`; `endpointsOf`; a
  picked endpoint is a pushed view change), `duplicate-block` (`left`, `left_lines`,
  `left_at`, `right`, `right_lines`, `right_at`; `duplicateBlockOf`).

New Session (`page:new-session`) is a Document page (`DocumentLayout`):
**Start a new session**, then **Earlier sessions**, and what extensions
mount in `new-session.sections` (ids `start`, `earlier`; it binds
`thread_id`). Earlier sessions is a kit toolbar and table (a filter and
the thread's closed sessions and saved conversations,
`list_earlier_sessions`; ↑/↓/Enter/Escape from the filter; a row
reopens, resumes or shows its session by the action the backend chose —
`terminal.md` "Earlier sessions"). A diff's `revealLine` is not in its
id, so re-clicking reuses the tab.

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
The set of extension panels (like extension pages and slot mounts)
reloads on `lensRerun.extensionsChanged`: a deployment changed (`oxplow.extension.deploy`, `rollback`, `enable`,
`disable`), or the config did — enabling or disabling
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

## Page archetypes and the kit

Core owns the look; a page, core or extension, fits one of three shapes
and draws it with the **component kit** (`apps/desktop/src/kit/`, one
export point `kit/index.ts`). The kit's parts render the classes of one
stylesheet, `crates/oxplow-daemon/assets/oxplow-kit.css`, which the app
imports (`main.tsx`) and custom components link, so a core page, a lens
and a sandboxed component look alike. A page composes the kit; a new local
section, table, filter list or "Loading…" line is a review finding.
Settings, Stream and Thread settings, Alerts, Go To and the wiki page list
draw their parts with the kit `Section` (no local copies): a page's action
is a `PageAction` on its section (Alerts' "Clear failed operations" is
`action-clear-ops`), and the Settings index scrolls to a section by its
`data-section` id.
Core tables are `DataTable` too (AI usage, Data entities, the Metrics list
and a metric's recordings, a model's documented columns). A row built in a
loop that opens a page takes its click from `routeDispatch(ref, ctxNav,
options)` + `routeHandlers(dispatch)` (`tabs/RouteLink.tsx`, the same
dispatch `useRouteDispatch` memoizes), not a hook per row.
Every list driven by ↑/↓/Enter/Escape uses `useListNav` (usability.md
"Keyboard"): the conversation picker, the dashboard's metric picker, the
Plan keyboard value picker and the launcher. The launcher adds Tab (next
section), Home and End to it and leaves out the pointer-moves-the-cursor
part (it scrolls the cursor's row into view, which a hovering pointer would
fight). The New Session page's earlier-sessions list is the one remaining
copy.
A time said as "how long ago" is `formatRelative` (`components/format.ts`;
`formatRelativeSeconds` for a unix time): the launcher, the wiki list, a
comment's time and the task panel use it, not their own copies.

| Archetype | When | Core form | Extension form |
|---|---|---|---|
| **Document** (the default) | reading, picking, small forms, lists | `DocumentLayout`: title + intro in a reading column, then `Section`s (a heading over a rule, no boxes) | `viz: document` + `sections:` |
| **Dashboard** | many numbers and charts at a glance | `TileGrid` / `TileSlot` / `Tile` at full width | `viz: dashboard` + `tiles:` |
| **Custom** | what the kit can't express: editors, terminals, boards, git flows | a React page on `Page`, using kit parts where they fit | `viz: custom` (the sandboxed component), or a whole custom page: `pages[].component: <id>` (`CustomPageView`) |

A cell's match is drawn with the kit class `.ox-mark` (a lens column's `highlight`, extensions.md "Lenses").

The kit: `Section` (`id`, `title`, `description`, `actions`, `collapsible` / `defaultCollapsed` — a toggle in the heading, remembered per page, see usability.md "Collapsible page sections" — `source` —
the extension that added it), `DocumentLayout` (its own sections plus what
extensions mount in its parts slot, placed by `placeParts`; a contents list
in the rail from four sections), `DataTable` (fixed `<col>` widths,
`numeric` and `truncate` columns, groups, per-row props — the lens `table`
viz draws through it), `Toolbar` (a core list's filter and actions; a
lens page has none; `useSearchFilter(key, …)` builds its `filter` on the `key` view
param) with `useListNav`
(the one ↑/↓/Enter/Escape contract, usability.md "Keyboard"; a list that
holds focus itself spreads its `keyProps`), `Stat` (a figure; `big` when
it is its place's whole — the lens `number` viz), `Facts` (labels beside
values — the lens `detail` viz, a metric's definition),
`MetricFilters` (the range, branch and slice of a metric page as `ParamRow`s),
`TileGrid` / `TileSlot` / `Tile`, `Loading`, `ActionButtons`, and
`EmptyState` (re-exported; it lives beside the agent prompts it offers).
Lengths come from the spacing and radius tokens (theming.md).

**Extensions add to a page through its parts slot** (`<page>.sections`,
`<extension>/<slug>.sections` of a document lens, `.tiles` of a dashboard),
placed `at` the start, the end, or beside a part by its stable id —
extensions.md "Slots". A core Document page lists its section ids with its
slot in `SLOTS` and passes the slot to `DocumentLayout`.

## Body layouts

`<Page>` takes two independent props: `width?: "full" | "reading"`
(default `"full"`) and `rightRail?: ReactNode`. The tab-level chrome
(nav bar, header, legacy backlinks footer) is the same either way — only
the body region differs.

- **Full, no rail** — the body region is `flex: 1; overflow: auto` with
  no width cap and no padding; children own their own padding. Lists,
  history tables, the agent terminal, file/diff editors, dashboards and
  anything that wants every available pixel.
- **Reading** — a padded (`24px`) body whose center column carries the
  `.oxplow-reading-column` class (defined in `index.html`), which owns the
  measure: `max-width: 78ch; margin-inline: auto`. The layout owns the
  width so **children just fill it and don't manage their own** — that
  class also cancels the per-leaf self-cap on `.oxplow-md` /
  `.oxplow-rt-field` (which otherwise self-center at `78ch` for
  unbounded contexts), so the title, description, and activity share one
  left edge and width. Reuse `.oxplow-reading-column` for any bounded
  reading area.
- **The rail** is the page's control panel (params, filters, collapse
  controls, a contents list): a `320px` sticky column beside a body of
  **either** width (the body is padded then), titled by `rightRailTitle`
  ("Details" by default). A lens page's rail is `LensRail` — Filters,
  Selection, About this page, built from `RailGroup` and `ParamList`
  (usability.md "A page's controls are in its rail").
  It is orthogonal to the archetype — a dashboard may have one for its
  filters, Git History has one only while an extension mounts something
  in `vcs.history.sidebar`.

The rail is **purely responsive — no user toggle**. A `ResizeObserver`
on the body watches its width; below `960px` the same panel moves to the
top of the center column (`RailBody`). A page with no rail content for
some condition passes `rightRail={undefined}`.

**Which width:** detail pages (single record — wiki page, task, …) and
Document pages use `width="reading"`; lists, dashboards and editors stay
full.

## The nav bar is core's

The bar above every page (`tabs/PageNavBar.tsx`: back/forward, siblings,
title, bookmark, backlinks, comments, Ask, the extensions' Commands menu,
Pin to Dashboard, the page's actions) is laid out and drawn only by core.
A page says **what** goes in it, as data:

- `title` / `usePageTitle`, `chips: PageChip[]` (a chip with `onClick`
  is pressable — a wiki page's freshness opens its report);
- `actions: PageAction[]` (`kit/actions.ts`: `id`, `label`, `run`,
  `primary`, `disabled`, `busy`, `title`), drawn by `ActionButtons` in the
  bar — or in the header when a page shows no bar — never in the rail;
  each button's test id is `action-<id>`;
- `pin: { tile, onOpenPage }`: what the page would add to a dashboard —
  the bar's Pin to Dashboard menu (`page-pin`). A lens page pins its lens.

**A pinned tab has no history:** it leads the strip and is always itself,
so its bar leaves out back and forward (`PageNavigation.pinned`, set from
the tab's `pinned` in `App.tsx`; `PageNavBar history={false}`). Today only
an agent session's tab is pinned.

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
   The launcher's page list is the declared pages with a `category`
   (`launcherPages`; Work / Code / Git / Activity / Knowledge / Data /
   System) plus the Settings areas, so the launcher empty state reads
   like a start menu. The ↗ opens the **Go To** page
   (`goToRef()`, `page:go-to`) for the full hub + bookmark management.
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
  - **Recent** = the 10 most recently visited pages for the selected thread
    (`listRecentPageVisits({ dedupeByRef, excludeKinds:
    RAIL_HISTORY_EXCLUDE_KINDS })`, the same source as the rail History
    block), kept live via `subscribePageVisitEvents`. Visit rows don't
    persist the ref payload, so `buildRecentEntries` rebuilds each openable
    ref with `refFromTabId` and `recent:`-prefixes the id to avoid colliding
    with a static directory page. `LauncherSection = PageCategory |
    "Recent"`; recent rows are the lighter `LauncherPageEntry` (no static
    category). The section only renders when there are visits, and self-hides
    while searching (recent entries never enter `buildQuickOpenResults`).
  - **Persistence:** static category expansion is an app preference
    (`quickOpen.expandedCategories`); Recent's collapse is another
    (`quickOpen.recentCollapsed`, default un-collapsed) so
    its default-open state doesn't disturb the category choice. The
    effective expanded set is `expandedCategories ∪ {Recent unless
    collapsed}`.
- **Typing** → ranked results, pages → **commands** → files → body hits.
  Commands are the bus's offers (`commandOffers`, passed in as `offers`)
  that can run now (`searchableCommands`); the same offers feed the menu
  bar (`menuBar.ts`). Body hits come from `searchSite` (BM25 over every
  searchable kind and file contents: work items with their comments,
  wiki pages, comments, thread notes, threads, agent sessions, turns,
  efforts, decisions, commits, lenses, dashboards; the thread-shaped kinds
  include archived work); each row shows its kind's icon and name
  (`pageKindIconComponent` / `pageKindLabel`); a hit is a ref, opened through the
  ref opener (`openSearchHit` → `openRef` → `tabs/refOpener.ts`),
  which may select a thread or switch stream. The ordering is a pure helper —
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
- **Done work** (`page:done-work`, a foundation lens page) —
  full descending list of the current thread's closed items (done and canceled).
- **Backlog** (`page-backlog`) — the active list's items on no thread
  (`v_work_item.thread_id IS NULL`), for a list with the `lists`
  feature; `oxplow.work_item.move` takes one to a thread and back. The
  `backlogReadyCount` badge in the rail directory hangs off this entry.

Tasks and Backlog read the work-item interface (`workItems.readWorkList`) and
wrap `PlanPane`, which takes the active list's profile
(`useWorkListProfile`: its features and declared fields) and offers only
what the list can do: drag reordering with `ordering`, the backlog with
`lists`, epics with `hierarchy`, Delete with `delete`. Its rows show the
list's editable enum fields (oxplow's priority), and the Tasks page's
filter bar a chip row per declared enum field. They pass filter props
(`visibleSections`, `sectionItemLimit`, `onlyStates`, `excludeStates`,
`extraSectionLinks`, `forceMode`). There's no
Archived page: oxplow's archived tasks are done or canceled, as the
interface states them.

The four pages reuse the shared `<Card>` + `cardLinkButton` from
`apps/desktop/src/components/Card.tsx` for cross-page "View X →" affordances;
GitDashboardPage uses the same shell so the dashboard vocabulary is
consistent across IA.

Named ref helpers — `tasksRef()`, `doneWorkRef()`,
`backlogRef()`, `archivedRef()` — mirror the GitDashboard pattern
(`gitDashboardRef`, `uncommittedChangesRef`).

## Navigation model: a tab is a session of entries

A thread's center tabs are one pure model, `tabs/session.ts`, as a
browser keeps its session history. App holds a `Session` per thread
(`useTabSessions`) and draws everything tab-shaped from it: the strip,
the bodies, Back/Forward, Up/Down, the star, Ask, what agents read.

| Browser | oxplow |
|---|---|
| a tab's session history | `Tab { slot, entries, at, pinned? }` — `slot` a stable name (`t3`), never an id |
| URL = path + query | an entry's `ref`: identity (kind + id) and view (its payload's `params`; `kind:id?k=v` as a string) |
| `history.state` | `Entry.historyState` (persisted when it is JSON) |
| the page you came from a list of | `Entry.from: Origin { title, items: [{ ref, label }], index }` |
| pushState / replaceState | `navigate(slot, entry, { replace })`, `setView(slot, ref, { push })` |
| back-forward cache | `bodies(tab, pageIdentity)`: hidden bodies stay mounted |
| a SPA re-rendering on popstate | **every page is live**: one body per page identity, keyed `slot/identity`, so entries that differ only in view params re-render it — no remount, no Loading |

**Operations** (pure, tested in `session.test.ts`): `open` (the tab
showing that exact ref, view params included, else a new tab; active.
A decision, not an accident: opening Board while a tab shows
`page:board?show=all` opens a second tab, because the views differ. The
one exception is a diff, opened with `identity: pageIdentity`: the same
diff at another line moves the tab showing it), `navigate` (push; clears forward;
`replace` takes the current entry's place; a page shown in another tab
closes there), `setView` (a page changes its own view), `setHistoryState`, `patchHistoryState`,
`back`, `forward`, `stepOrigin` (Up/Down in place, history untouched),
`close` (the most recent other tab becomes active, as a browser does),
`dropCurrent` (the page's record was deleted: back, dropping the entry,
else close), `setStrip` (the strip as a list of refs: agent-session
reconcile, a new thread's tabs, an agent's `oxplow.tab.*` in another
thread), `reorder`, `activate`, `evictions` (`MAX_PAGE_TABS` = 15;
never the active, a pinned or a dirty file's tab).

**A page changes its view params through `PageNavigation.setViewParams(params, { push })`**
(the whole set, defaults left out; the host writes them into the page's
own kind of ref, `withViewParams`, and reads them back as
`PageNavigation.viewParams`, `viewParamsOf`).
A filter replaces the entry (typing a search adds nothing to Back); a
selection — a param a column `select`s — pushes one, so Back un-selects
and Forward re-selects. Core decides by the param's role; no page
chooses. Only the body drawing the tab's current entry gets
`setViewParams`. `PageNavigation.ref` is the entry's ref the body draws.

**Names say the scope**, the same in core, `PageNavigation`, the
component bridge, the docs and the skill: **view params** (this tab's
address), **history state** (this step of this tab's history), **page
memory** (this page, for this person, every tab) and **app preference**
(the whole app, for this person).

**Origins.** A list passes an `origin: Origin` (`{ title?, items:
[{ ref, label }], index }`) to `useRouteDispatch` / `navigate`; the host
stores it as the entry's `from` (`originOf`: the index snapped to the
destination's id), and the nav bar's Up/Down and "N of M" read it back
through `PageNavigation.origin` and `stepOrigin(index)` (the nav bar's
test ids stay `page-nav-sibling-*`). A diff item's ref is `diffRef(spec)`,
which carries its spec. New-tab paths drop it. Back/Forward restore each
entry's own origin. Adopted lists: `BacklinksList`, `FileTree` (file rows
of one directory), `CommitList` / `CommitGraphTable`
(`CommitRowDispatcher`), `MetricsPage` (`metricOrigin`, flattened across
sections), the Local History dashboard. `WorkGroupList` rows open an edit
modal, not a page, so they don't adopt.

**Keys.** The global handler in `App.tsx` (`tabs/navigationKeys.ts`):
Cmd+[ and Cmd+] are Back and Forward of the active tab, Alt+ArrowUp and
Alt+ArrowDown step its origin (`stepOrigin`, history untouched). None
fires while focus is in an input, textarea, select, contenteditable or
Monaco — those keys are the field's.

**What is keyed by what.** Active, MRU and eviction by slot; the strip,
`center-tab-<id>` / `center-tab-body-<id>` test ids, titles
(`pageTitles`), bookmarks and the tab report by the current ref id;
visits by the page's **identity** (`identityOf`: the id without its
`?…`) — a filter or a pick isn't a new visit, so Recents lists the page
once. A bookmark is of the values: change a filter and the star goes
off; set it back and it's on.

**Persistence.** One key, `oxplow.layout.v4.sessions` (`{ [thread]:
Session }`, written on every change, read on start; corrupt text reads
as none). It holds every tab's history and the active tab, so a restart
comes back to the same pages at the same params. A diff's spec rides in
its ref (`diffRef`'s state), so diffs survive too. The open files'
paths per stream stay in `oxplow.layout.v2.fileSessions`
(`fileSessionPersistence.ts`): fileSessions is the editor's document
model (content, drafts), not navigation.

## What agents see of the tabs

Each window reports what it shows (`report_tabs`, debounced 300 ms from
`App.tsx`, built by the pure `tabs/tabReport.ts` from the sessions):
every thread's tabs with their shown titles (the tab strip's label, else
a page's own title, else its kind's label), each tab's `back` and
`forward` (the ids of its entries before and after, oldest first) and
`origin` (the title of the collection it was opened from), the one
selected in each, the thread it shows,
whether it is focused and visible, and the detail the shown page
publishes through `tabs/openPageDetail.ts` (a lens's `{lensId, params}`,
an editor's `{path, caret, selection}`). A window is its renderer's
`getUiClientId()`. The backend keeps the reports in memory
(`oxplow_app::client_reports`): a window not heard from for 5 minutes is
forgotten, a report older than a minute is listed as stale.

- `oxplow_list_tabs { scope?, thread? }` lists the tabs of a thread
  (default the caller's), its stream's threads or every thread, in every
  window: id (a ref), kind, title, thread, stream, agent session for a
  session tab, `selected` (in its thread, in that window), `visible`
  (selected, in the thread its window shows, in a visible window),
  window and staleness; `unreported` names the threads no window reported.
- `oxplow_get_open_page { thread_id, tab? }` is the page selected in the
  thread in the window the person was in last (most recently focused),
  with `others` (each other window's), or the listed `tab`: its
  `pageId`, its `page` (identity) and `params` (the view, decoded —
  `refs::build::view_of`; an agent never parses an id), its tab's
  `back`, `forward` and `origin`, and, for a lens, the lens re-run as
  text. Detail comes only from a window showing that thread's page. Two windows never
  collapse into one "open page".

Both are read-only: nothing an agent reads activates a tab or a window.

## Linking between tabs — single chokepoint rule

**What a ref opens is the registry's; how is the click's.** A kind's
`v_ref_kind.page` template says where its refs open, for core and
extension kinds alike, and `tabs/refOpener.ts` (`refTarget`;
`refTargetNow` when no row is needed) is the one function that fills
it: a tab, a thread or stream to select, or nothing ([refs.md](./refs.md)
"Where a ref opens"). **A link to a ref string opens it at the click**
through `tabs/openRef.ts`: `openRef(ref, how, host)` resolves it with
`refTarget` and does what it says through the window's `RefHost` — a tab
(in the page's tab or a new one), a thread to select, a stream to show,
or an error naming why not. `PageNavigation.openRef` is that for a link
inside a page (`App` builds it per tab); outside a page (the rail, the
palette) it is the window's opener, `openRefInWindow`. Wikilinks
(`linkRef`), lens links, timeline entries, backlink rows, a component's
`navigate`, search hits and `oxplow_navigate` all take this path.
`refFromTabId` parses a **tab id** (history, bookmarks, a restored tab)
and returns null for a ref of any other kind. Never add a per-surface
switch on a ref's kind to decide where it opens, and never build a
`TabRef` from a ref of an arbitrary kind: hand the string to
`<RouteLink to={ref}>` or `openRef`.

**Browser-tab semantics are non-negotiable: plain-click navigates
in-tab; only Cmd/Ctrl-click + middle-click + right-click open a new
tab.** This rule has regressed several times — every regression has
the same root cause: a list/tree row called `onOpenFile` /
`onOpenPage` directly instead of dispatching through the page-
context chokepoint, and the host's callback always opens a new tab.

**Any clickable row that targets another page (a `TabRef`, or a ref
string — `to` takes both) MUST go through `RouteLink` or
`useRouteDispatch`.** Don't write raw
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
- `apps/desktop/src/tabs/BacklinksList.tsx` — rows open a ref string
  through `ctxNav.openRef` with per-event new-tab branches (outside a page,
  the window's opener). `useRouteDispatch` is preferred for new code.

If you're adding a new index/list page and find yourself threading an
`onOpenPage` / `onOpenFile` / `onOpenNote` callback all the way down
to a row's `onClick`, **stop and use the hook instead**. The callback
should only survive as the rail-side fallback.

## Live by default: a page is a function of its entry

A tab keeps **one mounted body per page identity** (`bodies(tab,
pageIdentity)`), so a change of view params — a filter, a pick, Back or
Forward between them — re-renders the page and never mounts it again: its
state, its scroll and its loaded data stay. A page therefore reads its
view from its props or `PageNavigation.viewParams` on every render; a
`useState(fromProp)` seeded once is a bug.

**A page's identity is its ref without its view params**
(`pages/pageIdentity.ts`), which also keys visits (a filter isn't a new
visit) and page memory:
- a lens: all its params are view;
- an extension's page: all but `ref` (what it was opened for);
- one of oxplow's own pages: the view params it declares in
  `BUILT_IN_VIEW` (Tasks and Backlog: `isTasksViewParam`; Metrics: range, branch,
  slice, q, show; a custom dashboard: range, branch — `pages/metricsView.ts`;
  the searches — `pages/searchViews.ts`: Git History `q`/`author`/`branch`,
  Comments `days`, Settings `q`, Explore Data `model` (a pick, so it
  pushes), `sql` (the query that ran), `viz`, `chart`, `template`, `raw`; the
  SQL being edited and not yet run is history state `sql`; Files `filter`,
  which files the tree lists, Alerts `open`, a pick, and Board `show` (it follows
  the thread until picked) — `pages/modeViews.ts`); New Session's agent choice
  is page memory;
  the same table names an **entity kind** whose page has a view: a metric's
  page is `metric:<key>?range=…&branch=…&slice=…`, its key the identity (so
  comments, backlinks and visits still anchor on `metric:<key>`). A kind not
  in the table is all identity, and `withView` refuses a view change to it.
  The ref grammar needs nothing for it: `?` and `=` are legal in an id, and
  the Rust `metric` kind accepts the query as `lens` and `page` do; a page that
  declares none is all identity — a diff's path and sides, a dashboard's
  id — so two diffs in a tab are two pages.

## Page state: four homes, one name each

Every piece of state a page keeps has one home, and its name says its
scope — the same name in core, `PageNavigation`, the component bridge,
the docs and the skill:

| Name | Scope | Home | API |
|---|---|---|---|
| **view params** | this tab's address: shareable, bookmarkable, agents see it; a filter replaces the entry, a pick pushes one | the entry's ref | `nav.viewParams`, `nav.setViewParams(params, { push })` |
| **history state** | this step of this tab's history: scroll, open sections, a draft | `Entry.historyState` | `useHistoryState(key, initial)` |
| **page memory** | this page, for the person, every tab of it, across restarts | `ui_preference`, scoped by the page's ref without params (`page:tasks`, `lens:x/y`) or `kind:<kind>` | `usePageMemory(key, initial)` (scope from the page) |
| **app preference** | the whole app, for the person | `ui_preference`, scope `app` | `useAppPreference(key, initial)` |
| (a panel's memory) | a left-nav panel, for the person | `ui_preference`, the panel's ref | `usePanelMemory(panel, key, initial)` |
| **extension state** | shared with the team, read by scripts | the extension's state | commands |

`prefs/remembered.ts` holds page memory and app preferences: one store
loads everything once (`get_ui_preferences`), a change shows at once and
is written after a pause per key (`set_ui_preference`), a value equal to
what's there writes nothing; tests give it a backend
(`RememberedContext`). **Nothing touches the browser's storage but
`shellStorage.ts`**, which holds only the shell's own keys (the tab
sessions, the open files' paths, the remote daemon and its token, the
recent remotes, the window's log id); `no-direct-storage.test.ts` holds
every other file to it.

**One hook per kind, as easy as `useState`** (`tabs/useViewParam.ts`);
outside a tab each is the component's own state:

| Hook | Kind | Writes |
|---|---|---|
| `useViewParam(key, initial)` | a filter (a range, a search, a mode) | replaces the entry; typed back by `initial`'s type; dropped at `initial` |
| `useViewSelection(key)` | a pick (a row, a commit) | pushes when set (Back un-selects), replaces when cleared |
| `useViewList(key)` | several values (authors, states) | comma-joined, as a filter |
| `useCollapsedSet(key, initial?)` | open/closed groups (`ids`, `has`, `toggle`, `set`) | history state |
| `useDraft(key, initial)` | an unsaved form | history state; `clear` on submit |

A filter inside a part that many pages host (the changed-files tree's
search, `HierarchyView`) is history state, not a view param: it is a small
narrowing inside the page, and the commit and work-item pages are entity
kinds with no view. New Session's `q` is a view param because it is a
built-in page whose list is the page.

They write through `nav.patchViewParams(patch, { push })`, which merges
into the tab's *current* entry inside the session update (`undefined`
drops a key), so two changes in one tick both land. Every write goes
through `withView` (`pages/pageIdentity.ts`): a key that would change the
page's identity — one a built-in page didn't declare in `BUILT_IN_VIEW` —
is refused as an op error and the tab stays, rather than silently
remounting the page. A list's cursor (↑/↓ before a pick) is the list's
own (`useListNav`); only a pick is a selection, and `useListNav`'s
`selected` puts the cursor on a kept pick.

**A component inside a page shares the page's view only where its host
says which keys** (`lens/hostedView.ts`; `.context/extensions.md` "Whose
view a component has"): a replacement the keys its target's contract
lists (the history graph: Git History's `q`, `author`, `branch`, the
same ones core's commit list reads), a lens page's component the lens's
params. Its history state is its own object in the entry
(`component:<scope>`). A tile or a slot gives none.

**A page never navigates to itself, and a mode is a param, not a page
name.** Settings' areas, Diff View's endpoints and Local History's
grouping and "View all" are view changes on one page (pushed when Back
should return), so the page stays mounted and its other state stays.

Core pages follow the same rule: the Tasks page's scope and filter chips
are its view params (`components/Plan/tasksView.ts`: `scope` with
`stream`/`thread`, one `f.<field>` per filtered field), so two Tasks tabs
can differ, a scope or chip change replaces the entry, and a reload or a
bookmark keeps it.

## Every tab holds a Page

Every tab is a session tab, whatever its kind (`wiki`, `file`, a
`page:diff`, `work_item`, `lens`, `commit`, an agent session, …). The
builder in `App.tsx` (`centerTabs`) takes each tab's `bodies`, draws
each with **`pageRenderers: Record<PageKind, (ref, nav) => CenterTab |
null>`** — one entry per kind, so a kind without a renderer is a
compile error — and wraps it in a `PageNavigationContext`. `nav` is the
slot's (`navOpen`, `navOpenFile`, `navOpenDiff`, `navRevealCommit`), so
a hidden back page that navigates moves its tab as the visible one
would. `handleOpenPage` special-cases `agent_session` (activate its tab,
else its transcript), `symbol` (its file at its line), the working
tree's `file` (its buffer) and a `page:diff` carrying a spec; every
other kind is `open`.

- `fileSessions[stream.id]` is the editor's content and dirty-state
  cache, shared by a stream's threads; the tab is the session's.
  `closePageTab` drops the buffer with the tab (not refcounted across
  threads of a stream — a known gap).
- Agent sessions' tabs are **pinned** session tabs (`TabRef.pinned`):
  they lead the strip, are never dragged or dropped on, the bulk closes
  skip them, and the cap never counts or evicts them; `reconcileSessionTabs`
  keeps them in step with the open `agent_session` rows (`setStrip`).
  Their nav bar has no Back/Forward. Their × runs
  `oxplow.agent_session.close` with an Undo toast; the right-click menu
  adds Rename…. When no tab is active, a thread shows its home tab — its
  first session's, else the picker (`homeTabOf`, `HOME_TAB`).
- `closeOrGoBackPageTab` (a deleted wiki page or task) is `dropCurrent`.

The rule for new pages: a page is a `pages:` declaration (see "Adding a
page"); a new entity kind adds a `PageKind`, a `pageRefs.ts`
constructor with its `refFromTabId` inverse, and its `pageRenderers`
entry. **Don't** add a parallel tab track or keep tab state outside the
session.

**History state is `Entry.historyState`, through `useHistoryState(key, initial)`**
(`tabs/useHistoryState.ts`): one JSON value under `key` in the entry's
history state, as a browser's `history.state`. It comes back with Back
and Forward and across a restart, each entry its own. The host gives the
body `PageNavigation.historyState` and, to the tab's current page only,
`setHistoryState(patch)` (`patchHistoryState` in the model, which ignores
a write that changes nothing). Outside a tab it is local state. Used by the editor
and the diff pane (Monaco's view state, `useMonacoViewState`), a wiki page (scroll, and its unsaved edit: `editing`, `draft`), the collapsed sections
(`CollapsibleSections`), a lens tree's closed branches and open diff, a commit's expanded message, an answer's shown body and a lens's "More" params and the rows it picked (`picked`); and the unsaved forms (`useDraft`): New item,
New stream, the stream and thread prompt, the Settings-area forms (Agents, Integrations, Language servers; a secret such as an API key is never kept), the changed-files search, the
uncommitted commit message and a task's title. Keep values JSON: one
value that isn't drops the entry's whole state from storage.

## Adding a page

A page is a declaration, not core code:

1. Add a `pages:` entry to the extension's manifest (oxplow's own:
   oxplow-foundation's) with its `id`, `title`, `icon`, `category` (or
   none, if it's only opened from elsewhere), `params` and a `lens:`
   that draws it. Nothing in the desktop changes: the launcher, icons,
   labels, tab titles, visits and bookmarks all read the declaration.
2. Open it with `pageTabRef(id, params)` (oxplow's) or
   `extPageRef(ext, id, params)`; a typed wrapper in `pageRefs.ts` is
   worth it only when a payload is typed.
3. Where the kit can't draw it yet, grow the kit. A new built-in page
   (`component: oxplow:<name>`) is not an option: the golden refuses an
   added name.

A new **entity** kind (a thing in the graph whose tab id is its ref):

1. Extend `EntityPageKind` in `tabs/tabState.ts`. Decide whether its tab
   is `pinned` (leads the strip, never evicted, not dragged) — today only
   an agent session's is.
2. Add a `pageRefs.ts` helper (`fooRef(...)`), its `refFromTabId` case,
   and document the id format in the table above.
3. Build a `*Page` component in `apps/desktop/src/pages/` that wraps the
   body in `<Page>` and (typically) registers a title via
   `usePageTitle`.
4. Add its `pageRenderers` entry in `App.tsx` (the `Record<PageKind, …>`
   tells you it's missing) and its icon in `pageKinds.tsx`.
5. Plumb any list rows that target the new kind through
   `useRouteDispatch` / `RouteLink` so plain-click navigates in-tab.
6. If the page exposes data that other pages should backlink to,
   register a provider in `appPageBacklinks.ts`.

Do **not** introduce a parallel state slot for the new kind's tabs
(no `fooTabs` array, no look-aside registry): what a page needs rides
in its ref (its payload) or its entry's `historyState`, and its tab is a
session tab (`tabs/session.ts`).

## When to update this doc

- Add an entity page kind: extend `EntityPageKind`, add a `pageRefs.ts`
  helper, document the id format here. A declared page needs no change
  here unless it's oxplow's own (list it under "Declared pages").
- Add a new rail HUD section: document the data source and trigger
  conditions.
- Replace a legacy panel with a Page-wrapped renderer: tick the
  migration status row above and link the new page module.
