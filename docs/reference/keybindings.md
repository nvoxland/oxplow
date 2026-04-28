# Keybindings

Oxplow follows two ironclad UI rules everywhere they make sense:

- **Enter submits.** Whatever input you're in, Enter is the
  affirmative action.
- **Escape cancels.** Whatever input you're in, Escape gets you
  out without applying changes.

Beyond those, the keymap is intentionally small — the product
has a directory of pages, not a thousand commands.

## Global

| Action | macOS | Windows / Linux |
|---|---|---|
| Open project | `Cmd+O` | `Ctrl+O` |
| New stream | `Cmd+Shift+N` | `Ctrl+Shift+N` |
| Switch stream | `Cmd+Alt+→` / `←` | `Ctrl+Alt+→` / `←` |
| Quick open / command palette | `Cmd+K` | `Ctrl+K` |
| Find in current file | `Cmd+F` | `Ctrl+F` |
| Find in workspace | `Cmd+Shift+F` | `Ctrl+Shift+F` |
| Back / forward in active tab | `Cmd+[` / `Cmd+]` | `Ctrl+[` / `Ctrl+]` |

## Tabs and pages

| Action | Key |
|---|---|
| Close active tab | `Cmd/Ctrl+W` |
| Open link in new tab | `Cmd/Ctrl+Click`, middle-click, or right-click |
| Bookmark current page | Toggle from the page's nav bar |

## Work items (Plan / Backlog / Done / Archived pages)

| Action | Key |
|---|---|
| File new work item | inline new-row at the top of the list — type and press Enter |
| Edit selected item | `Enter` |
| Mark `done` (close) | `Cmd/Ctrl+Enter` |
| Cancel edit | `Escape` |
| Delete | row kebab → Delete (no single-key shortcut) |
| Reorder | drag, or `Alt+↑` / `Alt+↓` |

Destructive actions (delete, archive, cancel) are accessed via
the row kebab, not single keystrokes — this is deliberate, to
keep the queue from losing data to a stray keypress. The whole
product uses kebabs instead of right-click context menus for
the same reason: discoverability over chord memorization.

## Editor (Monaco)

Standard Monaco bindings apply. The most-used:

| Action | macOS | Windows / Linux |
|---|---|---|
| Save | `Cmd+S` | `Ctrl+S` |
| Multi-cursor | `Cmd+Click` | `Ctrl+Click` |
| Go to definition | `F12` | `F12` |
| Go to line | `Cmd+G` | `Ctrl+G` |
| Open Local History for current file | `Cmd+Alt+H` | `Ctrl+Alt+H` |

## Drop targets

When you drag a file, work item, or rail entry, the target
zone highlights as you drag. Drop highlighting is *the* signal
that a drop will work — if you don't see a highlight, the drop
isn't supported there.

The agent terminal accepts drag-to-add-context from work-item
rows, file rows, the rail's recent-files / active item / up-next
lists, backlinks entries, and code-quality file groups.

## Why so few

VS Code has hundreds of keybindings. Oxplow has a few dozen on
purpose. The product is small enough to learn the rail and the
agent does most of the typing — keybindings are for navigation
and submit/cancel, not for the long tail of editor commands.
