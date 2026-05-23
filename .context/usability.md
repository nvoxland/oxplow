# Usability rules


Things I keep forgetting. Read this before adding any UI.

> **IA redesign — phases 0–7 fully shipped.** Modal `ConfirmDialog`
> and `PromptDialog` chrome was retired in favor of inline patterns;
> the right-click → `ContextMenu` reflex was replaced by visible
> kebab `⋯` buttons on each row; per-stream and per-thread settings
> ship as Page tabs (`StreamSettingsPage`, `ThreadSettingsPage`); new-
> stream and new-task flows ship as Page tabs (`NewStreamPage`,
> `NewTaskPage`); snapshot- and commit-detail Slideover wrappers
> (`SnapshotDetailSlideover`, `CommitDetailSlideover`) cover the
> cross-page open path. The rules below describe the redesigned
> target. Phase 7 (density + visual polish) details live in
> `.context/theming.md`'s Density section; the per-phase migration log
> lives in `.context/pages-and-tabs.md`. Plan:
> `/Users/nvoxland/.claude/plans/the-ui-is-very-delightful-badger.md`.

## Forms

- **Edit-X-in-place actions are inline, not modal.** Click the
  displayed value to swap to an input; Enter commits, Escape reverts,
  blur commits unless Escape was pressed. The shared helper is
  `apps/desktop/src/components/InlineEdit.tsx`; `TaskDetail`'s
  `EditableField` and `WorkGroupList`'s `InlineItemRow` are older
  hand-rolled equivalents — copy whichever is closest. The cancel
  latch must be a `useRef` (state updates are async; the blur fires
  on the same tick). Use `multiline` for textareas (Cmd/Ctrl+Enter
  commits; Enter inserts newline). Use `allowEmpty` to permit
  clearing.
- **Tiny prompt strips render inline at the top of the owning
  panel** for "+ New file" / "+ New folder" / Rename flows where the
  trigger comes from a kebab menu rather than a row that already
  shows the editable value. See `InlinePromptStrip` in
  `ProjectPanel.tsx`. Same Enter-submits / Escape-cancels contract;
  the strip is dismissed by the panel's local `pendingPrompt` state.
- **Form-shaped flows that warrant a focused workspace use a page tab
  or a slideover, not a centered modal.** The "+ New" flows ship as
  Page tabs (`NewStreamPage`, `NewTaskPage`, the `Stream/Thread`
  settings pages); cross-page detail openings (snapshot, commit,
  branch rename, file commit) ship as Slideovers. The remaining
  legacy hand-rolled modal chrome inside `PlanPane.tsx`'s
  `NewTaskModal` only backs the edit-double-click flow — do not
  add new modal call sites; route new flows through pages or
  slideovers. The page pattern to copy is
  `apps/desktop/src/pages/SettingsPage.tsx` — full Page tab, no backdrop.
- **Never call `window.prompt()`.** The Tauri webview blocks it —
  it returns `null` synchronously without
  showing anything, so any code path gated on its return value
  silently no-ops. Use `InlineEdit` (for
  click-to-edit) or `InlinePromptStrip` (for new-X flows that need a
  target-path entry) instead. `window.confirm` / `window.alert` block
  the renderer; prefer `InlineConfirm` for destructive actions on a
  row/button and `showToast({ message, onUndo })` for fire-and-undo
  destructives that aren't tied to a specific row.
- **Async-op failures don't `alert`.** Push a record into
  `opErrorsStore` (`recordOpError({ label, command?, stderr?, stdout?,
  exitCode?, message? })`) — the RailHud renders an Errors section
  with red rows; clicking a row opens an `op-error` page tab with the
  full output. For ops that already have a page focus when they fail
  (e.g. `runConfirmed` in GitDashboardPage), call
  `onOpenPage(opErrorRef(id))` after recording so the user lands on
  the detail view directly.
- **Every `<button>` needs an explicit `type`.** HTML defaults
  `<button>` to `type="submit"`, which silently submits any enclosing
  form on click. Use `type="button"` for every action button; use
  `type="submit"` only on the form's primary action. Don't rely on
  the default — it's a tripwire.
- **Enter submits.** Any form with a primary action must submit on
  Enter from any single-line input or select when all required fields
  are valid. Use a real `<form onSubmit=...>` wrapper; the browser
  handles single-line Enter for you. For multi-line textareas, Enter
  inserts a newline and Cmd/Ctrl+Enter submits.
- **Escape cancels.** Inline edit fields and inline-confirm pairs
  revert on Escape. The legacy modals that haven't migrated yet still
  close on Escape via their own keydown listener.
- **Disabled submit button when invalid** rather than erroring on
  submit. Show required-field hints inline.
- **Autofocus the first input** in any inline edit / prompt strip
  when it mounts (and select existing text so the user can replace
  it with a single keystroke).
- **"Save and Another"** for repetitive-entry flows (see the New Work
  Item modal): saves and re-opens the form with the same
  category/priority/parent pre-filled so the user doesn't re-select
  them. Carry this convention forward when New task migrates to
  a page (phase 5e).

## Destructive actions

- **Per-row destructives use `InlineConfirm`** at
  `apps/desktop/src/components/InlineConfirm.tsx`. First click on the trigger
  swaps to a `[Confirm] [Cancel]` pair in the same horizontal real
  estate. The Confirm button auto-focuses; Escape, blur (outside the
  pair), or Cancel reverts. Examples in tree: Restore button on each
  file row in `SnapshotsPanel.tsx`'s detail pane; Delete button on
  `WaitPointRow.tsx`; Force-delete button in `BranchPicker.tsx`'s
  manage flow.
- **Non-row-anchored destructives fire immediately and surface an
  Undo toast.** Use `showToast({ message, onUndo })` from
  `apps/desktop/src/components/toastStore.ts`. The toast auto-dismisses after
  ~7s and the [Undo] button calls the supplied callback. Mount the
  `<UndoToastStack />` once near the app root (already done in
  `App.tsx`). When the action is genuinely irreversible (delete a
  task permanently) push a toast without `onUndo` so the user
  still sees confirmation feedback even if they can't undo. Don't
  block the renderer with a centered confirm modal.
- **Closing a dirty file tab** is fire-and-undo: the close completes
  immediately and a toast offers Undo (which restores both the saved
  buffer and the unsaved draft). See `App.tsx` →
  `handleCloseOpenFile`.

## Per-row actions (was right-click menus)

- **Visible kebab `⋯` button per row, not right-click.** The shared
  primitive is `apps/desktop/src/components/Kebab.tsx` (button + `ContextMenu`
  popover anchored under the button). The popover keeps the same
  `MenuItem[]` payload as the legacy right-click menus — call sites
  swap their handler, the menu items themselves are unchanged.
- The `ContextMenu` popover at `apps/desktop/src/components/ContextMenu.tsx` is
  still in use as the popover renderer; just don't open it from a
  raw `onContextMenu` handler in new code. If you find a surface
  that still does, that's a phase-5c continuation site — wire it
  through Kebab or pass a rect-based callback that opens the same
  menu.
- **`menu-item-<item.id>` testids** stay on every button inside the
  shared `MenuList` — the `MenuItem.id` becomes the testid suffix
  (e.g. `menu-item-task.delete`,
  `menu-item-task.rename`).
- Close on outside click, scroll, window resize.
- **The native WKWebView context menu is globally suppressed.**
  `installContextMenuSuppressor()` (in `apps/desktop/src/context-menu.ts`,
  mounted once from `App.tsx`) cancels the OS-default right-click menu
  (Look Up / Translate / Copy / Share / Inspect Element / Services) so it
  never appears on arbitrary surfaces — Oxplow's own per-row menus are
  the visible kebab `⋯` popovers instead. It exempts text inputs /
  textareas, contenteditable (Tiptap), Monaco (`.monaco-editor`), and the
  terminal (`.xterm`) so right-click copy/paste and the editor's own menu
  still work there. The decision is a pure `shouldSuppressContextMenu`
  predicate over an ancestor-descriptor chain (unit-tested without a DOM);
  add new exempt surfaces there.

## Commenting on any surface

Comments are not editor-only. Any page region can be commentable by
declaring *what it is* and mounting the generic layer.

- **`data-ref-kind` / `data-ref-id` mark a region as a typed "context
  node."** The `(kind,id)` pair uses the same canonical vocabulary as
  tab ids and the `page_ref` graph (`file` / `directory` / `wiki` /
  `task` / `git-commit` / `finding`). Stamp them on the element that
  *is* that thing (e.g. a task row carries `data-ref-kind="task"
  data-ref-id="42"`). Nesting is meaningful: a file row inside a commit
  card yields the chain `[file, git-commit, …]`, innermost first. Treat
  these as a first-class seam like `data-testid` — spread
  `contextNodeProps(kind, id)` from
  `apps/desktop/src/components/Comments/contextNodes.tsx`.
- **Selecting text on a context node shows a floating "Add comment"
  button** (`SelectionCommentToolbar`), driven by `useDomAnnotations`.
  It is additive and non-destructive (so a floating affordance is fine,
  unlike destructive actions which stay on the kebab), dismisses on a new
  selection or Escape, and reuses the **same** `shouldSuppressContextMenu`
  carve-out so it never appears inside Monaco / Tiptap / inputs / the
  terminal — those own their own comment UX.
- **`DomCommentLayer` is mounted once, at the app level** (`App.tsx`,
  beside the center tab outlet) — NOT per page. A selection only becomes
  a comment when it lands inside a `data-ref-*` region and outside the
  editor/terminal carve-out, so a single instance safely serves every
  plain-DOM page; a page with no context nodes simply never captures.
  Adding commenting to a new surface is therefore just "stamp
  `data-ref-*` on the regions" — no per-page wiring. The layer captures
  selections, paints existing comments back onto their context node's
  text via the **CSS Custom Highlight API** (no DOM mutation — critical
  for live React lists), and opens a thread popover when a highlight is
  clicked. The quote is re-resolved against the element's `textContent`
  each repaint (debounced to one per frame), so reordering /
  virtualization just re-anchors. The Highlight API is feature-detected;
  where it's absent the comment still works, just without an inline
  highlight. Surfaces opted in so far: task rows (`TaskGroupList`), the
  commit page (`GitCommitPage` meta → `git-commit`), finding pages
  (`FindingPage`: snippet `file` nested under the `finding` container),
  and commit-graph rows (`CommitGraphTable` → `git-commit`, under the
  `git-dashboard` root). The **terminal/agent pane** has its own layer
  (`TerminalCommentLayer`, not the app-level one) because it anchors to
  the xterm buffer rather than DOM text — see `.context/terminal.md`.
- **Make the text selectable.** Rows are often `userSelect: none` for
  clean drag — set `userSelect: "text"` on the specific label span you
  want commentable (e.g. the task title) so a quote can be anchored
  without re-enabling selection on the whole row.
- `data-testid`s on the affordances: `selection-comment-button`,
  `new-comment-popover`, `comment-popover-<id>`.

## Keyboard

- **Shortcuts go through the menu.** Add new shortcuts to
  `commands.ts` and `keybindings.ts` so they appear in the native
  menu and help discoverability.
- **The native menu is renderer-driven.** `App.tsx` pushes the menu
  snapshot to `set_native_menu` (built in
  `crates/oxplow-tauri-ipc/src/commands/menu.rs`); macOS shows the
  native bar, off-Mac falls back to the in-window `Menubar`. (There is
  no `isElectron` gate any more — that was dead post-Tauri code.) The
  builder supports **nested submenus** via `MenuItemSnapshot.submenu`;
  dynamic entries (e.g. File ▸ Open Recent ▸ `<project>`, built by
  `buildNativeMenuSnapshots`) use free-form ids like
  `project.openRecent:<path>` that the `menu:command` handler matches by
  prefix rather than going through the static `CommandId` map.
- **The macOS application submenu is added in Rust, not the snapshot.**
  `build_menu` prepends a `#[cfg(target_os = "macos")]` "Oxplow"
  submenu of `PredefinedMenuItem`s (About / Hide / Hide Others / Show
  All / Quit) before the renderer's groups, because on macOS the first
  submenu always renders bold under the app name — without it the File
  group lands there and there's no visible Quit. These items are
  OS-standard and state-free, so they stay out of the snapshot (and out
  of the off-Mac in-window `Menubar`).
- **The View menu is tab-IA navigation**, not a view toggle: Files /
  Uncommitted Changes / Comments Dashboard / Wiki / History each open
  the matching page in the active thread's tab set (via `indexRef` /
  `uncommittedChangesRef` / `commentsRef`). The old binary
  Agent-vs-Editor `checked` toggle from the pre-IA two-pane layout is
  gone, and Agent itself is no longer a View item — the agent tab is
  the pinned center tab. The Git and Tasks dashboards moved out of View
  into their own top-level menus (below).
- **The Git menu** carries the working-tree git surface: `Dashboard`
  (opens `gitDashboardRef`, gated on a stream) plus `Commit Changes…`,
  `Pull Changes`, and `Push Changes` (gated on `canCommit` — stream +
  git enabled). Pull/Push run via `gitPull` / `gitPush` as background
  tasks; failures record an op-error and surface a "Show details" toast
  (same pattern as the Git Dashboard's `runOp`). Commit opens the Files
  page and triggers the commit slideover.
- **The Tasks menu** (group id is still `plan` for keybinding/command-id
  stability; label is "Tasks") leads with `Dashboard` (opens the tasks
  index) followed by `New Task…` / `New Thread…` / `New Stream…`.
- **Common muscle memory:** Cmd/Ctrl+S save, Cmd/Ctrl+F find,
  Cmd/Ctrl+P quick open, Cmd/Ctrl+Shift+N new task. Don't
  collide with these.
- **Plan pane: single-click selects a task row (keyboard
  cursor); double-click opens the edit modal.** Enter also opens the
  modal for the selected row. Cmd/Ctrl+click toggles the mark set;
  Shift+click ranges from the selected anchor. A plain click clears
  marks and moves the selection. Marked rows render with a yellow
  left-stripe + tint. Dragging any marked row carries every marked
  id in `WORK_ITEM_DRAG_MIME.itemIds` so drops on BatchRail chips,
  the backlog chip, or StreamRail move all of them at once. Drop
  targets that handle single-item payloads still work — they fall
  back to `itemId` when `itemIds` is absent.
- **Plan pane: a selection-aware action bar appears at the top of the
  work-group region whenever ≥1 row is marked.** Component:
  `apps/desktop/src/components/Plan/SelectionActionBar.tsx`. Buttons mirror the
  marked-set right-click menu — Change status / Change priority /
  Add to agent context / Delete — plus a Clear button. The bar reads
  the existing marked-set state in `PlanPane`; there is no separate
  store. Pure helpers (`shouldShowSelectionActionBar`,
  `summarizeSelection`) are exported for tests.
- **Plan pane: Shift+↑/↓ reorders the selected task within its
  own status section.** Crossing a section boundary is a deliberate
  no-op — to change status, the user drags (which changes status as
  a side effect). Plain ↑/↓ just moves selection; Enter toggles the
  detail pane; `s`/`p` opens the status/priority pickers.
- **Cmd+K palette listener uses `capture: true`.** Monaco and other
  focused inputs run their own keydown handlers in the bubble phase;
  capture lets the palette fire before any of them. If you add
  another global shortcut that needs to beat an editor, copy that
  pattern.
- **Palette is the main keyboard lever — keep it populated.** Every
  new menu command in `commands.ts` flows into Cmd+K automatically
  (the palette reads from the same `buildMenuGroups` registry). When
  adding a user-visible action, prefer wiring it as a CommandId over
  a bespoke button so it stays keyboard-reachable. Current entries
  include `stream.new`, `thread.new`, `history.open`, `git.dashboard`,
  `git.commit`, `git.pull`, `git.push`, `tasks.dashboard` alongside
  save/find/quick-open/new-task.

## Test-driveability

- **Add a `data-testid` to every new seam a user — or a test —
  would need to drive:** tabs, primary action buttons, form inputs,
  list items, dock panels. Existing conventions:
  - `dock-tab-<id>` / `dock-panel-<id>` on DockShell rail + content
  - `file-tree-entry-<path>` on FileTree nodes (plus `data-kind` and,
    for dirs, `data-expanded`)
  - `monaco-host` on the editor container, `data-file-path=<path>`
  - `plan-new-task`, `task-title`, `task-priority`,
    `task-description`, `task-acceptance`, `task-save`,
    `task-save-another`, `task-cancel`
  - `command-palette-input`
  - `plan-pane` (the keydown-listening wrapper — focus this before
    dispatching keyboard probes, otherwise the listener misses them)
  - `plan-add-points-bar` (now a single ⋯ menu — only "New task" lives
    in it; commit/wait point markers were removed)
  - `files-commit`, `files-commit-message`, `files-commit-submit`
  - `thread-rail-new`, `thread-chip-<threadId>` (chip testid is on
    the outer wrapper that owns the drop handlers, so drag probes
    can target it directly), `thread-chip-kebab-<id>` on the kebab
    button inside each chip
  - `stream-tab-kebab-<id>` on the kebab button inside each stream
    tab; `center-tab-kebab-<id>` on each center-tab kebab
  - `task-row-kebab-<id>` on each task row's kebab
  - `menu-item-<item.id>` on every button inside the shared
    `ContextMenu` / `MenuList` — the `MenuItem.id` becomes the
    testid suffix (e.g. `menu-item-task.delete`,
    `menu-item-task.rename`, `menu-item-task.status`,
    `menu-item-task.priority` — rename/status/priority mirror
    the inline click / `s` / `p` shortcuts so keyboard-first users
    don't have to hover)
  - `undo-toast-stack`, `undo-toast-<id>`,
    `undo-toast-action-<id>`, `undo-toast-dismiss-<id>` on the
    Undo toast bottom-stack. The most-recent toast also gets the
    stable aliases `undo-toast`, `undo-toast-undo`, and
    `undo-toast-dismiss` (no id suffix) so probes can target "the
    toast that just appeared" without chasing the random toast id.
  - `rail-page-<entry-id>` on every Pages-section button in
    the left rail HUD (e.g. `rail-page-git-history`,
    `rail-page-local-history`, `rail-page-code-quality`,
    `rail-page-hook-events`). Click these to open the Page-wrapped
    renderer as a center tab; assert via `page-<kind>` on the body
    (e.g. `page-git-history`, `page-local-history`, etc.). The
    `dock-tab-history` / `dock-tab-snapshots` / `dock-tab-code-quality`
    / `dock-tab-hook-events` testids no longer exist — the bottom
    drawer was removed in the IA cleanup.
  - `center-tab-<id>` on CenterTabs tabs (id is `agent` for the
    agent tab, `file:<path>` for open-file tabs);
    `center-tab-close-<id>` on the × close button
  - `thread-rail-create-input`, `thread-rail-create-submit` on the
    new-thread creation row; `thread-chip-rename-input-<id>` on the
    inline rename input; `thread-chip-promote-<id>` and
    `thread-chip-complete-<id>` on the hover-card actions (also
    reachable via the kebab → `menu-item-thread.promote` /
    `menu-item-thread.complete` — keyboard-first users should never
    have to hover to promote a thread)
  These are load-bearing for `tests-e2e/` — don't rename casually.

## Feedback

- **Show loading state** for any operation >150ms.
- **Show counts** where relevant (e.g., "24 / 500 commits" in the
  history filter).
- **Don't silently drop edits.** Failed operations must surface an
  error near the affected control, not only in the toast area.

## Drag and drop

- **HTML5 DnD needs `dragDropEnabled: false` on the Tauri window.**
  Tauri v2 defaults `dragDropEnabled` to `true`, which registers an
  OS-level drag-drop handler that swallows `dragover`/`drop` before the
  webview DOM sees them — the drag ghost appears but no drop ever fires.
  Every in-app drag here (center-tab reorder, thread/stream rails,
  add-to-agent-context) is DOM drag-and-drop, so the `main` window in
  `apps/desktop/src-tauri/tauri.conf.json` sets `dragDropEnabled: false`.
  Don't re-enable it unless something starts needing Tauri's *native*
  file-drop events (and then reconcile both).
- **Highlight the drop target** (dashed border + accent glow) whenever
  a compatible drag enters it. Clear the highlight on leave/drop.
- **Use a custom MIME type** for internal drags so foreign drags
  (files, text) don't accidentally trigger app drops. Existing MIMEs:
  `WORK_ITEM_DRAG_MIME` (task reorder) in
  `apps/desktop/src/components/ThreadRail.tsx`, and `CONTEXT_REF_MIME`
  ("Add to agent context") in `apps/desktop/src/agent-context-dnd.ts`. Add a
  new MIME rather than overloading an existing one.
- **Tabs in the three tabbed sections (left dock rail, center pane, bottom
  dock rail) are drag-reorderable.** DockShell rail tabs persist their order
  in the dock's `localStorage` entry (`oxplow.layout.v1.dock.<key>.order`).
  CenterTabs reorders **every non-pinned tab freely across the whole
  strip** — there are no per-kind groups. "Pinned" = non-closable (only
  the `agent` tab); it stays at the front and is never a drag source or
  drop target, so nothing lands before it. Reorders persist by rewriting
  the unified `threadPageTabs` order (the strip renders
  `[agent, ...threadPageTabs]`); `App.tsx`'s `handleReorderCenterTabs`
  reorders that whole list, not a per-kind subset. Clicking a tab in the
  overflow `▾` panel (or any activation of an overflowed tab) promotes it
  to **right after `agent`** via `promoteHiddenIntoStrip` (inserts after
  the leading run of pinned tabs), so it surfaces in the most prominent
  slot. The drop indicator is a **vertical insertion line in the gap**
  (not a box on the target tab): the cursor's half of the hovered tab
  picks before/after, and the drop lands exactly there (`moveToIndex`).
  Pure reorder math lives in `centerTabsReorder.ts` (unit-tested).

## Capitalization

- **Title-case for every UI title.** Page titles (`<Page title=…>`),
  tab labels (`label:` in CenterTab arrays), section / card headers
  (`<Section title=…>`, `<Card title=…>`), modal headers, and menu
  items that name a destination (e.g. `New Stream…`) all use
  title case: capitalize the first and last words plus all major
  words (nouns, verbs, adjectives, adverbs, pronouns), and
  lowercase only articles (`a`, `an`, `the`), short prepositions
  (`in`, `on`, `of`, `at`, `to`, `by`, `for`, `with`), and
  coordinating conjunctions (`and`, `but`, `or`, `nor`, `yet`,
  `so`).
  - Right: `Git Dashboard`, `Hook Events`, `Recent Remote
    Branches`, `Ready in This Thread`, `Open in Browser`.
  - Wrong: `Git dashboard`, `Hook events`, `Open in browser`.
- **Sentence-case is OK for inline UI copy** — descriptions,
  hints, button labels that read as commands ("Save", "Cancel"),
  empty-state messages, error toasts. The rule is only for things
  the user reads as a *title*.
- **Mirror the literal across surfaces** — when you change a
  page's title, also update the matching tab label and any
  `deriveDefaultLabel` / `labelByKind` map entry so the renderer
  shows the same string everywhere.

## Empty and error states

- **Every pane has an empty state message** (not just a blank panel).
- **Non-destructive empty states:** "No commits match." rather than
  hiding the filter bar.

## Author badges

- **Runtime auto-filed rows carry a muted `auto` tag** before the
  title (see `AutoAuthorBadge` in `WorkGroupList.tsx`). Human /
  explicit-agent rows render no badge — silence is the dominant path.
  The Work panel header has a `Hide auto` toggle
  (`data-testid="plan-toggle-hide-auto"`) that filters those rows
  out client-side. Preference is local state; no DB persistence
  today.

## Add to agent context

The agent terminal accepts dropped references AND a "Add to agent
context" kebab/menu action; both share one path through
`apps/desktop/src/agent-input-bus.ts` (`insertIntoAgent`) and
`apps/desktop/src/agent-context-ref.ts` (`formatContextMention`).

- **Sources** (anything the user might want to reference): drag rows
  or pills from the Files tree, NotesPane, the WikiActivityBar, the
  Backlinks panel on every Page, the rail HUD recent-files / active
  item / up-next sections, and Code-quality file groups. Set the
  payload with `setContextRefDrag(e, ref)` from
  `apps/desktop/src/agent-context-dnd.ts`. Reuse the same helper and the same
  MIME (`application/x-oxplow-context-ref`) for any new referenceable
  surface — separate from `WORK_ITEM_DRAG_MIME`, which carries the
  reorder payload.
- **Multi-row task drag** is a separate path. Plan-pane
  `WorkGroupList` drag-start enriches the `WORK_ITEM_DRAG_MIME`
  payload with `items: [{id,title,status}, …]` so cross-pane drop
  targets can decode resolved refs without their own task
  lookup. The TerminalPane drop handler accepts both
  `CONTEXT_REF_MIME` (single ref) and `WORK_ITEM_DRAG_MIME`
  (multi-id), iterates the latter, and pastes a space-separated
  chain of mentions in one drop. Helpers:
  `decodeTaskDragRefs` / `dragHasTaskRefs` in
  `apps/desktop/src/agent-context-dnd.ts`.
- **Sink**: `TerminalPane` is the only drop target. It writes through
  `term.paste(text)` so the same xterm input pipeline handles both
  direct and tmux transports — do not branch by transport.
- **Mention shape** (`formatContextMention`):
  - file → `@<workspace-relative path> ` (Claude reads the file
    automatically on the next prompt).
  - note → `@.oxplow/wiki/<slug>.md `.
  - task → `[oxplow task <id>: "<title>" (<status>)] `
    (plain-text reference; agent can fetch via
    `oxplow__get_task`).
  - Always trailing space so the user can keep typing.
- **Kebab parity**: every drag source should also offer "Add to agent
  context" in its kebab menu — keyboard-first users shouldn't have to
  drag. Funnel both paths through the same `insertIntoAgent +
  formatContextMention` calls.
- **Visual feedback**: drop target shows a dashed accent border +
  centered "Drop to add to agent context" overlay only while a
  payload with our MIME is hovering. Foreign drags (text, OS files)
  must not trigger the overlay.
- **Don't fire `recordUsage`** for these gestures — adding to context
  isn't the same as opening the target; the recents list shouldn't
  reorder just because the user told the agent to look at something.
