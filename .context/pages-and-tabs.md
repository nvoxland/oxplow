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
  (the `new-session` page, a lens), `none` opens nothing, and an
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
| `apps/desktop/src/tabs/pageRefs.ts` | Tab ids. A `TabRef` is `{ id, pinned? }`: the id is the URI the tab shows (a web page's is its URL), and there is no `kind` and no `payload`. A tab's content is read from its URI (`typeOf`, `paramOf`, `parseUri`, `fileOf(ref)`, `diffViewOf(ref)`, `resourceParamsOf(ref)`). Constructors for things are the URI builders in `uri/` (`fileRef`, `wikiPageRef`, …); route constructors are `pageTabRef(id, params?, state?)`, `extPageRef`, typed wrappers such as `newSessionRef`, `diffRef(spec)`, `endpointDiffRef`, `settingsRef`; `pagePayload(ref)` reads a route, `isPage`, `recordedPageKind`. `refFromTabId(id)` accepts every oxplow URI and every allowlisted http(s) URL, else null. `diskFilePath(id)` is the one way to ask "which working-tree file is this tab". |
| `apps/desktop/src/tabs/Page.tsx` | Shared page chrome: title + kind chip + status chips + actions slot, optional **browser-style nav bar** (back/forward + bookmark + backlinks dropdown — auto-mounted from `PageNavigationContext` when present), body, collapsible legacy Backlinks region. Title can be passed as a `title` prop or registered programmatically by the page via `usePageTitle`; the chrome falls back to the context title when `title` is omitted. Every page shows the nav bar when it has a navigation context (a page outside one — a test — gets the plain header instead); a page doesn't opt out. `titleIcon` draws before the title (an agent's logo). **`titleInBody`** (default false) says the page renders its OWN title as an `<h1>` in the body (use the exported `pageH1Style` for consistency), so the chrome (nav bar / header) suppresses the title to avoid a duplicate — the tab-strip label via `usePageTitle` is unaffected. Adopters: `MetricDetailPage`, `DiffViewPage` (tsk137). **The body** takes `width?: "full" \| "reading"` and an independent `rightRail` (a 320px sticky control panel beside a body of either width); below ~960px the rail moves inline to the top of the body (`RailBody`) — "Body layouts" below. **Actions** are `PageAction[]` drawn in the nav bar, and `pin` adds Pin to Dashboard — "The nav bar is core's". Reads only semantic CSS variables (skin via theme). |
| `apps/desktop/src/tabs/PageNavBar.tsx` | Dumb nav-bar component: back/forward, the list stepper, the title, the bookmark star, the **comment navigator** slot (`comments` ReactNode), the primary and prominent actions as buttons, and the ☰ (`PageMenu`). Mounted by `Page` when context or explicit `navBar` prop is present. |
| `apps/desktop/src/tabs/PageMenu.tsx`, `tabs/pageMenuItems.ts` | The nav bar's ☰: one `ContextMenu` with everything else the page can be acted on with; `pageMenuItems` (pure, unit-tested) says what and in which order, `PageMenu` draws it and the panels it opens under the button (Backlinks, Outbound, Pin to Dashboard). "The nav bar is core's". |
| `apps/desktop/src/components/Comments/CommentNavigator.tsx` | Self-contained per-page comment navigator for the nav bar. `useCommentsForTarget(target)` (one URI) → shows "Comments (N)", steps through anchored comments with ◀ ▶ (each `requestCommentReveal`s to scroll + open inline on the surface), and a dropdown lists all comments plus an **Orphaned (M)** section. Orphaned entries are clickable too: they `requestCommentReveal`, and the surfaces open the thread popover at a fallback position (no anchor to scroll to) so the comment is readable. The popover then offers a **"Relink to selection"** button (`CommentPopover.onRelink`, wired by RichTextField + MonacoCommentLayer) that re-attaches the comment to the editor's current selection — select the intended text first. (The right-click "Relink orphaned" path still exists.) Comment-bearing pages pass it via `Page`'s `commentsNav` prop, on their URI (the editor page → the file's, WikiPage → the wiki page's, WorkItemPage → the item's); a declared page with `comments: true` gets it from `LensPage`. Renders nothing when the page has no comments. Pure helpers (`partitionPageComments`, `stepComment`) live in `pageCommentNav.ts` (unit-tested) and are shared with the per-thread stepper: `CommentPopover` takes an `onStep(dir)` prop so an open comment thread shows ◀ Prev / Next ▶ buttons that scroll to + reopen the adjacent comment (wired by RichTextField + MonacoCommentLayer via `stepComment` + `requestCommentReveal`). |
| `apps/desktop/src/tabs/PageNavigationContext.ts` | React context exposing `{ navigate(ref, { newTab? }), goBack, goForward, canGoBack, canGoForward, setTitle, title }` to descendants of an active page tab. Wrapped around every non-agent center tab in `App.tsx`. `BacklinksList` reads it so default-click navigates in-tab. The `usePageTitle(title)` helper registers the page's current title with the host so the same string drives the chrome header AND the tab strip label — no per-page duplicate header markup. **`useTabEditing({ dirty, save })`** tells the tab its page holds unsaved edits and how to save them (`setEditing`, on the current page only): `App.tsx` keeps one setter per tab (`editingSetterFor`) and draws `CenterTab.dirty` as a ● before the label, publishes `pageDirty` (the File → Save `when`), runs the shown tab's `save` from `oxplow.editor.save` when the page isn't a file, and keeps a dirty tab from the cap's `evictions`. The editor (`FilePage`) and the `markdown` viz's editor call it. |
| `apps/desktop/src/lens/EditorViz.tsx` | The `editor` viz that draws the working tree's file (foundation's `editor` page): its buffer from `editor/openFiles.ts`, `EditorPane` over it, `useTabEditing` for its ● / Save / Find. A renderer's lens page (`LensPage` with `renders`) shows the resource's backlinks and outbound links in the nav bar (`useBacklinks`), so wiki pages, tasks and commits that reference the file are listed there. `.context/editor-and-monaco.md` "The editor viz". |
| `apps/desktop/src/tabs/RouteLink.tsx` | Browser-style link button + the `useRouteDispatch(ref, { onNavigate?, pinnedSlot? })` hook that powers it; `ref` / `to` is a `TabRef` or a ref string (opened through `openRef`). Click semantics: left-click → in-tab navigate via `PageNavigationContext` (or `onNavigate` fallback when no context, e.g. rail / palette), Cmd/Ctrl-click + middle-click + right-click → new tab. The hook returns `{ dispatch, handlers }` so non-button rows (file tree entries, note rows, …) can adopt the same semantics without becoming a `<button>`. |
| `apps/desktop/src/components/RailHud/RailHud.tsx` | Persistent left rail HUD: a pinned search trigger at the top, then a set of **uniform collapsible panels** (every one an extension's — Go To, Uncommitted, Comments and Work from `oxplow-bundled`). Each renders through the shared `RailSection` wrapper — drag handle (⠿) + expand/collapse chevron + title + optional count badge + optional header action. (What needs the person — proposals, failed operations, undelivered events, firing badges — isn't a rail section: it's the dock's Alerts panel, the Alerts page and toasts; see `.context/usability.md`.) Sections **drag-to-reorder** (MIME `application/x-oxplow-rail-section`, the layout — order, collapsed, hidden — persisted in `panel_layout`; see "Left-nav panels" below). An extension panel with a `collapsed` lens (Work) keeps that summary when collapsed; the rest hide their body. Every section **always renders** (stable list) with an empty-state line ("Working tree clean", "No open comments", …) when it has no content. Passive — never auto-opens tabs. Go To is the `oxplow-bundled` panel `go-to` (bookmarks, then recent or most visited pages). |
| `apps/desktop/src/components/Navigator.tsx` | Far-left combined **stream + thread navigator** (each stream's open threads — `get_thread_state` leaves closed ones out; they're on the Closed Threads page): a 40px always-visible strip of letter glyphs (`navigator-strip-stream-<id>` / `navigator-strip-thread-<id>`) — a stream is a tab in the guide line's grey (`--text-muted`) with dark text (`--text-on-stream-tile`), coming out of the window's left edge and rounded on the right only, with its threads as tabs under it, all flush with the left edge and butted up against each other, each filling its shorter row and stopping short of the right (in the centre tab strip's header tint, `--panel-header-bg` over `--surface-card`, like a regular tab; the selected thread's accent bar is inside its tab, down its left, not beside it; a marked thread shows its mark as a corner fold on its top-left in `var(--thread-mark-<colour>)` (`navigator-thread-mark`, title "Marked <colour>"), not an outline; stacked tabs are set apart by a 1px divider along each one's bottom but the last's, in the guide line's grey (`--text-muted`), so none falls under the stream's tab, and the strip's column is `--surface-app`); a grey guide line runs down the column's right, flush with the stream tab's right edge, from under the stream's tab, with a tick from each thread's tab into it, ending at the last, each thread's agent status dot a node where the tick meets the tab; a stack's corners are rounded only where no tab sits next to it (the stream's top, the last tab's bottom), the rest flat; the strip draws no line against the HUD rail; in the expanded panel each row's tab runs on across the row carrying the title, set off from its abbreviation by a divider line through every row and a gap (`IconColumn wide`, `IconCell label`), so the tab's colour flows into the label, the glyph kept at its strip width so the two line up; the panel's rows draw the same icon column (`IconColumn`) so the two line up; clicking a glyph navigates directly — a thread glyph selects that thread, a stream glyph switches streams via `onSwitchStream`. **Hovering a glyph shows its full title as a native tooltip and nothing else (tsk269).** The `navigator-overlay` panel (~280px, covering the strip *and* the rail HUD to its right) expands only on an explicit click: the bottom-pinned `navigator-expand` chevron, dead space in the strip (`navigator-strip-empty`, `e.target === e.currentTarget`), a stream glyph (which also switches to that stream, then shows its threads), or the already-selected thread's glyph (selecting it again would do nothing). The chevron is pinned *outside* the scroll container and is the load-bearing affordance — once the list scrolls there is no dead space left, and `+ Add stream` lives only in the panel. The open/close state machine — click-to-open, geometric pointer-leave, Escape, outside-press, background-click, and the passive-vs-explicit guard — lives in the shared `apps/desktop/src/components/useSlideoutStrip.ts` hook (chevron in `SlideoutChevron.tsx`), which the Terminal page's `TerminalTabStrip` also runs on; a new slide-out strip adopts the hook rather than re-deriving it. It dismisses on a click in its own **dead background** (anything resolving to `button/input/select/textarea/a/label/[role=button]` is left alone so controls behave normally), on the pointer leaving its bounds (180ms grace), on Escape, and on any pointerdown outside it. The last two are *explicit* dismissals and beat the mid-rename / mid-new-thread form guard; the first two don't. **Pointer departure is measured geometrically** (document `pointermove` vs. the panel's rect), never `mouseleave` — the panel covers the rail and sits in the same DOM subtree as the strip, so the pointer never "leaves" the wrapper while parked over the covered region, which is what used to strand it open on top of the rail and swallow clicks meant for it (tsk131). Each overlay row (`navigator-stream-row-<id>` / `navigator-thread-row-<id>`) — and each strip glyph, with the same menu — opens its action menu on **right-click** (Menu key / Shift+F10 for keyboard), headed by the stream's or thread's name (`context-menu-header`); Rename and Add thread open the panel for their inline field. **Thread menu:** **"Mark with…"** (`menu-item-thread.mark`) opens a submenu headed by the line "A colour on the thread's tab, for grouping threads your way. It changes nothing else." (`context-submenu-header`, the generic `MenuItem.submenuHeader`) and lists `amber`, `red`, `green`, `blue`, `violet` (each with a colour chip, `MenuItem.swatch`) and None as `menu-item-thread.mark.<colour|none>`, the current one checked; picking one runs `oxplow.thread.mark` (via `onMarkThread` → `App.handleMarkThread`). Then Rename / Settings / Close. A mark only groups threads the person's way; every open thread of a stream shares its worktree. |
| `apps/desktop/src/tabs/bookmarks.ts` | The person's bookmarks, project data (data-model.md "bookmark"): `useBookmarks(threadId, streamId)` reads `v_bookmark` — a page once, at the narrowest scope the thread sees (thread / stream / project) — and `setBookmark` / `removeBookmark` run `oxplow.bookmark.set` / `oxplow.bookmark.remove`. Pages bookmark via the `PageNavigationContext.bookmark` binding (one `scope`; the star's menu moves it or, on its current scope, takes it off); the Go To lens page's bookmark row actions re-scope and remove. |
| ~~`apps/desktop/src/tabs/appPageBacklinks.ts`~~ | **Deleted.** Per-kind in-memory backlinks providers used to live here. Cross-page backlinks now come from the persisted `page_ref` graph (`crates/oxplow-db/src/page_ref_store.rs`) via the `list_backlinks` IPC; every page kind goes through the same code path. App pages that need their own provider would register a new `source_resource` writer in the backend instead. |
| `apps/desktop/src/components/RailHud/sections.ts` | The launcher's categories (`PAGE_CATEGORY_ORDER`, `PageDirectoryEntry`) ; every launcher page is a declared page (`components/extensionLauncher.ts`). |
| `apps/desktop/src/components/History/CommitGraphTable.tsx` | Pure presentation of the git-log graph (branch/merge dots + lines + sha + ref badges + subject + author + relative date). `indexRefsBySha(log)` exported alongside groups branch heads + tags by sha so callers feed identical maps. |
| (change analysis) | No page of its own: `crates/oxplow-app/src/change_analysis.rs` stores each change behind `v_change*`, and the oxplow-bundled `change-review` lens grid shows it in the `vcs.commit.details`, `vcs.status.details` and `oxplow-foundation/changes.sections` slots. The old per-page scope drilldown is gone; scoped tab ids reopen the unscoped page. |
| ~~`apps/desktop/src/tabs/backlinksIndex.ts`~~ | **Deleted.** The in-memory cross-kind indexer is replaced by the persisted `page_ref` table; see `data-model.md`. The `BacklinkEntry` type that renderers consume now lives in `apps/desktop/src/tabs/backlinkTypes.ts`. |
| `apps/desktop/src/tabs/useBacklinks.ts` | React hook that calls the unified `list_backlinks` IPC for a `TabRef` and maps the returned `BacklinkEdge` rows into `BacklinkEntry`s. Used by every page kind including the editor page. The sibling `usePageOutbound` hook does the same for the inverse direction. |
| `apps/desktop/src/tabs/backlinkTypes.ts` | Renderer-side `BacklinkEntry` interface (`{ ref: TabRef \| null, target, label, subtitle?, dead?, linkType? }`): `ref` is the page the row opens, null for a dead link (nothing resolves the ref; `BacklinkEdge.resolved`) or a resolved ref with no page. Decoupled from the SQLite `BacklinkEdge` shape. |
| `apps/desktop/src/tabs/BacklinksList.tsx` | Default renderer for the Page chrome's `backlinks` slot — buttons that route via `onOpenPage`; a row with no page renders muted as its raw ref (dead link). `entryMenu` supplies a row's right-click actions (a work item's link rows: Unlink). |
| `apps/desktop/src/components/Analytics/DailyBarChart.tsx` | Generic daily bar chart (`{ label, value }[]`); renders `bar` lenses. |
| `apps/desktop/src/components/CollapsibleSections.tsx` | Collapsible page sections (tsk84, tsk86). Two parts plus a hook: `CollapsibleSections` (state provider), `SectionCollapseControls` (the "Expand all" / "Collapse all" pair), and `useSectionCollapse`, which the kit `Section` (`collapsible`, `defaultCollapsed`; the toggle is a button inside the `<h2>`) reads. Sections register themselves on mount so the controls know what "all" means — only what's **currently rendered** counts, so a filtered-out section is never silently expanded — and the controls self-hide when nothing is registered. **The page places the controls**: Recorded Metrics renders them in the **details rail** beside its filters, which works because the provider wraps the whole `<Page>` and `rightRail` is *rendered* inside Page's subtree (context follows the render tree, not the creation site). For that to be safe the provider renders `children` **bare** — a wrapper element there would break the page chrome's `height: 100%` column. Collapsed ids are the tab entry's state (`useHistoryState`); default is expanded, and stored ids are deliberately **not** reconciled against the rendered set (a section hidden by a page's search must return still collapsed). Pure state/persistence in the sibling `sectionCollapse.ts`. **Not a `Page` prop** — see `.context/usability.md` → "Collapsible page sections". Adopters: none now. |
| `apps/desktop/src/components/Slideover.tsx` | Right-edge panel primitive (~38vw, backdrop-click + Escape close, focus-into-panel on open) for form-shaped flows that don't justify a full page. Use instead of a centered modal. |

## Renderers

A URI names a thing; a **page** is one **renderer** of it. A tab's id is
the URI it shows, and nothing in the desktop keeps a list of page kinds.
Which page draws a URI comes from declarations
(`pages/renderers.ts`):

1. The pages whose `for` names the URI's resource (an oxplow resource
   `<extension>/<name>`, or the scheme `https` / `http` for a web page)
   and whose `accepts` hold for its params.
2. The view key `#oxplow.renderer=<extension>/<page>` picks among them.
3. Else the page with `default: true`; else the first.
4. Else foundation's `resource` page (`pages/ResourcePage.tsx`): the
   resource's label and params, or the raw URI muted as a dead link when
   nothing declares the resource or its model has no row. Never an
   error, never a toast.

**Open With** in the nav bar's ☰ (a submenu, `menu-item-page-open-with`,
each renderer `menu-item-page-open-with-<extension>/<page>`, the one
drawing it checked) lists the renderers when there are several. Picking one is a view change
(Back returns to the first). `App.tsx` `renderTab` draws a route
(`oxplow:oxplow/page?page=…`) through `routeTab`, and anything else through
its renderer: a lens (`LensPage`, run with the URI), a custom component,
or the two built-ins — the `lens` renderer (the lens runtime) and
`builtInPages.resource` (the fallback). A pinned renderer's tab
(`pinned: true`, the agent session) is pinned; an agent session's tab also
draws its harness's logo, its status dot and what it waits on, from the
window's session list and statuses (`centerTabs`), and a page's actions
(`useTabActions`, `setTabActions`) are its tab's right-click menu. Page memory (history state, scroll) is scoped by
`keyIdentityOf` for a page, lens or metric, and `resource:<resource>`
otherwise. A tab's label and icon come from `recordedPageKind(ref)` /
`kindForTabId`: a route's page id, else the URI's resource
(`oxplow/file`), else a web page's scheme.

Foundation declares oxplow's renderers (`agent-session`, `commit`,
`editor`, `file-viewer`, `lens`,
`resource`, `wiki-page`, `work-item`, and `diff-view` also for
`oxplow/snapshot`, `oxplow/effort` and `oxplow/turn`; `custom-dashboard`
(a lens renderer, `dashboard`) for `oxplow/dashboard`; `external-url`, a lens page over the `embed` viz, for `https` and `http`). Their
bodies are listed under "Modules" and "Declared pages". An agent
session's tab follows its row (`reconcileSessionTabs`). There is no
`change-analysis` page.

## Tab id format

Built only by the helpers in `apps/desktop/src/tabs/pageRefs.ts` — never
hand-format an id.

**A tab's id is the URI it shows** ([resources.md](./resources.md)),
parsed through the shared grammar (`apps/desktop/src/uri/uri.ts`), never
split by hand (`oxplow:oxplow/work_item?id=tsk42&provider=oxplow`). A web
page's id is its URL. A tab's view (`#lines=…`, a filter, a pick) rides in
the id after `#` but is not its identity: the `?` names what the page
shows, the `#` how it shows it, and the identity is the id without its
`#` (`identityOf`). A `[[…]]` link, a `page_ref` row, a backlink and a tab
all agree on one string. Two shapes:

- **A thing** (a file, directory, wiki page, work item, commit, metric,
  lens, web page, …): the id is the thing's URI, and a renderer draws it
  ("Renderers").
- **A route** (`oxplow:oxplow/page?page=<id>`): a page an extension's
  manifest declares under `pages:` without `for` — not a thing in the
  graph. Its id is
  `oxplow:oxplow/page?page=<id>` plus its params for oxplow's own
  (oxplow-foundation has `namespace: oxplow`, so its pages are
  core-named as its commands are: `oxplow:oxplow/page?page=tasks`,
  `oxplow:oxplow/page?page=settings#area=agents`, the area in the `#`
  view) and
  `oxplow:oxplow/page?page=<extension>/<id>` plus its params for any other
  extension's. `pagePayload(ref)` reads it back as `{ extension, page,
  params }` (`extension` null for oxplow's). Nothing
  rides beside the id: what a page shows is its params (a diff's spec,
  Settings' landing section, a new item's parent), and what only one step
  of a tab keeps (a clipboard compare's two texts) is that entry's history
  state. A `?` param names the thing shown; a view (a filter, a pick, a
  line, a mode, an open panel) is the `#`. Routes never appear
  in `page_ref`. Params are the URI's own, sorted by key, and their values escape only
  what the grammar needs (`&`, `=`, `#`, `%`, whitespace, non-ASCII),
  so `path=src/a.ts&left=git:abc` stays readable.

`refFromTabId(id)` rebuilds the tab ref from an id alone: every oxplow
URI is a tab (an agent session's is pinned), and an allowlisted http(s)
URL is a tab ([external-url-tabs.md](./external-url-tabs.md)). It returns
`null` for anything else, and callers (rail History, Go To, the
launcher's Recent) drop the row rather than open a blank tab. Whether a
page by a route's id exists is the declaration's business, and an
undeclared one draws "No such page"; a URI nothing renders draws the
fallback `resource` page. `newTaskRef(payload)` names the `new-task`
page (a foundation lens form, `new-task`) with its `parent` param.

There are no sentinel tab ids (`HOME_TAB` in `App.tsx` means "nothing
chosen", never a tab), and "is this the editor's working-tree file" is
`diskFilePath(id)` (null for a pinned revision such as
`oxplow:oxplow/file?path=src/a.rs&rev=git:HEAD`, which is a read-only viewer tab).

**No compatibility layer for old ids.** When a kind is renamed or a page
moves into an extension, its saved tabs/bookmarks/history are dropped
(the tab-layout keys started fresh again as `oxplow.layout.v3.*` when
pages became declared, 2026-10-17, as `oxplow.layout.v5.*` when ids
became URIs, as `oxplow.layout.v6.*` when the view moved to the `#`, and
as `oxplow.layout.v7.*` when a tab became just its URI; bookmarks
moved into the project DB and started fresh again) — see the decision in [resources.md](./resources.md).

| Thing | Id format | Example |
|---|---|---|
| file | `oxplow:oxplow/file?path=<path>[&rev=<rev>][#lines=N-N]` — a line is the view (`fileRefAt(path, line)`, `lineOf(ref)`); the working tree has no `rev`; `rev=git:<ref>` / `rev=snap:<id>` pin a version (`revisionSlot` in `revision.ts`); a literal `&`/`=`/`#`/`%` in a path is percent-encoded | `oxplow:oxplow/file?path=crates/oxplow-app/src/lib.rs`, `oxplow:oxplow/file?path=src/a.rs&rev=git:HEAD` |
| dir | `oxplow:oxplow/dir?path=<path>` | `oxplow:oxplow/dir?path=crates/oxplow-app` |
| wiki | `oxplow:oxplow/wiki?slug=<slug>` | `oxplow:oxplow/wiki?slug=how-stop-hook-fires` |
| work_item | `oxplow:oxplow/work_item?id=<id>&provider=<provider>` — every list's item (`workItemTabRef(uri)`) opens `WorkItemPage` | `oxplow:oxplow/work_item?id=tsk142&provider=oxplow`, `oxplow:oxplow/work_item?id=W-1&provider=fake` |
| commit | `oxplow:oxplow/commit?sha=<sha>` | `oxplow:oxplow/commit?sha=abc1234` |
| metric | `oxplow:oxplow/metric?metric=<key>[#view]` (`metricRef(key, view?)`) | `oxplow:oxplow/metric?metric=oxplow.coverage.abs_pct` |
| lens | `oxplow:oxplow/lens?lens=<extension>/<slug>[&param=value…][#view]` (params sorted; `lensRef(id, params, view)`: `params` are the lens's `resource` params, its filters and picks are the `#`) | `oxplow:oxplow/lens?lens=review/waiting-on-me`, `oxplow:oxplow/lens?effort_id=12&lens=oxplow-bundled/effort-tests` |
| route | `oxplow:oxplow/page?page=<id>[&params]` (oxplow's) / `oxplow:oxplow/page?page=<extension>/<id>[&params]` (`pageTabRef(id, params?, view?)`, `extPageRef(ext, id, params?, view?)`, plus typed wrappers: `diffRef`, `endpointDiffRef`, `duplicateBlockRef`, `settingsRef`, `newTaskRef`, …) — a route; see "Declared pages" below | `oxplow:oxplow/page?page=tasks`, `oxplow:oxplow/page?page=symbols&path=src/lib.rs`, `oxplow:oxplow/page?page=github/open-prs`, `oxplow:oxplow/page?page=acme/pr&ref=oxplow:acme/pr?id%3D12` |
| web page | the URL itself, `https://example.com/docs` (`externalUrlRef(url)` is `{ id: url }`) — rendered by foundation's `external-url` page (lens `web-page`, `viz: embed`), `for: [https, http]`, its tab labelled `{{param.uri|url}}`; see [external-url-tabs.md](./external-url-tabs.md) | `https://example.com/docs` |
| symbol | `oxplow:oxplow/symbol?path=<path>&symbol=<name>` (`v_symbol.uri`, `symbolRef(uri)`) — no revision in the id; never a tab: opening one (`handleOpenPage`, or in-tab navigation) resolves it (`resolve_symbol`) and opens its file at the name's line (`openSymbol`). A pinned one (`rev=snap:<id>` / `rev=git:<sha>`) opens the file viewer at that revision; if the version is gone it shows a notice and opens the current symbol | `oxplow:oxplow/symbol?path=src/lib.rs&symbol=Widget::spin` |
| agent_session | `oxplow:oxplow/agent_session?id=<ses id>` (`agentSessionRef(id)`, pinned; the one URI whose tab is pinned) — an open agent session's terminal; its tab draws its harness's declared logo (`CenterTab.icon` from `HarnessListing.icon`, `AgentIcon`; the generic robot when it declares none) | `oxplow:oxplow/agent_session?id=ses3` |

## Page files

The unit replacing a lens plus its `pages:` entry (being built: they load
beside lenses and run nothing yet; the lens runtime is deleted once
oxplow's own extensions move over). A page is `pages/<slug>.yaml` in an
extension, three parts (`crates/oxplow-app/src/pages/schema.rs`):

- **Metadata**: `title`, `description`, `label`, `icon`, `category`,
  `keywords`, typed `params` (`role: resource | filter`; a `switch`'s param
  is a filter by default), `for` / `accepts` / `default` / `pinned` (a
  renderer), `comments`, `refresh`, `hidden`, `needs`, whole-page
  `actions`, `alert`, `tool`, `answers` (`{ question, about?, params,
  skill? }`: what agents find it by and what tests run) and `badge`
  (`{ loader, column? }`).
- **`data:`** — named **loaders**, in order; a loader reads only the ones
  before it: `sql` (binds `:param`s; an earlier loader reads as a table of
  its name), `command` (a read-only command run as the viewer, `args`
  each `{ param }`, `{ from: <loader>.<column> }` or a literal; it must
  say `reads` (models) or `refresh` (`manual`, `{ every }`)), `page`
  (another page's `loader`, with `params`) or `rows` (inline). A loader's
  name is a plain identifier, never a model's (`v_*`, `m_*`).
- **`view:`** (and an optional `rail:`) — a tree of nodes: `stack`,
  `grid` (children with `size`), `switch` (`{ param, children }`: the
  child whose id is the param's value), `viz` (+ `from: <loader>`, the
  viz's role block, `columns`, row `actions`, `empty`), `embed`
  (`{ page, params }`: params flow by name from the host's params and its
  loaders' first rows, `params` naming any explicitly), `slot` (a name
  other extensions mount pages at), `markdown` (static text) and
  `component` (`{ id, props }`: a custom component fed the page's data).
  Every node has an `id` (default: its loader, embedded page, slot or
  component; else `stack`, `stack-2`, …), unique in the page; `title`,
  `replaceable` and `size` are on any node.

A viz node's config is a `Lens` with no query (its rows are its loader's),
so the shape checks, action parsing, text renderers and the desktop's viz
components are the lens ones. The YAML only binds: logic is SQL or a read
command. An **inline page** (`parse_inline_page`: a markdown
` ```oxplow-page ` block, an agent's one-off) only reads: no `command`
loaders, no actions, forms, terminals, editors, components or slots.

**Running one** (`pages/runner.rs`, `PageRunner::run`): params resolve as
a lens's do (supplied, else the viewer's context, else the default;
`head_sha` the viewed stream's HEAD). Loaders run in order: a `sql`
loader gets each earlier loader it names as a temp table of that name
(`SqlQuery.temp`, beside `metric_grid()`'s); a `command` loader runs only
a command whose access is a read (`Access::reads_only`), as the viewer
(`Actor::Lens { lens_id: <page>#<loader>, on_behalf_of }` until the actor
is renamed), its result a `{ columns, rows }` table as is, objects a row
each, else one `value`; a `page` loader runs that page and takes its
loader. A loader that fails carries its `error` and the rest of the page
still shows. Within one run, embeds included, a loader with the same
definition and inputs (its `key`) runs once. A loader only a switch's
hidden children show doesn't run; one no node shows does (it feeds an
embed, the badge or a later loader). The tree resolves a switch to its
child in view (the param's value, else the first) and an embed to its
page's run, its params by name: the embed's `params`, else the host's
param of that name, else that column of the host loaders' first rows in
order. A `slot` node shows the pages enabled extensions mount there
(`ui.slots: [{ slot: <ext>/<page>.<name>, page: <slug>, at?, title?,
size? }]`, loaded as `ExtensionUi.page_slots`): each runs as an embed, its
params by name from the host, its node id the mounted page's id with `.`
for `/` (`beta.extra`); by extension name; `at: start` puts it first
among the slot's siblings, `{ before | after: <sibling id> }` beside that
sibling, otherwise it sits at the slot. A **panel**'s parts (`body`,
`badge`, `collapsed`, `count`) may name its extension's pages instead of
lenses (`ExtensionPanel.pages`; a badge page declares `badge` or
`alert`, a stream or thread panel's pages take `stream_id` /
`thread_id`), and a **replacement** may name a page (`ui.replacements:
[{ target, page }]`, `UiReplacement.page_id`), checked against the same
props contract. Which replacement shows is still decided in the window
(`useReplacement`: the item's provider and the person's choice). `dirty_loaders` is what a change makes run again: loaders that
read a changed model (a command loader's are its `reads`) or bind a
changed param, and everything downstream of them.

**Drafts** (`PageRunner::draft(svc, checkout, extension)`): the
extension's source in another stream's checkout, run over live data —
its pages found first, its models read through an overlay of temp views
(`extensions::draft_overlay`), its commands whatever is deployed (a
draft's code never acts). `oxplow.page.run { page, draft: true }` reads
the caller's stream's draft; its text starts `Draft of <page> (stream
strN, not deployed)` and its json carries `draft: { stream, checkout }`.

**Open runs** (`pages/open_runs.rs`, `Services.page_runs`): a window opens
a run (`open_page { page | specYaml, params, streamId, threadId, client }`
→ `{ runId, run }`) and closes it (`close_page`); a pick, filter or
switch is `set_page_params`, Refresh `rerun_page`. The change loop
(`models_changed::spawn`) signals `PageRuns` as it tells the assets
(never a second bus listener): models changed, measures landed, or
everything (it lagged). A worker started at boot coalesces signals for
100 ms and re-runs each open run one of whose loaders (embeds included)
read a changed model or measure — a command loader's are its `reads` —
then emits `OxplowEvent::PageRunChanged { runId, seq, loaders, tree }`
for each that changed: the loaders whose rows differ (an embed's as
`<embed>/<loader>`), never the rows. The window fetches what changed in
one `fetch_page_runs` batch and drops a reply older than a `seq` it
applied (`apps/desktop/src/pageRuns.ts`: runs shared by key, closed
250 ms after the last host lets go, every run opened again after a
reconnect). Runs belong to their window's `client` and go with it
(`keep_only_clients`). A re-run runs the whole page today; running only
the dirty loaders (`dirty_loaders`) comes once this is end to end.

**Actions** (`pages/actions.rs`, `run_page_action { runId, node?, action,
row?, input?, drop?, confirmed }`): pressed on an open run. A whole-page
action binds the run's params; a node's row action runs on the row the
presser names, which must be one the run holds (matched on the columns
given: a row that changed under the press is refused); a holds-one-row
action on its node's one row, read by the server. The checks and input
binding are the lens ones (`lens_actions::prepare_resolved`), and it runs
as the page acting for the presser. An agent presses with
`oxplow.page.run_action { page, params, node?, action, row? }`, which
holds a run of its own for the press. Forms: `page_form { runId, node }` is a
form node's start (its defaults with params bound, its loader's first
row, its `choices` and object fields' `from` read from the pages they
name: `<ext>/<page>` takes its first loader, `<ext>/<page>#<loader>`
that one), `submit_page_form` submits it as the page for the person, and
`page_action_form { runId, node?, action, row? }` is a form action's
start. A whole-page action may read `{{row.x}}` / `when:`: the first
loader's one row.

**The desktop host** (`apps/desktop/src/pages/PageHost.tsx`, built and
tested, not wired into `App.tsx` yet: lens pages still draw every tab).
`PageHost` takes a `PageKey`, holds its run (`usePageRunWith`; `RunState`
carries the `runId`, `rerun` is Refresh) and draws `tabs/Page.tsx`
chrome: the title is `tabLabel` over the run's params and `labelRow(run)`
(the first declared loader's first row), whole-page actions and Refresh
(`page.refresh`) are page actions, pressed through `useLensActions` with
a runner that calls `run_page_action` with no node, so a command that
asks is confirmed as a lens action's is. `PageRail` lists the filter
params (`ParamList`; a change is `setParams`) and draws `run.rail`.
`PageTree` draws a `RunNode` tree: stack → `Section`s, grid → `TileGrid`
at each child's `size`, switch → its one child, viz → `LensResultView`
over `nodeRunOf` (`lens/nodeRun.ts`, the server's `node_run`), embed → a
nested tree with ids `<embed>/<id>`, slot → its children, markdown → the
`Markdown` component, component → a placeholder (the bridge comes
later), a node or loader error → an inline `ox-error`. Every node's root
has `data-node` and registers in a `NodeRegistry`; `useNodeFocus` lands
on `#node=<id>` (scroll, `data-focused` for 1.5 s). Viz components press
their actions through `LensActionRunnerContext`
(`lens/lensActionRunner.ts`: `run_lens_action` by default); `PageTree`
provides `useNodeActions(runId, nodePath)` per viz node, so the same
components draw both. New params keep the old run on screen
(`data-stale`), and Loading shows only after 150 ms without a run.

## Declared pages

Every page is a `pages:` entry in an extension's manifest
(`.context/extensions.md` "Pages"); oxplow's own are in
oxplow-foundation's. A page with no `for` is a **route**
(`oxplow:oxplow/page?page=<id>`); a page with `for` is a **renderer** of
a resource ("Renderers"). The declaration gives the title (its name), the `label` its tab says
(`{{param.<name>}}` over what it shows, `{{row.<column>}}` over the one row of a document lens with a `query:` once it has run, filters `|short` and `|basename`; none: the title), icon, launcher
`category` (none: not in the launcher — a page only opened from
elsewhere; a page whose lens is a view document also lists each section
but the first, `<page>: <section>` at `#<view>=<id>`), `keywords`, the `params` its id carries, a `count:` lens for
the launcher badge, and what draws it: a `lens:` (most pages) or, for
oxplow's own until the kit can draw them, `component: oxplow:<name>`, a
**built-in page**. A renderer also has `for`, `accepts`, `default` and
`pinned`. A page whose body is a custom `component:` may name a `rail:`
lens of its extension, drawn in its rail with the page's view (Explore
Data's catalog: extensions.md "A custom page's rail"). The built-ins are pinned by
`crates/oxplow-app/src/built_in_pages.rs` and its golden
`fixtures/built_in_pages.json`. The list grew once with the renderers
(`agent-session`, `commit`, `directory`, `editor`, `file-viewer`, `lens`,
`resource`, `wiki-page`, `work-item`) and only shrinks
again (a page leaves when it becomes a lens or a custom page — Explore
Data is oxplow-foundation's custom page now; the editor, the Terminal and
the Web Page lenses over the `editor`, `terminal` and `embed` vizes). It
ends at two — every page is a lens now, the agent session a document of
`approvals` and `terminal`: `lens` (the lens runtime, which draws every lens
page) and `resource` (the fallback when nothing declares a URI) stay —
neither can be a lens;
`pages/declaredPages.ts`'s `BUILT_IN_PAGES` mirrors it and a test checks
they match.

`App.tsx` `renderTab` draws a route through `routeTab`: it finds the
declaration (`declaredPage`, by tab id among the enabled extensions),
draws a built-in from the
`builtInPages` table (`pages/builtInPages.tsx`) by name, a lens page through
`DeclaredPageView` (`LensPage` with the id's params), and a custom page
(`body.kind === "component"`) through `CustomPageView`: the component's
frame filling the body under the nav bar, told its stream and thread
and given the whole entry as its view (`useHostedView`), its title and
actions the frame's (`setTitle`, `setActions`). oxplow's Timeline (the `timeline` page, params `from`, `to`,
`thread`) is one: its window is its view params, written on every change
(not its opening default) and followed on Back/Forward. A frame's
`setViewParams` reaches the tab as `patchViewParams` (null drops a key); both write the id's `#`. Each built-in body is a
`(ctx) => CenterTab | null` over `{ ref, params, nav }`; `nav` is the slot's
`SlotNav` (`pages/slotNav.ts`), so a body never reaches into App's locals. While the
extensions load, an oxplow page whose id is a built-in's name draws the
built-in straight away. Icons and labels come from the declaration too:
`kindForTabId` gives a route's page id (`tasks`, `gh/open-prs`) or a
URI's resource as its kind,
which is also what visits and bookmarks record (`recordedPageKind`),
and `PageKindIcon` / `lensIcon` / `pageKindLabel` resolve it to the
page's icon and title. The launcher lists the declared pages with a
category (`launcherPages`), each `count:` lens run when it opens
(`useLauncherCounts`); a view document's page also lists each of its other sections.

oxplow's pages and their params:

- Lens pages: `wiki-page` (the renderer of `oxplow/wiki`, tab label `{{param.slug}}`, `comments: true`: a document of `wiki-page-body` — a `markdown` viz over `v_knowledge_body` with an `edit: true` Save on `oxplow.knowledge.write_page` and a Delete Page action — `wiki-page-info`, last edited and freshness, and `wiki-page-refs`, the cited files with Mark verified; a page not yet written is a row with an empty body, made by its first Save), `board` (a document with one replaceable part, `board-cards`: a `viz: board` of `v_work_item` grouped by state; `show` — this thread, the backlog, everything — is its filter), `alerts` (a document of `alerts-decisions` (approvals), `alerts-hints`, `alerts-operations` with `expand` into `alerts-operation`, `alerts-deliveries`, `alerts-reactions` and `alerts-extensions`), `duplicate-block` (a route, params `left`, `left_lines`, `left_at`, `right`, `right_lines`, `right_at`: a `code` viz of two rows with `sync: true`, the ranges marked), `diff` (a route, params `path`, `left`, `right`, `label`, `base`, view `lines`: a `diff` viz over one file's two revisions filling the page, tab label `{{param.path}} ({{param.label}})` — `diffRef` always sets `label`, "diff" by default; a diff of literal text, Compare with Clipboard, keeps its sides in the entry's history state, which `DiffViz` reads), `file-viewer` (the renderer of `oxplow/file` with a `rev`: a `code` viz of the file at the revision, `#lines=` marked), `answer` (the renderer of `oxplow/answer`, tab label `{{row.title}}`: a document whose bare section is the `answer` viz; what an agent shows opens as one, behind the shown tab — extensions.md "Thread answers"), `editor` (the renderer of `oxplow/file` with no `rev`, tab label `{{param.path|basename}}`, `comments: true`: an `editor` viz of the working file, `#lines=` landed on — `.context/editor-and-monaco.md` "The editor viz"), `commit` (the renderer of `oxplow/commit`, tab label `{{param.sha}}`: a document whose row `SELECT id AS change_id, base_revision, head_revision FROM v_change WHERE kind = 'commit' AND target = :sha` makes the commit's change and hands it to its sections — `commit-detail`, a `detail` over `v_commit WHERE sha = :sha` with Cherry-pick and Revert through `oxplow.git.cherry_pick` / `revert`, and `commit-files`, a `tree` over `v_commit_file` with status icons linking each file's diff — and to the lenses mounted at `oxplow-foundation/commit.sections`; any sha git knows reads, by its full id) and `uncommitted` (a document on the working change, `kind = 'working' AND target = ''`: `uncommitted-commit`, a form over `oxplow.vcs.commit`, and `uncommitted-files`, a tree over `v_change_file`; mounts at `oxplow-foundation/uncommitted.sections`), `comments` (the Comments inbox: a `document` of `comments-threads` — a table over `v_comment` for the stream, grouped by the page each is anchored to, `days` a choice filter on how far back resolved ones show, a row `select`s the `comment` param — and `comments-conversation`, the picked thread's `v_comment_message` rows; row actions are `knowledge.reply_comment` (a form), `update_comment` (resolve, reopen, note / follow-up) and `delete_comment`; the Go to location column is a link to the target page and does not scroll to the anchor), `directory` (the renderer of `oxplow/dir`, `accepts: { rev: absent }`, tab label `{{param.path}}`: a folder's rows from `v_workspace_file WHERE stream_id = :stream_id AND parent = :path`, folders first, each linking to its folder or file; params `uri`, `path`, `stream_id`), `problems`, `symbols` (`path`, `filter`), `closed-threads`,
  `wiki-index`, `wiki-freshness` (`slug`), `go-to`, `dashboards`,
  `hook-events`, `done-work`, `catalog`, `new-stream` (a form: `visible_if` on its `source`, `oxplow.stream.create`), `stream-settings` (`stream`) and `thread-settings` (`thread`) (documents: a hint, the thread's open sessions, and the prompt as an edit form over `v_stream` / `v_thread` with `oxplow.*.set_prompt`), `git-history` (a document with one replaceable part: the commit list, or the history graph; params `q`, `author`, `branch`), `local-history` (a document of
  two lenses; params `mode` `recent` | `by-commit` and `all`, which lifts
  the caps — the rail's, so no "View all" step to go Back from).
- Lens pages (more): `backlog` (badge: `backlog-count`; a document of `backlog-add`, a quick-add form over `oxplow.work_item.create`, and `backlog-list`, a state-grouped `list` of the items on no thread: params `q`, the search over title and fields, and `thread_id`; drag to reorder, mark rows for bulk state, priority, Bring to this thread and Delete, `r` `i` `b` `d` keys for the states).
- Lens pages (more): `diff-view` (the Changes page, the renderer of `oxplow/snapshot`, `oxplow/effort` and `oxplow/turn`; view params `start` and `end`, each `''` / `none` / `working` / `snap:<id>` / `git:<rev>`: a picked point is a pushed view change), `tasks` (a document: `tasks-list`, the `work_item.list` replaceable; `tasks-summary`; `tasks-backlog`; view params `show` and `priority`, plus `thread_id` and `stream_id`).
- Lens page `terminal` (the stream's terminals: a `terminal` viz over `v_terminal`, the picked one the `terminal` selection, `new` / `rename` / `close` actions over `oxplow.terminal.*`; terminal.md "The terminal viz and its host").
- Lens page `settings` (a view document: view `area`, the overview's search `q`; `#section=<id>` lands inside an area; extensions.md "A document with a view"): a section per area, each a document lens `settings-<area>`.
- Built-in pages: `board`,
  `files`, `git-dashboard`, `uncommitted-changes`,
  `comments`, `metrics-recorded`,
  `alerts`,
  `new-task`
  (`parent`),
  `diff` (`path`, `left`, `right`, `label`,
  `base` — the left side's name when it isn't the version's own — and
  `line`, a view param: the same diff opened at another line moves the
  tab showing it; `diffSpecOf` reads the spec back; a clipboard compare's
  texts are history state, `diffTextOf`), `duplicate-block` (`left`, `left_lines`,
  `left_at`, `right`, `right_lines`, `right_at`; `duplicateBlockOf`).

New Session (the `new-session` page) is a foundation document lens:
**Start a new session** (a form over `oxplow.agent_session.open`: the agent
from `v_agent_harness`, and "remember" — the person's `newThreadSession` —
folded into the command) then **Earlier sessions**, and what extensions
mount in `oxplow-foundation/new-session.sections` (ids `start`, `earlier`; it binds
`thread_id`). Earlier sessions is a lens table (`new-session-earlier`,
a `q` search in the rail; ↑/↓/Enter on the focused table; a row's
Reopen/Resume action, `then: replace`, or its link to a session already
open here — `terminal.md` "Earlier sessions"). A diff's `revealLine` is not in its
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
proposals, failed operations, undelivered events — isn't a rail panel: it's
the status bar's bell and the Alerts page (see usability.md); a firing badge
shows in its panel and a toast, not on the page. A
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
| **Document** (the default) | reading, picking, small forms, lists | `lens/PartsViz.tsx` (`DocumentViz`): title + intro in a reading column, then `Section`s (a heading over a rule, no boxes) | `viz: document` + `sections:` |
| **Dashboard** | many numbers and charts at a glance | `TileGrid` / `TileSlot` / `Tile` at full width | `viz: dashboard` + `tiles:` |
| **Board** | cards by group, dragged across columns | (none) | `viz: board` + `group: { by, values }` and a `drag: true` action |
| **Document with an edited body** | a wiki page, a work item's body: markdown read and written, with a Contents rail and comments | the `Markdown` component (`.context/markdown.md`) | `viz: markdown` + `markdown: { text }` and an `edit: true` action (its Save), the page `comments: true` |
| **Reader** | one file, two files side by side, or one diff, at the page's whole size | `Page` with `fill` (the body stretches to the page's height) | `viz: code` / `viz: diff` |
| **Custom** | what the kit can't express: editors, terminals, boards, git flows | a React page on `Page`, using kit parts where they fit | `viz: custom` (the sandboxed component), or a whole custom page: `pages[].component: <id>` (`CustomPageView`) |

A cell's match is drawn with the kit class `.ox-mark` (a lens column's `highlight`, extensions.md "Lenses").

The kit: `Section` (`id`, `title`, `description`, `actions`, `collapsible` / `defaultCollapsed` — a toggle in the heading, remembered per page, see usability.md "Collapsible page sections" — `source` —
the extension that added it), `DataTable` (fixed `<col>` widths,
`numeric` and `truncate` columns, groups, per-row props — the lens `table`
viz draws through it), `Toolbar` (a core list's filter and actions; a
lens page has none; `useSearchFilter(key, …)` builds its `filter` on the `key` view
param) with `useListNav`
(the one ↑/↓/Enter/Escape contract, usability.md "Keyboard"; a list that
holds focus itself spreads its `keyProps`), `Stat` (a figure; `big` when
it is its place's whole — the lens `number` viz), `Facts` (labels beside
values — the lens `detail` viz, a metric's definition),
`TileGrid` / `TileSlot` / `Tile`, `Loading`, `ActionButtons`, and
`EmptyState` (re-exported; it lives beside the agent prompts it offers).
Lengths come from the spacing and radius tokens (theming.md).

**Extensions add to a page through its parts slot**
(`<extension>/<slug>.sections` of a document lens, `.tiles` of a dashboard),
placed `at` the start, the end, or beside a part by its stable id —
extensions.md "Slots".

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
  filters, Git History's holds its find, author and branch.

The rail is **purely responsive — no user toggle**. A `ResizeObserver`
on the body watches its width; below `960px` the same panel moves to the
top of the center column (`RailBody`). A page with no rail content for
some condition passes `rightRail={undefined}`.

**Which width:** detail pages (single record — wiki page, task, …) and
Document pages use `width="reading"`; lists, dashboards and editors stay
full.

## The nav bar is core's

The bar above every page (`tabs/PageNavBar.tsx`) is laid out and drawn
only by core. Left to right: back/forward, the list stepper (↑ ↓ "N of
M"), the title, then the bookmark star, the comment navigator, the page's
**primary and prominent actions** as buttons, and one **☰ menu**
(`tabs/PageMenu.tsx`, `page-nav-menu`) holding everything else the page
can be acted on with — so the bar shows how a page is navigated, and what
it is acted on with is in one place. The ☰'s order is fixed
(`tabs/pageMenuItems.ts`), sections split by separators, an empty one left
out and the ☰ itself hidden when it would be empty:

1. **oxplow's own, always first and in their own order**, so they sit in
   the same spot on every page: **Ask the Assistant** (`menu-item-page-ask-assistant`, icon `sparkles`: opens the
   dock's Assistant about the page; the catalog's prompts about the page's kind are the
   panel's chips), then **Add to Context** (the page's ref and title go in the selected
   thread's basket; `menu-item-page-add-to-context`), then the page's
   `standard` actions (a lens page's Refresh when it declares `refresh: true`,
   Copy);
2. **the page's and its extensions' own, sorted by label** (case and
   accents aside): each other action (`menu-item-action-<id>`; a busy one
   says `label…` and is greyed), and each submenu by its name
   (`menu-item-page-submenu-<name>`), which holds the actions whose
   `submenu` and the commands about the page's resource whose `ui.group`
   is that name (`menu-item-ref-command-<id>`), merged and sorted;
3. **Open With** (when two or more pages render the URI);
4. **Backlinks (N)…**, **Outbound (N)…** and **Pin to Dashboard…**, which
   open their panel under the ☰ (`page-nav-backlinks-popover`,
   `page-nav-outbound-popover`, `page-pin-menu`) rather than a submenu.

An item may draw an icon (`MenuItem.icon`, a name in `src/icons.ts`'s
`ICONS`) before its label; a bar button with one (`PageAction.icon`) is
the icon alone, its label the hover (`title`) and its accessible name.

A page says **what** goes in the bar, as data:

- `title` / `usePageTitle`, `chips: PageChip[]` (a chip with `onClick`
  is pressable — a wiki page's freshness opens its report);
- `actions: PageAction[]` (`kit/actions.ts`: `id`, `label`, `run`,
  `primary`, `prominent`, `standard`, `submenu`, `icon`, `disabled`,
  `busy`, `title`). In the bar a
  `primary` or `prominent` action is a button (`ActionButtons`, test id
  `action-<id>`) and the rest are ☰ items; a page that shows no bar draws
  them all in its header, never in the rail. A lens's whole-lens action
  is `prominent`, in a `submenu` or drawn with an `icon` when its manifest
  says so, a custom component's when its `setActions` does; only oxplow
  marks an action `standard`;
- `pin: { tile, onOpenPage }`: what the page would add to a dashboard —
  the ☰'s Pin to Dashboard… (`menu-item-page-pin`). A lens page pins its
  lens.

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
   row and the header (`open` is the `comments` page) opening the Comments inbox.
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
   (`goToRef()`, the `go-to` page) for the full hub + bookmark management.
   With a query, pages rank by `pageRank` (tsk1031): every token starting
   a word of the title, then the title as a subsequence, then the id, then
   the `keywords` — where each token must start a word, since a lens's
   keywords hold its whole description and a short subsequence ("todo")
   is in almost any description.

## The dock

The dock is the row of panel icons at the left of the bottom bar. The
background-task indicator stays at the right. An icon opens its panel
above the bar. The Alerts bell is the Alerts panel's icon.

- **Code:** `apps/desktop/src/components/Dock/`. `Dock.tsx` is the bottom
  row plus the panel. `DockBar.tsx` draws the icons. `DockPanel.tsx` draws
  the open panel. `dockStore.ts` holds the state. `dockPanels.tsx` holds
  the registry.
- **Store:** `{ open: id | null, height }`. At most one panel is open.
  `toggle`, `open`, `close` and `setHeight` change it. The height is
  clamped to 160px or more and kept in `shellStorage` under
  `oxplow.dock.height`. The panel also keeps it within the column height
  minus 80px.
- **Registry:** a `DockPanelDef` is `{ id, title, icon, useBadge, Body }`.
  `icon` is a name from `ICONS`. `useBadge` is a hook that returns a count
  and an optional `danger` tone, or null. `Body` is mounted only while the
  panel is open. Core panels call `registerDockPanel`. Their order is
  `CORE_ORDER` (assistant, context, alerts). Extension panels with
  `dock: true` follow, sorted by title. Their body is the panel's lens and
  their badge is its count lens. `useDockPanels()` returns the whole list.
- **Layout:** the panel is absolute and anchored above the bar, inside the
  center column. The page does not reflow. Undo toasts sit above an open
  panel.
- **Rail:** a panel with `dock: true` is not in the rail.
- **Test ids:** `dock-bar`, `dock-<id>`, `dock-<id>-badge`, `dock-panel`,
  `dock-panel-<id>`, `dock-panel-resize`, `dock-panel-close`. The Alerts
  body has `alerts-panel`, `alerts-panel-item`, `alerts-panel-review`,
  `alerts-panel-empty` and `alerts-panel-open-page`.

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
`oxplow/lens` pages mounted through slots ([extensions.md](./extensions.md)).

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
    block), kept live via `subscribePageVisitEvents`. Visit rows keep
    only the id, so `buildRecentEntries` rebuilds each openable
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

- **Tasks** (the `tasks` page, a foundation lens page) — the thread's items by state,
  or its stream's or everything's (the `show` filter); the last five done and a "View all done →"
  link to Done Work; the agent's open turn is a live row in In progress. Ref helper is `tasksRef()`.
- **Done work** (the `done-work` page, a foundation lens page) —
  full descending list of the current thread's closed items (done and canceled).
- **Backlog** (the `backlog` page, a foundation lens page) — the active list's items on no thread
  (`v_work_item.thread_id IS NULL`); `oxplow.work_item.move` takes one to a thread and back (its
  Bring to this thread action needs the `lists` feature).

Tasks and Backlog read `v_work_item` and offer only what the active list can
do (`v_capability_provider`): drag reordering with `ordering`, Send to
backlog and Bring to this thread with `lists`, epics with `hierarchy`,
Delete with `delete`. Rows show the list's editable enum fields (oxplow's
priority). There's no Archived page: oxplow's archived tasks are done or
canceled, as the interface states them.

The pages reuse the shared `<Card>` + `cardLinkButton` from
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
| URL = path + query + fragment | an entry's `ref`: identity (the `?`, what it shows) and view (the `#`, how it shows it); `PagePayload.params` holds only the `?` params |
| `history.state` | `Entry.historyState` (persisted when it is JSON) |
| the page you came from a list of | `Entry.from: Origin { title, items: [{ ref, label }], index }` |
| pushState / replaceState | one `navigate(s, slot, entry, { replace })` |
| back-forward cache | `bodies(tab)`: hidden bodies stay mounted, keyed by identity |
| a SPA re-rendering on popstate | **every page is live**: one body per page identity, keyed `slot/identity`, so entries that differ only in the `#` re-render it — no remount, no Loading |

**Operations** (pure, tested in `session.test.ts`): `open` (finds the tab
showing that **identity**, else a new tab; active. The same page at another
view — a diff at another line, Board with another filter — moves the tab
showing it to that view), `navigate` (the one move: pushes and clears
forward; `{ replace: true }` takes the current entry's place. The same
identity in another view keeps the page: its state, origin and mounted
body. A same-page `replace` rewrites the entry in place and keeps forward
history, so a typed filter leaves Back and Forward alone. A page shown in
another tab closes there), `setHistoryState`, `patchHistoryState`,
`back`, `forward`, `stepOrigin` (Up/Down in place, history untouched),
`close` (the most recent other tab becomes active, as a browser does),
`dropCurrent` (the page's record was deleted: back, dropping the entry,
else close), `setStrip` (the strip as a list of refs: agent-session
reconcile, a new thread's tabs, an agent's `oxplow.tab.*` in another
thread), `reorder`, `activate`, `evictions` (`MAX_PAGE_TABS` = 15;
never the active, a pinned or a dirty file's tab). There is no
`setView`: a view change is a `navigate` to the same identity.

**A page changes its view through `PageNavigation.setViewParams(view, { push })`**
(the whole set, defaults left out) or `patchViewParams(patch, { push })`
(a merge; `undefined` drops a key). Both write the `#` of the tab's
current entry through `navigate` (`withViewParams`, any kind of ref) and
`PageNavigation.viewParams` reads it back (`viewParamsOf(ref)`). They
replace unless `push`: a filter replaces (typing a search adds nothing to
Back); a selection — a param a column `select`s — pushes, so Back
un-selects and Forward re-selects. Core decides by the param's role; no
page chooses. A change of what the page shows — another `?` — is
`withResourceParams(ref, params, view)`: another resource of the same page.
Only the body drawing the tab's current entry gets these.
`PageNavigation.ref` is the entry's ref the body draws.

**Names say the scope**, the same in core, `PageNavigation`, the
component bridge, the docs and the skill: **view params** (the `#`: how
this tab shows the page), **history state** (this step of this tab's history), **page
memory** (this page, for this person, every tab) and **app preference**
(the whole app, for this person).

**Origins.** A list passes an `origin: Origin` (`{ title?, items:
[{ ref, label }], index }`) to `useRouteDispatch` / `navigate`; the host
stores it as the entry's `from` (`originOf`: the index snapped to the
destination's id), and the nav bar's Up/Down and "N of M" read it back
through `PageNavigation.origin` and `stepOrigin(index)` (the nav bar's
test ids stay `page-nav-sibling-*`). A diff item's ref is `diffRef(spec)`,
which carries its spec. New-tab paths drop it. Back/Forward restore each
entry's own origin. Adopted lists: `BacklinksList`, `CommitList` / `CommitGraphTable`
(`CommitRowDispatcher`), and every lens table or list's link column
(`lens/linkOrigin.ts`). That one is **live**: the origin keeps no items, only
`live: { lens, params, column }`, and `LiveOriginResolver` (`tabs/liveOrigin.tsx`)
runs the lens again, as a lens page does, and hands App the column's pages in row
order. `resolvedOrigin` finds the current page in that list for the nav bar and the
keys; `stepOrigin(s, slot, index, items)` steps through the resolved list. A page
the lens no longer lists has no Up/Down. `WorkGroupList` rows open an edit
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
`#…`) — a filter or a pick isn't a new visit, so Recents lists the page
once (V102 emptied `page_visit` when the identity rule changed). A
bookmark is of the full URI, view included: change a filter and the star
goes off; set it back and it's on. The same page
(`identityOf`) starred at another view shows a coloured outline star ("Bookmarked
at other values") whose popover offers **update that bookmark**
(`bookmarkedElsewhere`, `moveBookmark`: one `oxplow.command.sequence` of remove +
set at the same scope, so one undo).

**Persistence.** One key, `oxplow.layout.v7.sessions` (`{ [thread]:
Session }`, written on every change, read on start; corrupt text reads
as none). It holds every tab's history and the active tab, so a restart
comes back to the same pages at the same params. A diff's spec rides in
its ref (`diffRef`), so diffs survive too. An open file's buffer is
read again when its tab's page mounts; unsaved edits don't outlive the
window (`editor/buffers.ts`).

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
  `pageId`, its `page` (the identity, the id without its `#`) and `view`
  (the `#`, decoded — `uri::build::view_of`; an agent never parses an id), its tab's
  `back`, `forward` and `origin`, and, for a lens, the lens re-run as
  text. Detail comes only from a window showing that thread's page. Two windows never
  collapse into one "open page".

Both are read-only: nothing an agent reads activates a tab or a window.

## Linking between tabs — single chokepoint rule

**A URI opens as itself; how is the click's.** `tabs/refOpener.ts`
(`refTarget`; `refTargetNow` when no row is needed) is the one function
that decides: a stream's or thread's URI selects it, every other URI
opens as itself (a tab whose id is the URI, drawn by its renderer), and
a string that is no URI errors ([resources.md](./resources.md) "The
desktop"). A URI nothing renders opens on the fallback `resource` page. **A link to a URI opens it at the click**
through `tabs/openRef.ts`: `openRef(ref, how, host)` resolves it with
`refTarget` and does what it says through the window's `RefHost` — a tab
(in the page's tab or a new one), a thread to select, a stream to show,
or an error naming why not. `PageNavigation.openRef` is that for a link
inside a page (`App` builds it per tab); outside a page (the rail, the
palette) it is the window's opener, `openRefInWindow`. Wikilinks
(`linkRef`), lens links, timeline entries, backlink rows, a component's
`navigate`, search hits and `oxplow_navigate` all take this path.
`refFromTabId` parses a **tab id** (history, bookmarks, a restored tab)
and returns null for text that is neither an oxplow URI nor an
allowlisted URL. Never add a per-surface switch on a URI's type to
decide where it opens, and never build a `TabRef` from a string of
unknown origin: hand the string to
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
`onClick` call `ctxNav.navigate(ref, { newTab })`. (`navigate(uri, { newTab, replace })`
is the one move; the default is the shown tab, pushed.) Falling back to
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

A tab keeps **one mounted body per page identity** (`bodies(tab)`), so a
change of the `#` — a filter, a pick, Back or Forward between them —
re-renders the page and never mounts it again: its state, its scroll and
its loaded data stay. A page therefore reads its view from its props or
`PageNavigation.viewParams` on every render; a `useState(fromProp)` seeded
once is a bug.

**A page's identity is its ref without its `#`** (`identityOf`). One rule
for every page, with no table: the `?` is identity, the `#` is view.
`BUILT_IN_VIEW`, `pageIdentity` and `withView` are gone. A built-in page
that has a filter (Tasks' scope and chips, Metrics' range, Git History's
`q`, Files' `filter`) keeps it in the `#` — and so does a custom page
(Explore Data's `model`, `sql`, `viz`); its
`?` is only what it shows (a diff's path and sides, a dashboard's id, a
metric's key), so two diffs in a tab are two pages. A param written in
the wrong place is not read from there: a lens whose `?` holds a
`filter` or `selection` param draws a `lens-error` box
(`misplacedViewParams`). Page memory is scoped by `keyIdentityOf` (the
identity without its `?` params: `oxplow:oxplow/page?page=tasks`) for
a page, or `resource:<ext>/<name>` for a resource page.

## Page state: four homes, one name each

Every piece of state a page keeps has one home, and its name says its
scope — the same name in core, `PageNavigation`, the component bridge,
the docs and the skill:

| Name | Scope | Home | API |
|---|---|---|---|
| **view params** | this tab's view: shareable, bookmarkable, agents see it; a filter replaces the entry, a pick pushes one | the `#` of the entry's ref | `nav.viewParams`, `nav.setViewParams(view, { push })`, `nav.patchViewParams(patch, { push })` |
| **history state** | this step of this tab's history: scroll, open sections, a draft | `Entry.historyState` | `useHistoryState(key, initial)` |
| **page memory** | this page, for the person, every tab of it, across restarts | `ui_preference`, scoped by `keyIdentityOf` (the page's URI without its view and its non-key params: `oxplow:oxplow/page?page=tasks`, `oxplow:oxplow/lens?lens=x/y`) or `resource:<ext>/<name>` | `usePageMemory(key, initial)` (scope from the page) |
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
| `useViewParam(key, initial)` | a filter (a range, a search, a mode) | the `#`, replacing the entry; typed back by `initial`'s type; dropped at `initial` |
| `useViewSelection(key)` | a pick (a row, a commit) | pushes when set (Back un-selects), replaces when cleared |
| `useViewList(key)` | several values (authors, states) | comma-joined, as a filter |
| `useCollapsedSet(key, initial?)` | open/closed groups (`ids`, `has`, `toggle`, `set`) | history state |
| `useDraft(key, initial)` | an unsaved form | history state; `clear` on submit |

A filter inside a part that many pages host (the changed-files tree's
search, `HierarchyView`) is history state, not a view param: it is a small
narrowing inside the page. New Session's `q` is a view param because it is a
built-in page whose list is the page.

They read `nav.viewParams` and write through
`nav.patchViewParams(patch, { push })`, which merges into the tab's
*current* entry inside the session update (`undefined` drops a key), so
two changes in one tick both land. A key goes to the `#` and never
changes the identity, so nothing is refused. A list's cursor (↑/↓ before a
pick) is the list's own (`useListNav`); only a pick is a selection, and
`useListNav`'s `selected` puts the cursor on a kept pick.

**A component inside a page shares the page's view only where its host
says which keys** (`lens/hostedView.ts`; `.context/extensions.md` "Whose
view a component has"): a replacement the keys its target's contract
lists (the history graph: Git History's `q`, `author`, `branch`, the
same ones core's commit list reads), a lens page's component the lens's
params. Its history state is its own object in the entry
(`component:<scope>`). A tile or a slot gives none.

**A page never navigates to itself, and a mode is a param, not a page
name.** Settings' areas, Diff View's endpoints and Local History's
grouping and "Show every snapshot" are view changes on one page (pushed when Back
should return), so the page stays mounted and its other state stays.

Core pages follow the same rule: the Tasks page's scope and filter chips
are its view params (`components/Plan/tasksView.ts`: `scope` with
`stream`/`thread`, one `f.<field>` per filtered field), so two Tasks tabs
can differ, a scope or chip change replaces the entry, and a reload or a
bookmark keeps it.

## Every tab holds a Page

Every tab is a session tab, whatever it shows (a wiki page, a file, a
diff, a work item, a lens, a commit, an agent session, a web page, …). The
builder in `App.tsx` (`centerTabs`) takes each tab's `bodies`, draws
each with `renderTab` — a route through `routeTab`, anything else
through its renderer ("Renderers"), so there is no per-kind table to
extend — and wraps it in a `PageNavigationContext`. `nav` is the
slot's (`navOpen`, `navOpenFile`, `navOpenDiff`, `navRevealCommit`), so
a hidden back page that navigates moves its tab as the visible one
would. `handleOpenPage` special-cases `agent_session` (activate its tab,
else its transcript), `symbol` (its file at its line), the working
tree's `file` (its buffer read first, so a path that isn't a file opens
no tab and says so) and the diff page carrying a spec; every other URI
is `open`.

- A working file's content and unsaved edits are its **buffer**
  (`editor/openFiles.ts`), shared by every tab of every thread showing
  it; the tab is the session's. A buffer lives while some tab's history
  holds the file (`buffers.retainOnly`), so closing one thread's tab
  keeps another's edits.
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
page"); a new kind of thing is a resource plus a `pages[].for` renderer.
**Don't** add a parallel tab track or keep tab state outside the
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
   worth it only when the params are typed.
3. Where the kit can't draw it yet, grow the kit. A new built-in page
   (`component: oxplow:<name>`) is not an option: the golden refuses an
   added name.

A new **kind of thing** (its tab id is its URI) is a resource plus a
renderer, with no desktop code:

1. Declare the resource in the extension (`resources:`,
   [resources.md](./resources.md)), so its URI has a type, params and a
   label.
2. Add a `pages:` entry with `for: <extension>/<name>`, optional
   `accepts`, `default: true` (the extension's own resource, one page
   per resource) and `pinned`. A lens renderer declares a `uri` param
   and is run with the URI; its other params that the URI carries get
   their values from it. [extensions.md](./extensions.md) "Pages" has
   the rules.
3. Plumb any list rows that target the thing through
   `useRouteDispatch` / `RouteLink` so plain-click navigates in-tab.
4. Backlinks come from the persisted `page_ref` graph; nothing to
   register.

Do **not** introduce a parallel state slot for the new tabs
(no `fooTabs` array, no look-aside registry): what a page needs rides
in its URI or its entry's `historyState`, and its tab is a
session tab (`tabs/session.ts`).

## When to update this doc

- Add a kind of thing: declare its resource and a `for` renderer; add a
  row to the id table above only if it has a notable id format. A
  declared page needs no change here unless it's oxplow's own (list it
  under "Declared pages").
- Add a new rail HUD section: document the data source and trigger
  conditions.
- Replace a legacy panel with a Page-wrapped renderer: tick the
  migration status row above and link the new page module.
