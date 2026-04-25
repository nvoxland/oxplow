# Editor

The center pane is a Monaco-based editor wrapped in a custom
React shell. It's not the full VS Code workbench — just the
editor, with the IDE features oxplow needs layered on top.

## File browser

A custom React tree, scoped to the current stream's worktree. It
will not climb outside the workspace root: even if your stream's
worktree happens to live inside a larger repo, the file browser
stays inside the stream's checkout.

Right-click a file for:

- Open
- Open to side
- Local History
- Reveal in OS file browser

## Editor tabs

Tabs are per-stream. Switching streams swaps the tab set. Open
files persist across restarts.

Standard Monaco features all work: multi-cursor, find/replace,
keyboard shortcuts, indentation rules, bracket matching, code
folding.

## Git decorations

Files in the browser show their git state with colored badges:

- **A** — added (untracked)
- **M** — modified
- **D** — deleted
- **R** — renamed
- **C** — conflicted

Edited files show a dirty dot in their tab. Save with the
standard shortcut.

## Blame overlay

Toggle the blame overlay to see, per line, the commit and author
that last touched it. Useful when reviewing what the agent
changed against what was already there.

## Diff editor

When you click an entry in Local History, the file opens in a
diff editor — old version on the left, current on the right.
Same for `git diff`-style views opened from the SCM-ish parts of
the UI.

## LSP

LSP integration runs as a daemon-managed bridge: the editor
talks to language servers through Monaco's LSP client, scoped to
the stream's worktree. Hover, go-to-definition, and find-
references work against the workspace root.

The agent can also call LSP via MCP tools (`lsp_definition`,
`lsp_hover`, `lsp_references`, `lsp_diagnostics`). It uses the
same servers the editor does — answers stay consistent.

## What's deliberately missing

- **Extensions.** Oxplow doesn't host VS Code extensions. The
  feature set is what's built in.
- **Multi-root workspaces.** One stream, one root.
- **Settings UI.** Settings live in `settings.json`-style
  files; see [Settings](../reference/settings.md).

If you want a richer editor experience, you can still open the
project in your usual IDE alongside oxplow — the worktree on
disk is just files.
