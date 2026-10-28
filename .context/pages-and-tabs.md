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
  (the `new-session` page), `none` opens nothing, and an
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
  opens its transcript page, not a missing tab. The picker says nothing
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
| `apps/desktop/src/tabs/Page.tsx` | Shared page chrome: title + kind chip + status chips + actions slot, optional **browser-style nav bar** (back/forward + bookmark + backlinks dropdown — auto-mounted from `PageNavigationContext` when present), body, collapsible legacy Backlinks region. Title can be passed as a `title` prop or registered programmatically by the page via `usePageTitle`; the chrome falls back to the context title when `title` is omitted. Every page shows the nav bar when it has a navigation context (a page outside one — a test — gets the plain header instead); a page doesn't opt out. `titleIcon` draws before the title (an agent's logo). **`titleInBody`** (default false) says the page renders its OWN title as an `<h1>` in the body (use the exported `pageH1Style` for consistency), so the chrome (nav bar / header) suppresses the title to avoid a duplicate — the tab-strip label via `usePageTitle` is unaffected. Adopters: `MetricDetailPage`, `DiffViewPage`. **The body** takes `width?: "full" \| "reading"` and an independent `rightRail` (a 320px sticky control panel beside a body of either width); below ~960px the rail moves inline to the top of the body (`RailBody`) — "Body layouts" below. **Actions** are `PageAction[]` drawn in the nav bar, and `pin` adds Pin to Dashboard — "The nav bar is core's". Reads only semantic CSS variables (skin via theme). |
| `apps/desktop/src/tabs/PageNavBar.tsx` | Dumb nav-bar component: back/forward, the list stepper, the title, the bookmark star, the **comment navigator** slot (`comments` ReactNode), the primary and prominent actions as buttons, and the ☰ (`PageMenu`). Mounted by `Page` when context or explicit `navBar` prop is present. |
| `apps/desktop/src/tabs/PageMenu.tsx`, `tabs/pageMenuItems.ts` | The nav bar's ☰: one `ContextMenu` with everything else the page can be acted on with; `pageMenuItems` (pure, unit-tested) says what and in which order, `PageMenu` draws it and the panels it opens under the button (Backlinks, Outbound, Pin to Dashboard). "The nav bar is core's". |
| `apps/desktop/src/components/Comments/CommentNavigator.tsx` | Self-contained per-page comment navigator for the nav bar. `useCommentsForTarget(target)` (one URI) → shows "Comments (N)", steps through anchored comments with ◀ ▶ (each `requestCommentReveal`s to scroll + open inline on the surface), and a dropdown lists all comments plus an **Orphaned (M)** section. Orphaned entries are clickable too: they `requestCommentReveal`, and the surfaces open the thread popover at a fallback position (no anchor to scroll to) so the comment is readable. The popover then offers a **"Relink to selection"** button (`CommentPopover.onRelink`, wired by RichTextField + MonacoCommentLayer) that re-attaches the comment to the editor's current selection — select the intended text first. (The right-click "Relink orphaned" path still exists.) Comment-bearing pages pass it via `Page`'s `commentsNav` prop, on their URI (the editor page → the file's, WikiPage → the wiki page's, WorkItemPage → the item's); a page file with `comments: true` gets it from `PageHost`. Renders nothing when the page has no comments. Pure helpers (`partitionPageComments`, `stepComment`) live in `pageCommentNav.ts` (unit-tested) and are shared with the per-thread stepper: `CommentPopover` takes an `onStep(dir)` prop so an open comment thread shows ◀ Prev / Next ▶ buttons that scroll to + reopen the adjacent comment (wired by RichTextField + MonacoCommentLayer via `stepComment` + `requestCommentReveal`). |
| `apps/desktop/src/tabs/PageNavigationContext.ts` | React context exposing `{ navigate(ref, { newTab? }), goBack, goForward, canGoBack, canGoForward, setTitle, title }` to descendants of an active page tab. Wrapped around every non-agent center tab in `App.tsx`. `BacklinksList` reads it so default-click navigates in-tab. The `usePageTitle(title)` helper registers the page's current title with the host so the same string drives the chrome header AND the tab strip label — no per-page duplicate header markup. **`useTabEditing({ dirty, save })`** tells the tab its page holds unsaved edits and how to save them (`setEditing`, on the current page only): `App.tsx` keeps one setter per tab (`editingSetterFor`) and draws `CenterTab.dirty` as a ● before the label, publishes `pageDirty` (the File → Save `when`), runs the shown tab's `save` from `oxplow.editor.save` when the page isn't a file, and keeps a dirty tab from the cap's `evictions`. The editor (`FilePage`) and the `markdown` viz's editor call it. |
| `apps/desktop/src/viz/EditorViz.tsx` | The `editor` viz that draws the working tree's file (foundation's `editor` page): its buffer from `editor/openFiles.ts`, `EditorPane` over it, `useTabEditing` for its ● / Save / Find. A renderer's page (`PageHost` with `renders`) shows the resource's backlinks and outbound links in the nav bar (`useBacklinks`), so wiki pages, tasks and commits that reference the file are listed there. `.context/editor-and-monaco.md` "The editor viz". |
| `apps/desktop/src/tabs/RouteLink.tsx` | Browser-style link button + the `useRouteDispatch(ref, { onNavigate?, pinnedSlot? })` hook that powers it; `ref` / `to` is a `TabRef` or a ref string (opened through `openRef`). Click semantics: left-click → in-tab navigate via `PageNavigationContext` (or `onNavigate` fallback when no context, e.g. rail / palette), Cmd/Ctrl-click + middle-click + right-click → new tab. The hook returns `{ dispatch, handlers }` so non-button rows (file tree entries, note rows, …) can adopt the same semantics without becoming a `<button>`. |
| `apps/desktop/src/components/RailHud/RailHud.tsx` | Persistent left rail HUD: a pinned search trigger at the top, then a set of **uniform collapsible panels** (every one an extension's — Go To, Uncommitted, Comments and Work from `oxplow-bundled`). Each renders through the shared `RailSection` wrapper — drag handle (⠿) + expand/collapse chevron + title + optional count badge + optional header action. (What needs the person — proposals, failed operations, undelivered events, firing badges — isn't a rail section: it's the dock's Alerts panel, the Alerts page and toasts; see `.context/usability.md`.) Sections **drag-to-reorder** (MIME `application/x-oxplow-rail-section`, the layout — order, collapsed, hidden — persisted in `panel_layout`; see "Left-nav panels" below). An extension panel with a `collapsed` page (Work) keeps that summary when collapsed; the rest hide their body. Every section **always renders** (stable list) with an empty-state line ("Working tree clean", "No open comments", …) when it has no content. Passive — never auto-opens tabs. Go To is the `oxplow-bundled` panel `go-to` (bookmarks, then recent or most visited pages). |
| `apps/desktop/src/components/Navigator.tsx` | Far-left combined **stream + thread navigator** (each stream's open threads — `get_thread_state` leaves closed ones out; they're on the Closed Threads page): a 40px always-visible strip of letter glyphs (`navigator-strip-stream-<id>` / `navigator-strip-thread-<id>`) — a stream is a tab in the guide line's grey (`--text-muted`) with dark text (`--text-on-stream-tile`), coming out of the window's left edge and rounded on the right only, with its threads as tabs under it, all flush with the left edge and butted up against each other, each filling its shorter row and stopping short of the right (in the centre tab strip's header tint, `--panel-header-bg` over `--surface-card`, like a regular tab; the selected thread's accent bar is inside its tab, down its left, not beside it; a marked thread shows its mark as a corner fold on its top-left in `var(--thread-mark-<colour>)` (`navigator-thread-mark`, title "Marked <colour>"), not an outline; stacked tabs are set apart by a 1px divider along each one's bottom but the last's, in the guide line's grey (`--text-muted`), so none falls under the stream's tab, and the strip's column is `--surface-app`); a grey guide line runs down the column's right, flush with the stream tab's right edge, from under the stream's tab, with a tick from each thread's tab into it, ending at the last, each thread's agent status dot a node where the tick meets the tab; a stack's corners are rounded only where no tab sits next to it (the stream's top, the last tab's bottom), the rest flat; the strip draws no line against the HUD rail; in the expanded panel each row's tab runs on across the row carrying the title, set off from its abbreviation by a divider line through every row and a gap (`IconColumn wide`, `IconCell label`), so the tab's colour flows into the label, the glyph kept at its strip width so the two line up; the panel's rows draw the same icon column (`IconColumn`) so the two line up; clicking a glyph navigates directly — a thread glyph selects that thread, a stream glyph switches streams via `onSwitchStream`. **Hovering a glyph shows its full title as a native tooltip and nothing else.** The `navigator-overlay` panel (~280px, covering the strip *and* the rail HUD to its right) expands only on an explicit click: the bottom-pinned `navigator-expand` chevron, dead space in the strip (`navigator-strip-empty`, `e.target === e.currentTarget`), a stream glyph (which also switches to that stream, then shows its threads), or the already-selected thread's glyph (selecting it again would do nothing). The chevron is pinned *outside* the scroll container and is the load-bearing affordance — once the list scrolls there is no dead space left, and `+ Add stream` lives only in the panel. The open/close state machine — click-to-open, geometric pointer-leave, Escape, outside-press, background-click, and the passive-vs-explicit guard — lives in the shared `apps/desktop/src/components/useSlideoutStrip.ts` hook (chevron in `SlideoutChevron.tsx`), which the Terminal page's `TerminalTabStrip` also runs on; a new slide-out strip adopts the hook rather than re-deriving it. It dismisses on a click in its own **dead background** (anything resolving to `button/input/select/textarea/a/label/[role=button]` is left alone so controls behave normally), on the pointer leaving its bounds (180ms grace), on Escape, and on any pointerdown outside it. The last two are *explicit* dismissals and beat the mid-rename / mid-new-thread form guard; the first two don't. **Pointer departure is measured geometrically** (document `pointermove` vs. the panel's rect), never `mouseleave` — the panel covers the rail and sits in the same DOM subtree as the strip, so the pointer never "leaves" the wrapper while parked over the covered region, which is what used to strand it open on top of the rail and swallow clicks meant for it. Each overlay row (`navigator-stream-row-<id>` / `navigator-thread-row-<id>`) — and each strip glyph, with the same menu — opens its action menu on **right-click** (Menu key / Shift+F10 for keyboard), headed by the stream's or thread's name (`context-menu-header`); Rename and Add thread open the panel for their inline field. **Thread menu:** **"Mark with…"** (`menu-item-thread.mark`) opens a submenu headed by the line "A colour on the thread's tab, for grouping threads your way. It changes nothing else." (`context-submenu-header`, the generic `MenuItem.submenuHeader`) and lists `amber`, `red`, `green`, `blue`, `violet` (each with a colour chip, `MenuItem.swatch`) and None as `menu-item-thread.mark.<colour|none>`, the current one checked; picking one runs `oxplow.thread.mark` (via `onMarkThread` → `App.handleMarkThread`). Then Rename / Settings / Close. A mark only groups threads the person's way; every open thread of a stream shares its worktree. |
| `apps/desktop/src/tabs/bookmarks.ts` | The person's bookmarks, project data (data-model.md "bookmark"): `useBookmarks(threadId, streamId)` reads `v_bookmark` — a page once, at the narrowest scope the thread sees (thread / stream / project) — and `setBookmark` / `removeBookmark` run `oxplow.bookmark.set` / `oxplow.bookmark.remove`. Pages bookmark via the `PageNavigationContext.bookmark` binding (one `scope`; the star's menu moves it or, on its current scope, takes it off); the Go To page's bookmark row actions re-scope and remove. |
| ~~`apps/desktop/src/tabs/appPageBacklinks.ts`~~ | **Deleted.** Per-kind in-memory backlinks providers used to live here. Cross-page backlinks now come from the persisted `page_ref` graph (`crates/oxplow-db/src/page_ref_store.rs`) via the `list_backlinks` IPC; every page kind goes through the same code path. App pages that need their own provider would register a new `source_resource` writer in the backend instead. |
| `apps/desktop/src/components/RailHud/sections.ts` | The launcher's categories (`PAGE_CATEGORY_ORDER`, `PageDirectoryEntry`) ; every launcher page is a declared page (`components/extensionLauncher.ts`). |
| `apps/desktop/src/components/History/CommitGraphTable.tsx` | Pure presentation of the git-log graph (branch/merge dots + lines + sha + ref badges + subject + author + relative date). `indexRefsBySha(log)` exported alongside groups branch heads + tags by sha so callers feed identical maps. |
| (change analysis) | No page of its own: `crates/oxplow-app/src/change_analysis.rs` stores each change behind `v_change*`, and the oxplow-bundled `change-review` page shows it, mounted at the `oxplow-foundation/diff-view.sections`, `commit.sections` and `uncommitted-changes.sections` slots. The old per-page scope drilldown is gone; scoped tab ids reopen the unscoped page. |
| ~~`apps/desktop/src/tabs/backlinksIndex.ts`~~ | **Deleted.** The in-memory cross-kind indexer is replaced by the persisted `page_ref` table; see `data-model.md`. The `BacklinkEntry` type that renderers consume now lives in `apps/desktop/src/tabs/backlinkTypes.ts`. |
| `apps/desktop/src/tabs/useBacklinks.ts` | React hook that calls the unified `list_backlinks` IPC for a `TabRef` and maps the returned `BacklinkEdge` rows into `BacklinkEntry`s. Used by every page kind including the editor page. The sibling `usePageOutbound` hook does the same for the inverse direction. |
| `apps/desktop/src/tabs/backlinkTypes.ts` | Renderer-side `BacklinkEntry` interface (`{ ref: TabRef \| null, target, label, subtitle?, dead?, linkType? }`): `ref` is the page the row opens, null for a dead link (nothing resolves the ref; `BacklinkEdge.resolved`) or a resolved ref with no page. Decoupled from the SQLite `BacklinkEdge` shape. |
| `apps/desktop/src/tabs/BacklinksList.tsx` | Default renderer for the Page chrome's `backlinks` slot — buttons that route via `onOpenPage`; a row with no page renders muted as its raw ref (dead link). `entryMenu` supplies a row's right-click actions (a work item's link rows: Unlink). |
| `apps/desktop/src/components/Analytics/DailyBarChart.tsx` | Generic daily bar chart (`{ label, value }[]`); renders `bar` vizzes. |
| `apps/desktop/src/components/CollapsibleSections.tsx` | Collapsible page sections. Two parts plus a hook: `CollapsibleSections` (state provider), `SectionCollapseControls` (the "Expand all" / "Collapse all" pair), and `useSectionCollapse`, which the kit `Section` (`collapsible`, `defaultCollapsed`; the toggle is a button inside the `<h2>`) reads. Sections register themselves on mount so the controls know what "all" means — only what's **currently rendered** counts, so a filtered-out section is never silently expanded — and the controls self-hide when nothing is registered. **The page places the controls**: Recorded Metrics renders them in the **details rail** beside its filters, which works because the provider wraps the whole `<Page>` and `rightRail` is *rendered* inside Page's subtree (context follows the render tree, not the creation site). For that to be safe the provider renders `children` **bare** — a wrapper element there would break the page chrome's `height: 100%` column. Collapsed ids are the tab entry's state (`useHistoryState`); default is expanded, and stored ids are deliberately **not** reconciled against the rendered set (a section hidden by a page's search must return still collapsed). Pure state/persistence in the sibling `sectionCollapse.ts`. **Not a `Page` prop** — see `.context/usability.md` → "Collapsible page sections". Adopters: none now. |
| `apps/desktop/src/components/Slideover.tsx` | Right-edge panel primitive (~38vw, backdrop-click + Escape close, focus-into-panel on open) for form-shaped flows that don't justify a full page. Use instead of a centered modal. |

## Renderers

`PageTree` passes fill height through stack and selected switch containers
when their visible descendants contain a filling viz. Keep that flex chain
intact: the agent-session page stacks approvals above its terminal, and a
collapsed terminal cannot activate or resume its session (`useShown`). The
browser regression checks usable terminal height before and after reload.

A URI names a thing; a **page** is one **renderer** of it. A tab's id is
the URI it shows, and nothing in the desktop keeps a list of page kinds.
Which page draws a URI comes from the page files' declarations
(`pages/renderers.ts`, `chooseRenderer` in `pages/pageFiles.ts`, "The
desktop host"):

1. The pages whose `for` names the URI's resource (an oxplow resource
   `<extension>/<name>`, or the scheme `https` / `http` for a web page)
   and whose `accepts` hold for its params.
2. The view key `#oxplow.renderer=<extension>/<page>` picks among them.
3. Else the page with `default: true`; else the first.
4. Else foundation's `resource` page (`pages/ResourcePage.tsx`): the
   resource's label and params, or the raw URI muted as a dead link when
   nothing declares the resource or its dataset has no row. Never an
   error, never a toast.

**Open With** in the nav bar's ☰ (a submenu, `menu-item-page-open-with`,
each renderer `menu-item-page-open-with-<extension>/<page>`, the one
drawing it checked) lists the renderers when there are several. Picking one is a view change
(Back returns to the first). `App.tsx` `renderTab` draws a route
(`oxplow:oxplow/page?page=…`) through `routeTab` (its page file, else
`MissingPage`), an ephemeral view (`oxplow:oxplow/view?id=…`) through
`EphemeralView`, and anything else through its renderer's page file, else
`builtInPages.resource` (the fallback, the one built-in). Every page draws
in `PageFileView` / `PageHost`; the desktop reads every page from a
page file. A pinned renderer's tab
(`pinned: true`, the agent session) is pinned; an agent session's tab also
draws its harness's logo, its status dot and what it waits on, from the
window's session list and statuses (`centerTabs`), and a page's actions
(`useTabActions`, `setTabActions`) are its tab's right-click menu. Page memory (history state, scroll) is scoped by
`keyIdentityOf` for a page or metric, and `resource:<resource>`
otherwise. A tab's label and icon come from `recordedPageKind(ref)` /
`kindForTabId`: a route's page id, else the URI's resource
(`oxplow/file`), else a web page's scheme.

Foundation's page files are oxplow's renderers (`agent-session`,
`commit`, `directory`, `editor`, `file-viewer`, `wiki-page`, `work-item`,
`diff-view` for `oxplow/snapshot`, `oxplow/effort` and `oxplow/turn`, and
`external-url`, a page over the `embed` viz, for `https` and `http`). An agent
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
  web page, …): the id is the thing's URI, and a renderer draws it
  ("Renderers").
- **A route** (`oxplow:oxplow/page?page=<id>`): a page file with no
  `for` — not a thing in the graph. Its id is
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
page (a foundation form page, `new-task`) with its `parent` param.

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
| route | `oxplow:oxplow/page?page=<id>[&params]` (oxplow's) / `oxplow:oxplow/page?page=<extension>/<id>[&params]` (`pageTabRef(id, params?, view?)`, `extPageRef(ext, id, params?, view?)`, plus typed wrappers: `diffRef`, `endpointDiffRef`, `duplicateBlockRef`, `settingsRef`, `newTaskRef`, …) — a route; see "Declared pages" below | `oxplow:oxplow/page?page=tasks`, `oxplow:oxplow/page?page=symbols&path=src/lib.rs`, `oxplow:oxplow/page?page=github/open-prs`, `oxplow:oxplow/page?page=acme/pr&ref=oxplow:acme/pr?id%3D12` |
| web page | the URL itself, `https://example.com/docs` (`externalUrlRef(url)` is `{ id: url }`) — rendered by foundation's `external-url` page (a `viz: embed` over the URL), `for: [https, http]`, its tab labelled `{{param.uri|url}}`; see [external-url-tabs.md](./external-url-tabs.md) | `https://example.com/docs` |
| symbol | `oxplow:oxplow/symbol?path=<path>&symbol=<name>` (`v_symbol.uri`, `symbolRef(uri)`) — no revision in the id; never a tab: opening one (`handleOpenPage`, or in-tab navigation) resolves it (`resolve_symbol`) and opens its file at the name's line (`openSymbol`). A pinned one (`rev=snap:<id>` / `rev=git:<sha>`) opens the file viewer at that revision; if the version is gone it shows a notice and opens the current symbol | `oxplow:oxplow/symbol?path=src/lib.rs&symbol=Widget::spin` |
| agent_session | `oxplow:oxplow/agent_session?id=<ses id>` (`agentSessionRef(id)`, pinned; the one URI whose tab is pinned) — an open agent session's terminal; its tab draws its harness's declared logo (`CenterTab.icon` from `HarnessListing.icon`, `AgentIcon`; the generic robot when it declares none) | `oxplow:oxplow/agent_session?id=ses3` |

## Page files

The catalog's runtime publication listener also invalidates every open
page run. Reloading only the extension metadata leaves runs with no SQL
loaders (a dashboard of embeds or a markdown page) on their old definition
after deploy. The existing listener sends `page_runs.notify_everything()`
before `ExtensionsChanged`; it does not add another event-bus listener.

A page is `pages/<slug>.yaml` in an extension, three parts
(`crates/oxplow-app/src/pages/schema.rs`). It is the one unit that
defines what the person sees:

- **Metadata**: `title`, `description`, `label`, `icon`, `category`,
  `keywords`, typed `params` (`role: resource | filter`; a `switch`'s param
  is a filter by default; a param a column `select:`s is a `selection`),
  `for` / `accepts` / `default` / `pinned` (a
  renderer), `comments`, `refresh`, `hidden`, `needs`, whole-page
  `actions`, `alert`, `tool` (an MCP tool `<namespace>_<tool>` whose
  input is the params and whose call is `oxplow.page.run`; kept for the
  few questions agents ask every session — foundation's `thread_work`,
  `work_next`, `effort_summary`, `catalog` — everything else is
  `find_page` → `run_page`), `answers` (`{ question, about?, params,
  skill? }`: what agents find a page by and what `extension test`
  runs) and `badge`
  (`{ loader, column? }`).
- **`data:`** — named **loaders**, in order; a loader reads only the ones
  before it: `sql` (binds `:param`s; an earlier loader reads as a table of
  its name), `command` (a read-only command run as the viewer, `args`
  each `{ param }`, `{ from: <loader>.<column> }` or a literal; it must
  say `reads` (datasets) or `refresh` (`manual`, `{ every }`); `items:
  <field>` takes a list inside its result as the rows, as the `definition`
  page reads `oxplow.page.definition`'s `nodes`), `page`
  (another page's `loader`, with `params`) or `rows` (inline). A loader's
  name is a plain identifier, never a dataset's (`v_*`, `m_*`).
- **Show Definition** is foundation's hidden `definition` page (params
  `of`, the page id, or `spec`, an inline view's exact YAML, and `params`,
  its params as JSON text), from one `oxplow.page.definition` call:
  where the page comes from (its file a link to the file view), its
  description, how it is used (a line each: what it draws, how it is
  opened, its tab, updates, comments, how agents read and find it), its
  params with this run's values, its loaders (what each runs and reads,
  its rows), its node tree (embeds and slot mounts included) and its
  actions where they sit. Only an inline view, which has no file, shows
  its YAML.
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

A viz node's config is a `VizConfig` (`Viz` names the kind; `VizRun`
is what a run holds for it). Its rows are its loader's. The shape checks,
action parsing (`viz_actions.rs`), text renderers (`viz_text.rs`) and the
desktop's viz components all work from it. The YAML only binds: logic is
SQL or a read command. An **inline page** (`parse_inline_page`: a markdown
` ```oxplow-page ` block, an agent's one-off) only reads: no `command`
loaders, no actions, forms, terminals, editors, components or slots.

The shared Rust metadata types are `PageParam`, `PageColumn`, `PageAction`,
`PageAlert` and `PageProvenance`; the viewer's runtime context is
`ViewContext`.

`oxplow extension check` dry-runs every page (`pages/checks.rs`: each
page runs with its example params; layout warnings and column checks
report what would draw badly). Load checks refuse a page file whose
renderer, component, replaceable or params declarations do not hold.
A slot mount whose host page is missing is a warning.

**Ephemeral views** (`oxplow/view?id=…`) route directly to `EphemeralView`
and the same `PageHost`. `open_page` returns the exact `specYaml` with
the run. The subscription store retains it for daemon reconnects, and the
tab keeps it under `view.specYaml` in its entry's history state. On a
restart the daemon uses the restored source if its in-memory view is
gone; nothing creates a persistent answer record. View params still live
in the URI's `#`, and inline source keeps its read-only restrictions.

Every page host offers **Show definition** in its standard actions.
An inline view also offers **Save to wiki…** and **Save as Page…**. The
forms live in the rail (Enter submits, Escape cancels). Saving to wiki
runs `oxplow.knowledge.write_page` with an `oxplow-page` fence, so the
saved page remains live. Saving as a page runs
`oxplow.extension.write_page { extension, page, spec, deploy: true }`,
then opens that page; the extension defaults to app preference
`pages.extension`, else `my-pages`, and a successful save remembers it.
Title edits preserve the YAML document's comments and key order.
Refused writes keep the form and source for retry. A component's shown
inline page (Explore Data) offers Save as Page for any valid node tree,
including inline rows and multiple loaders.

**Running one** (`pages/runner.rs`, `PageRunner::run`): params resolve in
this order (supplied, else the viewer's context, else the default;
`head_sha` the viewed stream's HEAD; a stream change re-runs a page
that declares it). A page whose `needs` (a capability such as
`work_items` or `knowledge`) are unmet runs no loader: its run has
`inactive` with the unmet needs and a message, and the page says so.
A `sql` loader reads the extension's settings through `config()`.
Loaders run in order: a `sql`
loader gets each earlier loader it names as a temp table of that name
(`SqlQuery.temp`, beside `metric_grid()`'s); a `command` loader runs only
a command whose access is a read (`Access::reads_only`), as the viewer
(`Actor::Page { page_id: <page>#<loader>, on_behalf_of }`), its result a `{ columns, rows }` table as is, objects a row
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
(`ui.mounts: [{ page: <slug>, at: <ext>/<page>.<name>, position?, title?,
size? }]`, loaded as `ExtensionUi.page_slots`): each runs as an embed, its
params by name from the host, its node id the mounted page's id with `.`
for `/` (`beta.extra`); by extension name; `position: start` puts it first
among the slot's siblings, `{ before | after: <sibling id> }` beside that
sibling, otherwise it sits at the slot. A **panel**'s parts (`body`,
`badge`, `collapsed`, `count`) each name one of its extension's pages
(a badge page declares `badge` or `alert`; a stream or thread panel's
pages take `stream_id` / `thread_id`), and a **replacement** names a
page (`ui.mounts: [{ page, at: <target>, version }]`,
`UiReplacement.page_id`), checked against the same props contract. Which replacement shows is decided by the page run
(`PageRunner::replacement`: the person's choice, the only candidate, the item's provider), which runs it as the node's embed. `dirty_loaders` is what a change makes run again: loaders that
read a changed dataset (a command loader's are its `reads`) or bind a
changed param, and everything downstream of them.

**Drafts** (`PageRunner::draft(svc, checkout, extension)`): the
extension's source in another stream's checkout, run over live data —
its pages found first, its datasets read through an overlay of temp views
(`extensions::draft_overlay`), its commands whatever is deployed (a
draft's code never acts). `oxplow.page.run { page, draft: true }` reads
the caller's stream's draft; its text starts `Draft of <page> (stream
strN, not deployed)` and its json carries `draft: { stream, checkout }`.

**What the person is looking at**: a window's tab report carries
`detail_json: { runId, focus }` for a page tab (`focus` is `{ node }` when
the tab's `#node=<id>` lands on a node, else null; no row focus is
reported yet), and
`oxplow_get_open_page` returns that open run as `pageRun: { page, title,
params, focus, source, text }` — the person's own run, not a re-run.

**Showing a page** (`oxplow.page.show { page, params } | { spec }`,
the agent's `oxplow_show_page`): a page by id opens at its uri; an
inline `spec` is kept in memory as a **view** (`PageRuns::keep_view`,
`oxplow:oxplow/view?id=<id>`), opened with `open_page { view }` — gone
with a daemon restart. It opens as a tab of the caller's thread when a
window is open (`tabs.write/open`; `opened`, else `why`) and returns the
page's text.

**Open runs** (`pages/open_runs.rs`, `Services.page_runs`): Interest starts
before the first loader read: an invalidation revision is captured before execution
and checked under the same fence as insertion. Dataset/measure invalidations which
reach the run's actual reads retry its initial execution; catalog-wide changes also
retry. Unrelated writes (including records from its own command loaders) do not
starve opening. A later change finds the published run. This closes the query/registration gap without a timed missed-event cache.
A window opens
a run (`open_page { page | specYaml, params, streamId, threadId, client }`
→ `{ runId, run }`) and closes it (`close_page`); a pick, filter or
switch is `set_page_params`, Refresh `rerun_page`. The change loop
(`datasets_changed::spawn`) signals `PageRuns` as it tells the assets
(never a second bus listener): datasets changed, measures landed, or
everything (it lagged); the bus signals the live datasets an event changed
(`EventBus::reach_open_runs`: a keychain write, a config edit). A worker started at boot coalesces signals for
100 ms and re-runs each open run one of whose loaders (embeds included)
read a changed dataset or measure — a command loader's are its `reads` —
then emits `OxplowEvent::PageRunChanged { runId, seq, loaders, tree }`
for each that changed: the loaders whose rows differ (an embed's as
`<embed>/<loader>`), never the rows. A re-run of one run that takes 250 ms or
more logs `slow page re-run` (run, page, loaders, `took_ms`): the place
to look when a page, or the window around it, lags. The window fetches what changed in
one `fetch_page_runs` batch, whose entries are `{ run, seq }` captured
under the same lock. It drops replies older than the actual snapshot
sequence it applied, not the triggering notice's sequence. Direct
parameter/refresh operations fence fetch replies at both start and finish;
an overlapping reply reconciles from a fresh snapshot instead of replacing
newer state (`apps/desktop/src/pageRuns.ts`: runs shared by key, closed
250 ms after the last host lets go, every run opened again after a
reconnect). The window's run store publishes its whole live roster with
`runIds()` / `subscribeRunIds`, including panel and markdown subscriptions.
`TabReport.runs` carries that roster, and its heartbeat reads the ids again
instead of repeating a cached list. Reports use a single-flight writer:
while one is in flight, only the newest pending snapshot is retained, so
an older HTTP report cannot arrive after a newer roster and close its runs.
Runs belong to their window's `client`; reports prune only that client's
previously acknowledged runs which are now absent. The event socket's
`client` query param names the same stable id used by logs and reports
(`uiClientId.ts`, independent of the API/transport import graph), and its
disconnect and client expiry reclaim owned runs. A reconnect clears the
old roster and reopens its held runs; stale open replies are fenced out and
closed instead of resurrecting old ids. A dataset change keeps the loaders it didn't reach:
the re-run is seeded (`PageRunner::run_seeded`) with every loader run —
embeds' and mounts' too — that read none of the changed datasets or
measures and reads no loader that did (`runner::unreached`), so only the
rest query again. A param change or Refresh runs the page whole. Per-run request revisions fence
asynchronous completions: a slow older refresh cannot replace newer params or rows.
The desired request is recorded before execution so concurrent invalidation uses it.

Each returned run has a shared **5 MiB serialized loader-result budget**, including
embeds, rail trees and slot mounts. Admission preserves a prefix of whole rows,
marks cropped results `truncated`, and keeps read/coverage provenance. Downstream
loaders see only admitted rows. Memo entries are independently bounded and every
retained copy counts against the shared budget. This bounds loader results, not
page definitions, params or tree metadata; metadata which cannot fit produces a
bounded diagnostic rather than silently losing provenance.

**Actions** (`pages/actions.rs`, `run_page_action { runId, node?, action,
row?, input?, drop?, confirmed }`): pressed on an open run. A whole-page
action binds the run's params; a node's row action runs on the row the
presser names, which must be one the run holds (matched on the columns
given: a row that changed under the press is refused); a holds-one-row
action on its node's one row, read by the server. The checks and input
binding are `viz_actions::prepare_resolved`, and it runs as the page
acting for the presser (`Actor::Page`). An agent presses with
`oxplow.page.run_action { page, params, node?, action, row? }`, which
holds a run of its own for the press. Forms: `page_form { runId, node }` is a
form node's start (its defaults with params bound, its loader's first
row, its `choices` and object fields' `from` read from the pages they
name: `<ext>/<page>` takes its first loader, `<ext>/<page>#<loader>`
that one), `submit_page_form` submits it as the page for the person, and
`page_action_form { runId, node?, action, row? }` is a form action's
start. A whole-page action may read `{{row.x}}` / `when:`: the first
loader's one row.

**The desktop host** (`apps/desktop/src/pages/PageHost.tsx`). `App.tsx`
draws a tab from its page file first (`pages/pageFiles.ts`):
`chooseRoute` finds a route's page by id (`pageFileOf`: oxplow's — the
`oxplow` namespace, `CORE_NAMESPACE` — by its slug, any other's by
`<extension>/<slug>`), `chooseRenderer` a resource URI's among the page
files' `for` / `accepts` / `#oxplow.renderer=` / `default`. Nothing else
draws a tab (a page's slot shows only the pages mounted there). oxplow-foundation and
oxplow-bundled both have `namespace: oxplow` and share the core page
names, so a slug one of them declares the other may not
(`pages::schema::refuse_shared_core_pages`: the later in load order loses
that page with a load error; bundled's panel bodies are
`comments-panel` and `go-to-panel` for that reason). Open With lists every
page file that renders the URI by name (`openWithOptions`). The tab body is `PageFileView`: its run's
params are the entry's URI (`pageRunParams`: the `?` params — a
renderer's `uri` and its resource's own — and the `#` params the page
declares as a filter or a selection; a param with no role is a resource),
so a view change is a new key and Back/Forward show each entry's run; the
host keeps the last run of the same page on screen, `data-stale`, until
the new key answers. A change the page makes goes back through the tab
(`pageViewChange`: a filter replaces the entry, a pick or
a switch's section pushes, another resource navigates —
`withRenderedParams` for a renderer, `withResourceParams` for a route);
`#node=<id>` is the host's `focus`. Outside a tab (`onChange` absent) the
run takes new params itself (`setParams`). `PageHost` takes a `PageKey`,
holds its run (`usePageRunWith` over `usePageRunsStore()`: the
`PageRunsContext`'s store, else the window's; `RunState` carries the
`runId`, `rerun` is Refresh) and draws the `tabs/Page.tsx` chrome:
Contents and Selection (the rows picked, `picked` history state) in the
rail (`pages/railGroups.tsx`); About this page in the ☰ (a `Popover`
over the body's top right: the page's `description`, `page.origin`, out
of date — the description is never drawn in the body); `comments: true` → the
`CommentNavigator`, `PageCommentsContext` and `#comment=` on the tab's
identity; a renderer's backlinks and outbound links; the stream as
`VizStreamContext` (`viz/VizStream.ts`); full width for a grid, a board or a dashboard; and
the open page's detail `{ runId, focus: { node } }` for
`get_open_page`. A page node's link column's live origin
(`<page>#<node>`) runs the page and reads that node (`liveOriginRun`).
The chrome: the title is `tabLabel` over the run's params and `labelRow(run)`
(the first declared loader's first row), whole-page actions and Refresh
(`page.refresh`) are page actions, pressed through `useVizActions` with
a runner that calls `run_page_action` with no node, so a command that
asks is confirmed. **Copy** (standard, `page-copy`) puts the open run's
text on the clipboard (`page_text(runId)`, through `PageReads.text`);
Add to Context is the ☰'s own. After a press or a form submit on a run
ran, the window runs that run again (`PageRuns.refresh(runId)`, from
`useNodeActions` / `useNodeForms`): a command may change what no dataset
change announces (a keychain entry: a credential, an AI provider's key).
An embedded page's own whole-page actions are buttons above its tree
(`EmbeddedActions`, `viz-action-<id>`), pressed on the host's run naming
the embed's node; an `expand`ed row's page keeps its vizzes' buttons
(`partButtons`) though it is drawn compact. A page a slot mounted is a
section titled by the mounted page's title. A `custom` viz shares the
host's view (`HostedViewContext` from `PageHost`: its filter and selection
params, `pageViewKeys`; a change is the rail's `onChange`), as a
replacement's shares its target's. `PageRail` lists the filter
params (`ParamList`; a change is `setParams`) and draws `run.rail`.
`PageTree` draws a `RunNode` tree (`compact` in a rail panel): a node
another extension's page stands in for (`replacedBy`) draws its embed in `Replaced`; stack → `Section`s, grid → `TileGrid`
at each child's `size`, switch → its one child, viz → `VizView`
(`viz/VizView.tsx`, the one component that draws a viz) over
`nodeRunOf` (`viz/nodeRun.ts`, the server's `node_run`), embed → a
nested tree with ids `<embed>/<id>`, slot → its children, markdown → the
`Markdown` component, component → its extension's component in a frame
at its content's height (`pages/PageComponent.tsx` `ComponentNode`), a
node or loader error → an inline `ox-error`. A component is fed its
page's data over the bridge (protocol 12, `.context/extensions.md`
"Custom components": `pageData` — every loader's rows by name, the
params, its props — and an `update` naming the loaders that changed); a
`custom` viz is fed the same (`VizPages.componentData`), its own rows also
its `run`, and in a page replacement shares the view its target lists
(`HostedViewContext`). A page whose `view:` is one component node *is* the
component (`ComponentPage`, drawn by `PageHost` in place of the tree):
it fills the page, its `setTitle` / `setActions` are the tab's title and
nav-bar actions, the tab entry's whole `#` is its view, and what it hands
over with `showView` (an inline page's YAML) opens as a run of its own and
is drawn under it (`ShownView`, with Save as Page); its `rail:` is drawn
in the rail as any page's. Every node's root
has `data-node` and registers in a `NodeRegistry`; `useNodeFocus` lands
on `#node=<id>` (scroll, `data-focused` for 1.5 s; a shown view's run
never takes `node` as a param). Viz components press
their actions through `VizActionRunnerContext`
(`viz/vizActionRunner.ts`; outside a page run it refuses); `PageTree`
provides `useNodeActions(runId, nodePath)` per viz node. Forms go the same way: `FormViz` and `ActionForm`
start and submit through `FormSourceContext` (`viz/formSource.ts`;
outside a page run it refuses), which
`PageTree` sets per viz node to `useNodeForms(runId, nodePath)`
(`page_form`, `submit_page_form`, `page_action_form`; a form action's
submit is a press with its `input`); `PageHost` draws a whole-page form
action's `ActionForm` with no node. A form's draft is keyed by its node's
id, so a re-read of the run keeps it. What a viz reads of *other*
pages comes from `VizPagesContext` (`viz/vizPages.ts`, set per viz node
by `PageTree`): an `expand` column's row opens the page it names
(`<ext>/<page>`, its first loader, or `<ext>/<page>#<loader>`) as a run of
its own — the row's values it declares (`get_page`; the server refuses an
undeclared param), the named loader's viz node drawn compact — and a lazy
tree's open branch is a run of the node's own page with the lazy param
set to the branch's id (`pages/pageBranches.ts`, `usePageRunsFor`), whose
rows press on that run (`BranchRun.runner` / `forms`: the server only
takes a row its run holds). A filter's `options_from` names a page
(`pageOptionsSource`, `OptionsSourceContext`, the daemon's by default,
`PageRail`'s over its `PageReads`): run once with the values it declares
(`run_page`), its named loader's `value` / `label` / `group` the
options. A live origin (Up and Down through a page node's link column,
`<page>#<node>`) holds the producing page's run (`LiveOriginResolver`
over the window's runs) and reads that node's rows (`liveOriginNodeRun`). Other pages are read through `PageReadsContext`
(`pages/pageReads.ts`), a test's fake in place of the daemon. New params keep the old run on screen
(`data-stale`), and Loading shows only after 150 ms without a run.

## Declared pages

Every page the desktop draws, lists or names is a page file
(`Extension.pageFiles`). A page file with no `for`
is a **route** (`oxplow:oxplow/page?page=<slug>` for oxplow's,
`page=<extension>/<slug>` for any other's); one with `for` is a
**renderer** of a resource ("Renderers"). Its `label` is what its tab says
(`{{param.<name>}}` over what it shows, `{{row.<column>}}` over its first
loader's first row once it has run, filters `|short` and `|basename`;
none: the title); `icon`, `title`, launcher `category` (none, or
`hidden`: not in the launcher — a page only opened from elsewhere),
`keywords` and `badge` (the launcher's count, one `run_page` when the
launcher opens: `launcherCounts`, its `run.badge`). The launcher lists
the page files with a category that aren't hidden (`launcherPages`, by
`pageFileUri`); a page whose view is a switch also lists each of its
other sections (`<page>: <section>` at `#<param>=<id>`).

`App.tsx` `renderTab` draws a route through `routeTab`: its page file in
`PageFileView`, `MissingPage` when no enabled extension has it, nothing
while the extensions load. The one built-in is `builtInPages.resource`
(`pages/builtInPages.tsx`), the fallback renderer; it is a
`(ctx) => CenterTab | null` over `{ ref, params, nav }`, `nav` the slot's
`SlotNav` (`pages/slotNav.ts`). A page whose `view:` is one component node
*is* that component (`ComponentPage`): oxplow's Timeline (the `timeline`
page, params `from`, `to`, `thread`) is one — its window is its view
params, written on every change (not its opening default) and followed on
Back/Forward; a frame's `setViewParams` reaches the tab as
`patchViewParams` (null drops a key). Icons and labels: `kindForTabId`
gives a route's page id (`tasks`, `gh/open-prs`) or a URI's resource as
its kind, which is also what visits and bookmarks record
(`recordedPageKind`), and `PageKindIcon` / `vizIcon` (`viz/vizIcons.tsx`) / `pageKindLabel`
resolve it to the page file's icon and title.

oxplow's pages (`extensions/oxplow-foundation/pages/*.yaml`; a view is a
`stack` of nodes drawn as sections unless noted; a section titled `''`
has no heading but keeps a section's spacing, and takes no space when it
draws nothing; a filling one is drawn bare):

- **Renderers.** `wiki-page` (`for: oxplow/wiki`, default, tab label
  `{{param.slug}}`, `comments: true`): a `markdown` viz over
  `v_knowledge_body` with an `edit: true` Save on
  `oxplow.knowledge.write_page` and Delete Page, a `detail` of last edit
  and freshness, and a `table` of the cited files with Mark verified (a
  page not yet written is a row with an empty body, made by its first
  Save). `editor` (`for: oxplow/file`, `accepts: { rev: absent }`,
  default, label `{{param.path|basename}}`, `comments: true`): an `editor`
  viz of the working file, `#lines=` landed on
  (`.context/editor-and-monaco.md` "The editor viz"). `file-viewer`
  (`for: oxplow/file` with a `rev`, `comments: true`, label
  `{{param.path|basename}} ({{param.rev}})`): a `code` viz of the file at
  that revision, `#lines=` marked. `directory` (`for: oxplow/dir`, default,
  label `{{param.path|basename}}`; params `uri`, `path`, `stream_id`): a
  `list` viz of `v_workspace_file WHERE parent = :path`, folders first,
  each row linking to its folder or file. `commit` (`for: oxplow/commit`,
  default, label `{{param.sha|short}}`): a row from `v_change WHERE kind =
  'commit' AND target = :sha` feeds a `detail` over `v_commit` (Cherry-pick
  and Revert through `oxplow.git.cherry_pick` / `revert`) and a `tree` over
  `v_commit_file` with status icons linking each file's diff, then the
  `sections` slot; any sha git knows reads. `diff-view` (the Changes page;
  `for` `oxplow/snapshot`, `oxplow/effort` and `oxplow/turn`, default;
  params `uri`, `start`, `end`, `stream_id`; each of `start` / `end` is
  `''`, `none`, `working`, `snap:<id>` or `git:<rev>`; a picked point is a
  pushed view change): an `embed` of `changes-effort`, a `detail` of
  the range, the description, the `effort.verdict` replaceable, a `list`
  of concurrent work, a `tree` of files and a `table` of extensions,
  then the `sections` slot.
  `work-item` (`for: oxplow/work_item`) has the `work_item.detail` and
  `work_item.detail.state` replaceables and a `sections` slot.
  `agent-session` (`for: oxplow/agent_session`, pinned) stacks approvals
  above a `terminal` viz. `external-url` (`for: https, http`): an `embed`
  viz of the URL, label `{{param.uri|url}}`. `resource` is the built-in
  fallback, not a page file.
- **Routes with a diff or code viz.** `diff` (params `path`, `left`,
  `right`, `label`, `base`, view `lines`; label `{{param.path}}
  ({{param.label}})`; `diffRef` always sets `label`, "diff" by default):
  a `file-header` slot, then a `diff` viz of one file between two
  revisions filling the page. A diff of literal text (Compare with
  Clipboard) keeps its sides in the entry's history state, which
  `DiffViz` reads. `duplicate-block` (params `left`, `left_lines`,
  `left_at`, `right`, `right_lines`, `right_at`; label `{{param.left}} ↔
  {{param.right}}`): a `code` viz of two rows with `sync: true`, the
  ranges marked.
- **Lists and boards.** `tasks` (params `show`, `priority`, `thread_id`,
  `stream_id`): `list` (the `work_item.list` replaceable), `summary`,
  `backlog`, then a `sections` slot where bundled mounts the thread's
  tokens, plan, shared work and the handoff pages. `backlog` (badge
  `backlog-count`; params `q`, `thread_id`): a quick-add `form` over
  `oxplow.work_item.create` and a state-grouped `list` of the items on no
  thread (drag to reorder; marked rows get bulk state, priority, Bring to
  this thread and Delete; `r` `i` `b` `d` set states). `board` (params
  `show`: this thread, the backlog or everything): a `board` viz of
  `v_work_item` grouped by state, the `work_item.board` replaceable, drag
  to a column to move a card. `comments` (params `stream_id`, `days`,
  `comment`): a `table` over `v_comment` grouped by anchored page (a row
  selects `comment`) and a `list` of the picked thread's
  `v_comment_message` rows; row actions reply (a form), resolve, reopen,
  make a follow-up or note, and delete. `alerts`: `approvals`, `hints`,
  `operations` (with an `expand` into `alerts-operation`) and `extensions`
  nodes, with `embed`s of `alerts-deliveries` and `alerts-reactions`.
  `git-history` (params `stream_id`, `q`, `author`, `branch`, `head_sha`,
  `reveal_sha`, `limit`): a `table` with the `vcs.history.graph`
  replaceable. `local-history` (params `stream_id`, `mode` `recent` |
  `by-commit`, and `all`, which lifts the caps — the rail's, so no "View
  all" step to go Back from): two `table`s. `uncommitted-changes` (param
  `stream_id`; the working change, `kind = 'working' AND target = ''`):
  a commit `form` over `oxplow.vcs.commit` and a `tree` over
  `v_change_file`, then its `sections` slot. `problems` (`stream_id`,
  `severity`), `symbols` (`stream_id`, `path`, `filter`), `closed-threads`,
  `wiki-index`, `wiki-freshness` (`slug`), `go-to`, `dashboards`,
  `hook-events`, `done-work`, `catalog`.
- **Forms and settings.** `new-stream` (one `form` over
  `oxplow.stream.create`, `visible_if` on its source), `new-task`,
  `stream-settings` (param `stream`) and `thread-settings` (param
  `thread`; label `{{row.title}} settings`): a hint, the open sessions and
  the prompt as an edit `form` over `v_stream` / `v_thread` with
  `oxplow.*.set_prompt`. `settings`: a `switch` on the `area` param
  (`overview`, `agents`, `lsp`, `extensions`, `data`, `integrations`,
  `ai`) with the overview's `q` search; `#section=<id>` lands inside an
  area; each area is a stack of nodes drawn with the kit `Section`.
- **Component and terminal pages.** `terminal` (a `terminal` viz over
  `v_terminal`, the picked one the `terminal` selection, `new` / `rename`
  / `close` actions over `oxplow.terminal.*`; `terminal.md` "The terminal
  viz and its host"). `timeline` (one `component` node; params `from`,
  `to`, `thread`).

New Session (the `new-session` page) has two nodes, `start` and
`earlier`, then a `sections` slot. **Start a new session** is a form over
`oxplow.agent_session.open`: the agent from `v_agent_harness`, and
"remember" — the person's `newThreadSession` — folded into the command.
**Earlier sessions** is a table (a `q` search in the rail; ↑/↓/Enter on the focused table; a row's
Reopen/Resume action, `then: replace`, or its link to a session already
open here — `terminal.md` "Earlier sessions"). A diff's `revealLine` is not in its
id, so re-clicking reuses the tab.

## Left-nav panels (P6.G1)

The rail's sections are **panels** (`components/Panels/panelLayout.ts`):
every enabled extension's `panels:` — core has none of its own since Go
To moved into `oxplow-bundled`; with it disabled the rail is
the search box and + Add Panel
(`ext:<extension>/<id>`, rendered by `ExtensionPanelSection`). Every
panel's parts (`body`, `collapsed`, `badge`, `count`) are pages:
`useExtensionPanelRuns` holds one open page run per part (params its
scope's, the body's with the viewer's picks), the body and collapsed
pages drawn compact by `PageTree`, the header count the count page's
`badge` (else its first loader's rows) or the badge page's alert count
while it fires (else its `badge`), and the badge page's alert an Alerts
row whose Review opens that page (`PanelAlert.open`, a URI); the header's
↗ opens the panel's `open` page, else its body page (`panelOpenRef`).
A compact viz keeps its `group` headings and group actions, and its
linked rows drag into the agent's context.
The rail's panels are `oxplow-bundled`'s (`go-to`, `waiting`, `comments`,
`uncommitted`, `work`): pages over the existing views and commands. One owner
holds every panel's runs — `PanelRunsProvider`
(`components/Panels/PanelRunsContext.tsx`, around the app),
read by the rail, the status bar's bell and the Alerts page — through
`components/Panels/usePanelRuns.ts` → `useExtensionPanelRuns`: it binds each panel's scope — `stream_id` /
`thread_id` as row ids, from `panelParams` — for the stream and thread it
shows, opens new runs when either changes (the daemon re-runs them when
a read changes), and hands each section its runs. The firing badges come from the same runs
(`panelAlerts`), so a badge never runs twice. What needs the person —
proposals, failed operations, undelivered events — isn't a rail panel: it's
the status bar's bell and the Alerts page (see usability.md); a firing badge
shows in its panel and a toast, not on the page. A
layout stored before a core panel existed gets it appended at the bottom.
The set of extension panels (like extension pages and slot mounts)
reloads on `rerun.extensionsChanged`: a deployment changed (`oxplow.extension.deploy`, `rollback`, `enable`,
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
never undoes an edit. So every edit means the same on any base:
absolute (`setCollapsed(panel, collapsed)`, not a toggle) or relative to
a panel (`movePanelBeside(panel, target, side)`, not an index). One
`layoutSync` (`panelLayout.ts`) owns the round trip: a **failed
load** is reported and shows the defaults, but saves nothing over the
layout it couldn't read — the next edit loads again and is replayed on
what it finds; saves go **one at a time, the latest last**; and an edit
**keeps the placement of a panel not available yet** (an extension's,
before extensions load) right after the entry it followed.

## Page archetypes and the kit

Core owns the look; a page, core or extension, fits one of three shapes
and draws it with the **component kit** (`apps/desktop/src/kit/`, one
export point `kit/index.ts`). The kit's parts render the classes of one
stylesheet, `crates/oxplow-app/assets/component/oxplow-kit.css`, which the app
imports (`main.tsx`) and custom components link, so a core page, a page
file and a sandboxed component look alike. A page composes the kit; a new local
section, table, filter list or "Loading…" line is a review finding.
Settings, Stream and Thread settings, Alerts, Go To and the wiki page list
draw their parts with the kit `Section` (no local copies): a page's action
is a `PageAction` on its section (Alerts' "Clear failed operations" is
`action-clear-ops`), and the Settings index scrolls to a section by its
`data-section` id.
Core tables are `DataTable` too (AI usage, Data entities, the Metrics list
and a metric's recordings, a dataset's documented columns). A row built in a
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
| **Document** (the default) | reading, picking, small forms, lists | `PageHost` / `PageTree`: title + intro in a reading column, then a `stack`'s children as `Section`s (a heading over a rule, no boxes) | `view: { stack: [...] }` |
| **Dashboard** | many numbers and charts at a glance | `TileGrid` / `TileSlot` / `Tile` at full width | `view: { grid: [...] }` (a page in `category: Dashboards`: `DashboardGrid`) |
| **Board** | cards by group, dragged across columns | (none) | `viz: board` + `group: { by, values }` and a `drag: true` action |
| **Document with an edited body** | a wiki page, a work item's body: markdown read and written, with a Contents rail and comments | the `Markdown` component (`.context/markdown.md`) | `viz: markdown` + `markdown: { text }` and an `edit: true` action (its Save), the page `comments: true` |
| **Reader** | one file, two files side by side, or one diff, at the page's whole size | `Page` with `fill` (the body stretches to the page's height) | `viz: code` / `viz: diff` |
| **Custom** | what the kit can't express: editors, terminals, boards, git flows | a React page on `Page`, using kit parts where they fit | `viz: custom` (the sandboxed component), or a page whose `view:` is one `component` node (`PageComponent`) |

A cell's match is drawn with the kit class `.ox-mark` (a column's
`highlight`; `VizConfig` in `crates/oxplow-app/src/pages/spec.rs`).

The kit: `Section` (`id`, `title`, `description`, `actions`, `collapsible` / `defaultCollapsed` — a toggle in the heading, remembered per page, see usability.md "Collapsible page sections" — `source` —
the extension that added it), `DataTable` (fixed `<col>` widths,
`numeric` and `truncate` columns, groups, per-row props — the `table`
viz draws through it), `Toolbar` (a core list's filter and actions; a
page file has none; `useSearchFilter(key, …)` builds its `filter` on the `key` view
param) with `useListNav`
(the one ↑/↓/Enter/Escape contract, usability.md "Keyboard"; a list that
holds focus itself spreads its `keyProps`), `Stat` (a figure; `big` when
it is its place's whole — the `number` viz), `Facts` (labels beside
values — the `detail` viz, a metric's definition),
`TileGrid` / `TileSlot` / `Tile`, `Loading`, `ActionButtons`, and
`EmptyState` (re-exported; it lives beside the agent prompts it offers).
Lengths come from the spacing and radius tokens (theming.md).

**Extensions add to a page through its `slot` nodes** (a `slot` node named
`sections` in the foundation's `changes` page is
`oxplow-foundation/changes.sections`; the id is `<extension>/<page>.<name>`).
An extension lists `ui.mounts: [{ page, at, position }]` in its manifest. It
mounts one of its own pages there, placed (`position`) at the start, the end, or beside
a node by its stable id. A mount whose host slot is missing is a load
warning. extensions.md "Slots".

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
  ("Details" by default). A page's rail is `PageRail` — Filters, its
  `rail:` tree, Selection, built from `RailGroup` and `ParamList`
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
   `standard` actions (a page's Refresh when it declares `refresh: true`,
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
  them all in its header, never in the rail. A page's own action
  is `prominent`, in a `submenu` or drawn with an `icon` when its page file
  says so, a custom component's when its `setActions` does; only oxplow
  marks an action `standard`;
- `pin: { tile, onOpenPage }`: what the page would add to a dashboard —
  the ☰'s Pin to Dashboard… (`menu-item-page-pin`). A page file pins as an
  `embed` node of itself (`PagePin`, `pages/PagePinMenu.tsx`): the menu lists
  the writable pages in the Dashboards category (`v_page`) and adds the
  embed to the one picked, through `dashboardEdits.ts` and `write_page`
  ("Writing a page"). "New dashboard…" runs
  `oxplow.extension.new_dashboard` first.

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
   its dataset `thread_work` (one row per line, per thread), which reads the
   work-item interface (`v_work_item`): whichever list is active, nothing
   with none. The body page `work` groups the lines (`group`): "In
   progress" (the active item; under its epic, with the epic's other
   children indented and the active one emphasized, when it has one, plus
   an open effort no item names), "Ready" (10, by `rank` then
   `created_at`) and "Finished" (5 done items, wiki pages and unlinked
   efforts, its heading opening the thread's activity, with a `clear`
   group action: `oxplow.work.clear_finished` records an event and the
   `finished_cleared` dataset hides what finished before it — per thread,
   no longer browser-local). Status and kind show as row icons. Collapsed,
   the `work-line` page: the active item (with its epic), else the last
   finished. The header count (`work-count`) is in progress + ready, never
   an alert. Rows link (Cmd-click opens a new tab) and drag into the
   agent's context.
3. **Uncommitted** — an `oxplow-bundled` panel, stream-scoped,
   over the working change's stage-one file list (`v_change_file` /
   `v_change.conflicted` / `in_progress`, kept current by `change.analyze`
   on every move — the app no longer polls git for it). The body page
   `uncommitted` is a folder tree: each file its A/M/D (toned) linking to
   the file, each folder the letters under it, a merge or rebase in
   progress as a red row on top. Collapsed, `uncommitted-line`: `3A 2M 1D
   +10 −4` and the conflict line, opening the Uncommitted page; the count
   (`uncommitted-count`) is the changed files.
4. **Comments** — an `oxplow-bundled` panel: the stream-scoped
   body page `comments-panel` over `v_comment` shows two open-comment count rows split
   by intent — "For me" (`note`) and "For the agent" (`followup`) — each
   row and the header (`open` is the `comments` page) opening the Comments inbox.
5. **Go To** — an `oxplow-bundled` panel (`ext:oxplow-bundled/go-to`):
   the thread-scoped body page `go-to-panel` lists the bookmarks the thread sees
   (`v_bookmark`), then its History — or, through the header's
   Recent / Most visited toggle (the page's `mode` choice param), its most
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
   With a query, pages rank by `pageRank`: every token starting
   a word of the title, then the title as a subsequence, then the id, then
   the `keywords` — where each token must start a word, since a page's
   keywords can hold its whole description and a short subsequence ("todo")
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
  `dock: true` follow, sorted by title. Their body is the panel's `body`
  page and their badge is its `badge` or `count` page. `useDockPanels()` returns the whole list.
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
undone; see [usability.md](./usability.md) → "Per-row actions").
Git history has the phase-by-phase detail if you need it.

A later scope change moved analytics pages out of core into the
`oxplow-bundled` extension as page files mounted through slots
([extensions.md](./extensions.md)).

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
  efforts, decisions, commits, pages (dashboards among them); the thread-shaped kinds
  include archived work); each row shows its kind's icon and name
  (`pageKindIconComponent` / `pageKindLabel`); a hit is a ref, opened through the
  ref opener (`openSearchHit` → `openRef` → `tabs/refOpener.ts`),
  which may select a thread or switch stream. The ordering is a pure helper —
  `buildQuickOpenResults` in `components/quickOpenResults.ts` — so it's
  unit-tested without mounting React.

Because the launcher lists every page (including Code Quality, Hook
Events, Local History), no page needs a bespoke `CommandId` to be
reachable — that supersedes the old "wire each page as a menu command"
approach.

The rail no longer has a "Pages" section (the `rail-page-*` / `rail-pages`
testids are gone); Go To's bookmarks are the always-visible curated nav. E2e
probes that used to click `rail-page-*` should drive `title-bar-search` → the
launcher (type, then assert `page-<kind>` on the body).

**Work pages split (post-Phase-3).** The single `AllWorkPage` was
replaced by four focused pages so each has one job:

- **Tasks** (the `tasks` page, a foundation page file) — the thread's items by state,
  or its stream's or everything's (the `show` filter); the last five done and a "View all done →"
  link to Done Work; the agent's open turn is a live row in In progress. Ref helper is `tasksRef()`.
- **Done work** (the `done-work` page, a foundation page file) —
  full descending list of the current thread's closed items (done and canceled).
- **Backlog** (the `backlog` page, a foundation page file) — the active list's items on no thread
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
`backlogRef()` — mirror the GitDashboard pattern
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
(`CommitRowDispatcher`), and every page table or list's link column
(`viz/linkOrigin.ts`). That one is **live**: the origin keeps no items, only
`live: { node, params, column }` (`LiveOrigin` in `tabs/session.ts`; `node` holds
`<page>#<node>`, the page and the viz node), and `LiveOriginResolver`
(`tabs/liveOrigin.tsx`) holds the producing page's run (kept fresh by the
daemon, as the page itself is) and hands App the node's column pages in row
order (`liveOriginNodeRun`, `resolveLiveOrigin`). `resolvedOrigin` finds the current page in that list for the nav bar and the
keys; `stepOrigin(s, slot, index, items)` steps through the resolved list. A page
the node no longer lists has no Up/Down. `WorkGroupList` rows open an edit
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
publishes through `tabs/openPageDetail.ts` (a page file's `{runId, focus: {node}}`,
published by `PageHost`; an editor's `{path, caret, selection}`). A window is its renderer's
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
  `back`, `forward` and `origin`, and, for a page file, its tree and run as
  text (`pages/text.rs`). Detail comes only from a window showing that thread's page. Two windows never
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
(`linkRef`), page links (a column's `link`), timeline entries, backlink rows, a component's
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
(Explore Data's `dataset`, `sql`, `viz`); its
`?` is only what it shows (a diff's path and sides, a dashboard's id, a
metric's key), so two diffs in a tab are two pages. A param written in
the wrong place is not read from there. A page param's `role`
(`resource` by default, `filter`, or a column's `select`) says where its
value rides (docs/guide/pages.md "Where a param lives in the address").
Page memory is scoped by `keyIdentityOf` (the
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
| **page memory** | this page, for the person, every tab of it, across restarts | `ui_preference`, scoped by `keyIdentityOf` (the page's URI without its view and its non-key params: `oxplow:oxplow/page?page=tasks`, `oxplow:oxplow/page?page=x/y`) or `resource:<ext>/<name>` | `usePageMemory(key, initial)` (scope from the page) |
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
says which keys** (`viz/hostedView.ts`; `.context/extensions.md` "Whose
view a component has"): a replacement the keys its target's contract
lists (the history graph: Git History's `q`, `author`, `branch`, the
same ones core's commit list reads), a page's own component all of
the page's params (`"all"`). Its history state is its own object in the entry
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

When a page is current in one tab and also appears in another tab's Back
history, the current entry supplies its mounted body and navigation
context. The hidden history copy must not win the body's deduplication:
its links would navigate the older tab's current entry instead. The
Dashboards browser test covers creating a page, reopening the index and
following its link back to that existing page.

Every tab is a session tab, whatever it shows (a wiki page, a file, a
diff, a work item, a page file, a commit, an agent session, a web page, …). The
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

The rule for new pages: a page is a `pages/<slug>.yaml` file (see "Adding a
page"); a new kind of thing is a resource plus a page file with `for:`.
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
(`CollapsibleSections`), a page tree's closed branches and open diff, a commit's expanded message, and the rows a page picked (`picked`, in `PageHost`); and the unsaved forms (`useDraft`): New item,
New stream, the stream and thread prompt, the Settings-area forms (Agents, Integrations, Language servers; a secret such as an API key is never kept), the changed-files search, the
uncommitted commit message and a task's title. Keep values JSON: one
value that isn't drops the entry's whole state from storage.

## Adding a page

A page is a file, not core code:

1. Add `pages/<slug>.yaml` to the extension (oxplow's own:
   oxplow-foundation's) with its `title`, `icon`, `category` (or
   none, if it's only opened from elsewhere), `params`, its `data`
   loaders and a `view`. Nothing in the desktop changes: the launcher, icons,
   labels, tab titles, visits and bookmarks all read the page file. The
   manifest lists no pages.
2. Open it with `pageTabRef(slug, params)` (oxplow's) or
   `extPageRef(ext, slug, params)`; a typed wrapper in `pageRefs.ts` is
   worth it only when the params are typed.
3. Where the kit can't draw it yet, grow the kit, or ship a custom
   `component` node.

A new **kind of thing** (its tab id is its URI) is a resource plus a
page file, with no desktop code:

1. Declare the resource in the extension (`resources:`,
   [resources.md](./resources.md)), so its URI has a type, params and a
   label.
2. Add a page file with `for: <extension>/<name>`, optional
   `accepts`, `default: true` (the extension's own resource, one page
   per resource) and `pinned`. Such a page declares a `uri` param
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

## Writing a page

`oxplow.extension.write_page { extension, page, spec, deploy? }` is how the
app saves a page (Save as Page, Pin to Dashboard, a dashboard's reorder,
rename, tile removal and new dashboards). `spec` is the page's YAML text,
kept as written so comments and key order survive. It goes to
`pages/<page>.yaml` in the extension's source in the primary checkout
(`extension_deploy::write_page`), after `pages::schema::parse_page` accepts
it and the extension still loads with it; the extension is created with a
scaffolded manifest when it is missing (the app's default is the personal
`my-pages`). An existing page is replaced. A bundled extension is refused,
and so is a caller in another stream when the source is inside the project
(`refuse_draft_deploy`). `deploy: true` deploys it right away; otherwise
the page waits for a deploy like any source edit.
