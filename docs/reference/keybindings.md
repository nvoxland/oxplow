# Keybindings

Oxplow follows two ironclad UI rules everywhere they make sense:

- **Enter submits.** Whatever input you're in, Enter is the
  affirmative action.
- **Escape cancels.** Whatever input you're in, Escape gets you
  out without applying changes.

Beyond those, the keymap is intentionally small.

## Global

| Action | macOS | Windows / Linux |
|---|---|---|
| Open project | `Cmd+O` | `Ctrl+O` |
| New stream | `Cmd+Shift+N` | `Ctrl+Shift+N` |
| Switch stream (rail tab cycle) | `Cmd+Alt+→` / `←` | `Ctrl+Alt+→` / `←` |
| Toggle Plan pane | `Cmd+1` | `Ctrl+1` |
| Toggle file browser | `Cmd+2` | `Ctrl+2` |
| Toggle terminal pane | `Cmd+\`` | `Ctrl+\`` |
| Quick open file | `Cmd+P` | `Ctrl+P` |
| Find in current file | `Cmd+F` | `Ctrl+F` |
| Find in workspace | `Cmd+Shift+F` | `Ctrl+Shift+F` |

## Work queue (Plan pane)

| Action | Key |
|---|---|
| File new work item | `N` while focused on the queue |
| Edit selected item | `Enter` |
| Mark `human_check` (close) | `Cmd/Ctrl+Enter` |
| Cancel edit | `Escape` |
| Delete (with confirm) | Right-click → Delete |
| Reorder | drag, or `Alt+↑` / `Alt+↓` |

Destructive actions (delete, archive, cancel) are accessed via
right-click, not single keystrokes — this is deliberate, to keep
the queue from losing data to a stray keypress.

## Editor

Standard Monaco bindings apply. The most-used:

| Action | macOS | Windows / Linux |
|---|---|---|
| Save | `Cmd+S` | `Ctrl+S` |
| Multi-cursor | `Cmd+Click` | `Ctrl+Click` |
| Go to definition | `F12` | `F12` |
| Go to line | `Cmd+G` | `Ctrl+G` |
| Toggle blame overlay | `Cmd+Alt+B` | `Ctrl+Alt+B` |
| Open Local History for current file | `Cmd+Alt+H` | `Ctrl+Alt+H` |

## Drop targets

When you drag a file from the file browser onto the editor, the
target zone highlights as you drag. Drop highlighting is *the*
signal that a drop will work — if you don't see a highlight,
the drop isn't supported there.

Same pattern in the Plan pane: dropping a work item between two
others highlights the gap.

## Why so few

VS Code has hundreds of keybindings. Oxplow has a few dozen on
purpose. The product is small enough to learn the menu, and the
agent does most of the typing — keybindings are for navigation,
not for the long tail of editor commands.
