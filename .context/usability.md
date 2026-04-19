# Usability rules

Things I keep forgetting. Read this before adding any UI.

## Forms

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
  revert on Escape. When the pattern is "Escape → set a cancel latch →
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
- **Cmd+K palette listener uses `capture: true`.** Monaco and other focused
  inputs run their own keydown handlers in the bubble phase; capture lets
  the palette fire before any of them. If you add another global shortcut
  that needs to beat an editor, copy that pattern.

## Test-driveability

- **Add a `data-testid` to every new seam a user — or a test — would
  need to drive:** tabs, primary action buttons, form inputs, list
  items, dock panels. Existing conventions:
  - `dock-tab-<id>` / `dock-panel-<id>` on DockShell rail + content
  - `file-tree-entry-<path>` on FileTree nodes (plus `data-kind` and,
    for dirs, `data-expanded`)
  - `monaco-host` on the editor container, `data-file-path=<path>`
  - `plan-new-work-item`, `work-item-title`, `work-item-priority`,
    `work-item-description`, `work-item-acceptance`, `work-item-save`,
    `work-item-save-another`, `work-item-cancel`
  - `command-palette-input`
  - `plan-add-points-bar`, `plan-add-commit-point`, `plan-add-wait-point`
  - `files-commit`, `files-commit-message`, `files-commit-submit`
  - `batch-rail-new`, `batch-chip-<batchId>` (chip testid is on the
    outer wrapper that owns the drop handlers, so drag probes can
    target it directly)
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
  don't accidentally trigger app drops. See `WORK_ITEM_DRAG_MIME` in
  `src/ui/components/BatchRail.tsx`.

## Empty and error states

- **Every pane has an empty state message** (not just a blank panel).
- **Non-destructive empty states:** "No commits match." rather than hiding the
  filter bar.

## Context menus (right-click)

- **Right-click is preferred over visible icons** for per-item destructive
  actions (delete, etc.) to reduce visual noise in lists.
- Close on outside click, scroll, window resize.
