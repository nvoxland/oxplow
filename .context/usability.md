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
  revert on Escape.
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
