# Editor and pages

The center of the window is a stack of **page tabs**. Most
pages are one of: file, diff, work item, wiki note,
code-quality finding, dashboard, settings, agent terminal, or
a panel-style index page (Files, Notes, Code quality, Local
history, Git history, etc.).

## Tabs are per-thread

Each thread owns its own set of open tabs and active tab.
Switching threads restores its tab set; the agent terminal is
always available per thread. Switching streams swaps to the
selected thread of the new stream.

## Browser-style navigation

Page tabs have **back / forward** buttons. Click a backlink
inside a page and the current tab navigates in place (with the
prior page pushed onto the back stack). Cmd/Ctrl-click,
middle-click, or right-click on a link to open in a new tab
instead.

Bookmarks let you pin pages to the rail in three scopes —
thread, stream, or global. The rail HUD surfaces the merged set.

## Files page

The **Files** page is a tree-style file browser scoped to the
current stream's worktree. It will not climb outside the
workspace root: even if your worktree happens to live inside a
larger repo, the file browser stays inside the stream's
checkout.

Right-click (or row kebab) for:

- Open
- Open to side
- Local History
- Reveal in OS file browser
- Add to agent context

Git decorations show file state with colored badges: **A**
added, **M** modified, **D** deleted, **R** renamed, **C**
conflicted.

## File tabs (Monaco)

Files open as Monaco editor tabs. Standard Monaco features
work: multi-cursor, find/replace, indentation rules, bracket
matching, code folding. Editors are pinned to dark mode —
oxplow is dark-only on purpose.

The blame margin shows the commit and author that last touched
each line; hover for the message and click the per-row kebab to
open that commit's page.

## Diff and commit pages

Diff tabs (`diff:<path>|from|to`) open the Monaco diff editor
side-by-side. Click a commit anywhere in the UI — git history,
blame, a wikilink — to open its **Git commit** page with the
full message, files changed, and per-file diff.

The **Uncommitted changes** page is a stats-focused view of the
working tree: per-file M/A/D/R/U + total +/-, collapsible
folder rollup, **Commit all** action. Use it to commit changes
without dropping into a terminal.

## Git dashboard / Git history

- **Git dashboard** — branch header (current + upstream + ahead
  / behind + push), uncommitted mini-card, last 5 commits on
  the current branch, worktrees row with per-row "Merge into
  current", recent remote branches with per-row pull/push.
- **Git history** — full commit graph for the current branch
  (or all branches), with detail pane.

All ref-mutating actions confirm the exact `git` command before
running.

## LSP

LSP integration runs as a daemon-managed bridge: the editor
talks to language servers through Monaco's LSP client, scoped
to the stream's worktree. Hover, go-to-definition, and
find-references work against the workspace root.

The agent can also call LSP via MCP tools (`lsp_definition`,
`lsp_hover`, `lsp_references`, `lsp_diagnostics`) — same
servers as the editor, so answers stay consistent.

## What's deliberately missing

- **Extensions.** Oxplow doesn't host VS Code extensions.
- **Multi-root workspaces.** One stream, one root.
- **Light mode.** Dark only.

If you want a richer editor experience, you can still open the
project in your usual IDE alongside oxplow — the worktree on
disk is just files.
