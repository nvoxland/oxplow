# Usability rules

Things I keep forgetting. Read this before adding any UI.

## Forms

- **Discrete create / edit actions use a modal, never an inline sub-panel.**
  When the user clicks "+ New thread", "New stream", "New work item",
  rename, settings, or any other action that opens a focused little
  form to fill in, render it as a centered modal dialog (backdrop +
  panel + Escape-to-cancel + Enter-to-submit) — not as an inline
  pane that pushes other UI around. Modals win on focus
  (one thing to fill in), consistency (every create flow looks the
  same), and predictability (Escape always cancels, Enter always
  submits, no half-typed state stranded behind some hidden toggle).
  References: `PromptDialog.tsx`, the `NewWorkItemModal` in
  `Plan/PlanPane.tsx`, `StreamRail`'s "New stream" modal, and
  `ThreadRail`'s `CreateThreadModal`. Inline click-to-edit fields on
  existing rows (title, description, priority pickers) are not
  sub-panels and stay inline — the rule is specifically about *create
  a new X* and *edit X in a focused form* flows that warrant a
  dedicated dialog.
- **Never call `window.prompt()`.** Electron disables it — it returns
  `null` synchronously without showing anything, so any code path
  gated on its return value silently no-ops (the user clicks the
  action and nothing happens). Use the themed `PromptDialog` at
  `src/ui/components/PromptDialog.tsx` instead. Same shape as
  `ConfirmDialog`: render conditionally from a `pending*` state, OK /
  Cancel buttons, Escape and backdrop-click cancel, autofocus +
  select. `window.confirm` / `window.alert` aren't disabled but block
  the renderer and don't match the app's visual language — prefer
  `ConfirmDialog` for destructive actions and an inline error / toast
  over `alert` for failures.
- **Every `<button>` needs an explicit `type`.** HTML defaults `<button>` to
  `type="submit"`, which silently submits any enclosing form on click. Use
  `type="button"` for every action button; use `type="submit"` only on the
  form's primary action. Don't rely on the default — it's a tripwire.
- **Enter submits.** Any form with a primary action must submit on Enter from
  any single-line input or select when all required fields are valid. Use a
  real `<form onSubmit=...>` wrapper; the browser handles single-line Enter for
  you. For multi-line textareas, Enter inserts a newline and Cmd/Ctrl+Enter
  submits.
- **Escape cancels.** Modals close on Escape. Inline edit fields (click-to-edit)
  revert on Escape.
- **Modals close via their buttons (and Escape), not backdrop click.** A
  click on the dimmed overlay behind a modal is a no-op — only the modal's
  own OK / Cancel / Close / ✕ buttons (and the Escape key) dismiss it.
  This prevents losing in-progress form state to a stray click outside
  the panel. Lightweight popup palettes that are not modal dialogs
  (command palette, quick-open, inline status/priority pickers) still
  close on outside click — that rule is specific to modal panels with
  action buttons. When the pattern is "Escape → set a cancel latch →
  blur() → onBlur decides commit-vs-revert," the latch **must be a
  `useRef`, not `useState`**. React state updates are async; the blur
  fires synchronously on the same tick and the onBlur closure will still
  see `cancelRequested === false` and silently commit the half-typed
  text. See `WorkItemDetail.tsx`'s `EditableField` and
  `WorkGroupList.tsx`'s `InlineItemRow` for the correct ref-based shape.
- **Disabled submit button when invalid** rather than erroring on submit. Show
  required-field hints inline.
- **Autofocus the first input** when a modal opens.
- **"Save and Another"** for repetitive-entry flows (see the New Work Item
  modal): saves and re-opens the modal with the same category/priority/parent
  pre-filled so the user doesn't re-select them.

## Keyboard

- **Shortcuts go through the menu.** Add new shortcuts to `commands.ts` and
  `keybindings.ts` so they appear in the native menu and help discoverability.
- **Common muscle memory:** Cmd/Ctrl+S save, Cmd/Ctrl+F find, Cmd/Ctrl+P quick
  open, Cmd/Ctrl+Shift+N new work item. Don't collide with these.
- **Plan pane: single-click selects a work-item row (keyboard cursor);
  double-click opens the edit modal.** Enter also opens the modal for the
  selected row. Cmd/Ctrl+click toggles the mark set; Shift+click ranges
  from the selected anchor. A plain click clears marks and moves the
  selection. Marked rows render with a yellow left-stripe + tint. Dragging
  any marked row carries every marked id in `WORK_ITEM_DRAG_MIME.itemIds`
  so drops on BatchRail chips, the backlog chip, or StreamRail move all of
  them at once. Drop targets that handle single-item payloads still work —
  they fall back to `itemId` when `itemIds` is absent.
- **Plan pane: Shift+↑/↓ reorders the selected work item within its own
  status section.** Crossing a section boundary is a deliberate no-op —
  to change status, the user drags (which changes status as a side
  effect). Plain ↑/↓ just moves selection; Enter toggles the detail
  pane; `s`/`p` opens the status/priority pickers.
- **Cmd+K palette listener uses `capture: true`.** Monaco and other focused
  inputs run their own keydown handlers in the bubble phase; capture lets
  the palette fire before any of them. If you add another global shortcut
  that needs to beat an editor, copy that pattern.
- **Palette is the main keyboard lever — keep it populated.** Every new
  menu command in `commands.ts` flows into Cmd+K automatically (the
  palette reads from the same `buildMenuGroups` registry). When adding a
  user-visible action, prefer wiring it as a CommandId over a bespoke
  button so it stays keyboard-reachable. Current entries include
  `stream.new`, `thread.new`, `history.open`, `snapshots.open` alongside
  save/find/quick-open/new-work-item.

## Test-driveability

- **Add a `data-testid` to every new seam a user — or a test — would
  need to drive:** tabs, primary action buttons, form inputs, list
  items, dock panels. Existing conventions:
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
  - `plan-add-points-bar`, `plan-add-commit-point`, `plan-add-wait-point`
  - `files-commit`, `files-commit-message`, `files-commit-submit`
  - `thread-rail-new`, `thread-chip-<threadId>` (chip testid is on the
    outer wrapper that owns the drop handlers, so drag probes can
    target it directly)
  - `menu-item-<item.id>` on every button inside the shared
    `ContextMenu` / `MenuList` — the `MenuItem.id` becomes the
    testid suffix (e.g. `menu-item-workitem.delete`,
    `menu-item-workitem.rename`, `menu-item-workitem.status`,
    `menu-item-workitem.priority` — rename/status/priority mirror the
    inline click / `s` / `p` shortcuts so keyboard-first users don't
    have to hover)
  - `confirm-dialog`, `confirm-dialog-confirm`, `confirm-dialog-cancel`
    on the themed destructive-action `ConfirmDialog`
  - `prompt-dialog`, `prompt-dialog-input`, `prompt-dialog-submit`,
    `prompt-dialog-cancel` on the themed `PromptDialog` (replaces
    `window.prompt`)
  - `center-tab-<id>` on CenterTabs tabs (id is `agent` for the
    agent tab, `file:<path>` for open-file tabs);
    `center-tab-close-<id>` on the × close button
  - `thread-rail-create-input`, `thread-rail-create-submit` on the
    new-thread creation row; `thread-chip-rename-input-<id>` on the
    inline rename input; `thread-chip-promote-<id>` and
    `thread-chip-complete-<id>` on the hover-card actions (also
    reachable via right-click → `menu-item-thread.promote` /
    `menu-item-thread.complete` — keyboard-first users should never
    have to hover to promote a thread)
  These are load-bearing for `tests-e2e/` — don't rename casually.

## Feedback

- **Show loading state** for any operation >150ms.
- **Show counts** where relevant (e.g., "24 / 500 commits" in the history
  filter).
- **Don't silently drop edits.** Failed operations must surface an error near
  the affected control, not only in the toast area.

## Drag and drop

- **Highlight the drop target** (dashed border + accent glow) whenever a
  compatible drag enters it. Clear the highlight on leave/drop.
- **Use a custom MIME type** for internal drags so foreign drags (files, text)
  don't accidentally trigger app drops. Existing MIMEs:
  `WORK_ITEM_DRAG_MIME` (work-item reorder) in
  `src/ui/components/ThreadRail.tsx`, and `CONTEXT_REF_MIME` ("Add to
  agent context") in `src/ui/agent-context-dnd.ts`. Add a new MIME
  rather than overloading an existing one.

## Empty and error states

- **Every pane has an empty state message** (not just a blank panel).
- **Non-destructive empty states:** "No commits match." rather than hiding the
  filter bar.

## Author badges

- **Runtime auto-filed rows carry a muted `auto` tag** before the title
  (see `AutoAuthorBadge` in `WorkGroupList.tsx`). Human / explicit-agent
  rows render no badge — silence is the dominant path. The Work panel
  header has a `Hide auto` toggle (`data-testid="plan-toggle-hide-auto"`)
  that filters those rows out client-side. Preference is local state;
  no DB persistence today.

## Context menus (right-click)

- **Right-click is preferred over visible icons** for per-item destructive
  actions (delete, etc.) to reduce visual noise in lists.
- Close on outside click, scroll, window resize.

## Add to agent context

The agent terminal accepts dropped references AND a "Add to agent context"
right-click action; both share one path through
`src/ui/agent-input-bus.ts` (`insertIntoAgent`) and
`src/ui/agent-context-ref.ts` (`formatContextMention`).

- **Sources** (anything the user might want to reference): drag rows or
  pills from the Files tree, NotesPane, and the recent-activity bar
  above the agent. Set the payload with `setContextRefDrag(e, ref)` from
  `src/ui/agent-context-dnd.ts`. Reuse the same helper and the same
  MIME (`application/x-oxplow-context-ref`) for any new referenceable
  surface — separate from `WORK_ITEM_DRAG_MIME`, which carries the
  reorder payload.
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
- **Right-click parity**: every drag source should also offer "Add to
  agent context" in its context menu — keyboard-first users shouldn't
  have to drag. Funnel both paths through the same
  `insertIntoAgent + formatContextMention` calls.
- **Visual feedback**: drop target shows a dashed accent border +
  centered "Drop to add to agent context" overlay only while a payload
  with our MIME is hovering. Foreign drags (text, OS files) must not
  trigger the overlay.
- **Don't fire `recordUsage`** for these gestures — adding to context
  isn't the same as opening the target; the recents list shouldn't
  reorder just because the user told the agent to look at something.
