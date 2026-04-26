# Usability rules

Things I keep forgetting. Read this before adding any UI.

> **IA redesign — phases 0–7 fully shipped.** Modal `ConfirmDialog`
> and `PromptDialog` chrome was retired in favor of inline patterns;
> the right-click → `ContextMenu` reflex was replaced by visible
> kebab `⋯` buttons on each row; per-stream and per-thread settings
> ship as Page tabs (`StreamSettingsPage`, `ThreadSettingsPage`); new-
> stream and new-work-item flows ship as Page tabs (`NewStreamPage`,
> `NewWorkItemPage`); snapshot- and commit-detail Slideover wrappers
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
  `src/ui/components/InlineEdit.tsx`; `WorkItemDetail`'s
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
  Page tabs (`NewStreamPage`, `NewWorkItemPage`, the `Stream/Thread`
  settings pages); cross-page detail openings (snapshot, commit,
  branch rename, file commit) ship as Slideovers. The remaining
  legacy hand-rolled modal chrome inside `PlanPane.tsx`'s
  `NewWorkItemModal` only backs the edit-double-click flow — do not
  add new modal call sites; route new flows through pages or
  slideovers. The page pattern to copy is
  `src/ui/pages/SettingsPage.tsx` — full Page tab, no backdrop.
- **Never call `window.prompt()`.** Electron disables it — it returns
  `null` synchronously without showing anything, so any code path
  gated on its return value silently no-ops. Use `InlineEdit` (for
  click-to-edit) or `InlinePromptStrip` (for new-X flows that need a
  target-path entry) instead. `window.confirm` / `window.alert` block
  the renderer; prefer `InlineConfirm` for destructive actions on a
  row/button and `showToast({ message, onUndo })` for fire-and-undo
  destructives that aren't tied to a specific row.
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
  them. Carry this convention forward when New Work Item migrates to
  a page (phase 5e).

## Destructive actions

- **Per-row destructives use `InlineConfirm`** at
  `src/ui/components/InlineConfirm.tsx`. First click on the trigger
  swaps to a `[Confirm] [Cancel]` pair in the same horizontal real
  estate. The Confirm button auto-focuses; Escape, blur (outside the
  pair), or Cancel reverts. Examples in tree: Restore button on each
  file row in `SnapshotsPanel.tsx`'s detail pane; Delete button on
  `WaitPointRow.tsx`; Force-delete button in `BranchPicker.tsx`'s
  manage flow.
- **Non-row-anchored destructives fire immediately and surface an
  Undo toast.** Use `showToast({ message, onUndo })` from
  `src/ui/components/toastStore.ts`. The toast auto-dismisses after
  ~7s and the [Undo] button calls the supplied callback. Mount the
  `<UndoToastStack />` once near the app root (already done in
  `App.tsx`). When the action is genuinely irreversible (delete a
  work item permanently) push a toast without `onUndo` so the user
  still sees confirmation feedback even if they can't undo. Don't
  block the renderer with a centered confirm modal.
- **Closing a dirty file tab** is fire-and-undo: the close completes
  immediately and a toast offers Undo (which restores both the saved
  buffer and the unsaved draft). See `App.tsx` →
  `handleCloseOpenFile`.

## Per-row actions (was right-click menus)

- **Visible kebab `⋯` button per row, not right-click.** The shared
  primitive is `src/ui/components/Kebab.tsx` (button + `ContextMenu`
  popover anchored under the button). The popover keeps the same
  `MenuItem[]` payload as the legacy right-click menus — call sites
  swap their handler, the menu items themselves are unchanged.
- The `ContextMenu` popover at `src/ui/components/ContextMenu.tsx` is
  still in use as the popover renderer; just don't open it from a
  raw `onContextMenu` handler in new code. If you find a surface
  that still does, that's a phase-5c continuation site — wire it
  through Kebab or pass a rect-based callback that opens the same
  menu.
- **`menu-item-<item.id>` testids** stay on every button inside the
  shared `MenuList` — the `MenuItem.id` becomes the testid suffix
  (e.g. `menu-item-workitem.delete`,
  `menu-item-workitem.rename`).
- Close on outside click, scroll, window resize.

## Keyboard

- **Shortcuts go through the menu.** Add new shortcuts to
  `commands.ts` and `keybindings.ts` so they appear in the native
  menu and help discoverability.
- **Common muscle memory:** Cmd/Ctrl+S save, Cmd/Ctrl+F find,
  Cmd/Ctrl+P quick open, Cmd/Ctrl+Shift+N new work item. Don't
  collide with these.
- **Plan pane: single-click selects a work-item row (keyboard
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
  `src/ui/components/Plan/SelectionActionBar.tsx`. Buttons mirror the
  marked-set right-click menu — Change status / Change priority /
  Add to agent context / Delete — plus a Clear button. The bar reads
  the existing marked-set state in `PlanPane`; there is no separate
  store. Pure helpers (`shouldShowSelectionActionBar`,
  `summarizeSelection`) are exported for tests.
- **Plan pane: Shift+↑/↓ reorders the selected work item within its
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
  include `stream.new`, `thread.new`, `history.open`, `snapshots.open`
  alongside save/find/quick-open/new-work-item.

## Test-driveability

- **Add a `data-testid` to every new seam a user — or a test —
  would need to drive:** tabs, primary action buttons, form inputs,
  list items, dock panels. Existing conventions:
  - `dock-tab-<id>` / `dock-panel-<id>` on DockShell rail + content
  - `file-tree-entry-<path>` on FileTree nodes (plus `data-kind` and,
    for dirs, `data-expanded`)
  - `monaco-host` on the editor container, `data-file-path=<path>`
  - `plan-new-task`, `work-item-title`, `work-item-priority`,
    `work-item-description`, `work-item-acceptance`, `work-item-save`,
    `work-item-save-another`, `work-item-cancel`
  - `command-palette-input`
  - `plan-pane` (the keydown-listening wrapper — focus this before
    dispatching keyboard probes, otherwise the listener misses them)
  - `plan-add-points-bar`, `plan-add-commit-point`,
    `plan-add-wait-point`
  - `files-commit`, `files-commit-message`, `files-commit-submit`
  - `thread-rail-new`, `thread-chip-<threadId>` (chip testid is on
    the outer wrapper that owns the drop handlers, so drag probes
    can target it directly), `thread-chip-kebab-<id>` on the kebab
    button inside each chip
  - `stream-tab-kebab-<id>` on the kebab button inside each stream
    tab; `center-tab-kebab-<id>` on each center-tab kebab
  - `work-item-row-kebab-<id>` on each work-item row's kebab
  - `menu-item-<item.id>` on every button inside the shared
    `ContextMenu` / `MenuList` — the `MenuItem.id` becomes the
    testid suffix (e.g. `menu-item-workitem.delete`,
    `menu-item-workitem.rename`, `menu-item-workitem.status`,
    `menu-item-workitem.priority` — rename/status/priority mirror
    the inline click / `s` / `p` shortcuts so keyboard-first users
    don't have to hover)
  - `undo-toast-stack`, `undo-toast-<id>`,
    `undo-toast-action-<id>`, `undo-toast-dismiss-<id>` on the
    Undo toast bottom-stack
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

- **Highlight the drop target** (dashed border + accent glow) whenever
  a compatible drag enters it. Clear the highlight on leave/drop.
- **Use a custom MIME type** for internal drags so foreign drags
  (files, text) don't accidentally trigger app drops. Existing MIMEs:
  `WORK_ITEM_DRAG_MIME` (work-item reorder) in
  `src/ui/components/ThreadRail.tsx`, and `CONTEXT_REF_MIME`
  ("Add to agent context") in `src/ui/agent-context-dnd.ts`. Add a
  new MIME rather than overloading an existing one.

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
`src/ui/agent-input-bus.ts` (`insertIntoAgent`) and
`src/ui/agent-context-ref.ts` (`formatContextMention`).

- **Sources** (anything the user might want to reference): drag rows
  or pills from the Files tree, NotesPane, the WikiActivityBar, the
  Backlinks panel on every Page, the rail HUD recent-files / active
  item / up-next sections, and Code-quality file groups. Set the
  payload with `setContextRefDrag(e, ref)` from
  `src/ui/agent-context-dnd.ts`. Reuse the same helper and the same
  MIME (`application/x-oxplow-context-ref`) for any new referenceable
  surface — separate from `WORK_ITEM_DRAG_MIME`, which carries the
  reorder payload.
- **Multi-row work-item drag** is a separate path. Plan-pane
  `WorkGroupList` drag-start enriches the `WORK_ITEM_DRAG_MIME`
  payload with `items: [{id,title,status}, …]` so cross-pane drop
  targets can decode resolved refs without their own work-item
  lookup. The TerminalPane drop handler accepts both
  `CONTEXT_REF_MIME` (single ref) and `WORK_ITEM_DRAG_MIME`
  (multi-id), iterates the latter, and pastes a space-separated
  chain of mentions in one drop. Helpers:
  `decodeWorkItemDragRefs` / `dragHasWorkItemRefs` in
  `src/ui/agent-context-dnd.ts`.
- **Sink**: `TerminalPane` is the only drop target. It writes through
  `term.paste(text)` so the same xterm input pipeline handles both
  direct and tmux transports — do not branch by transport.
- **Mention shape** (`formatContextMention`):
  - file → `@<workspace-relative path> ` (Claude reads the file
    automatically on the next prompt).
  - note → `@.oxplow/notes/<slug>.md `.
  - work-item → `[oxplow work-item <id>: "<title>" (<status>)] `
    (plain-text reference; agent can fetch via
    `oxplow__get_work_item`).
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
