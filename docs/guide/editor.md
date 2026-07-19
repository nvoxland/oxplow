# Editor and pages

The center of the window is a stack of **page tabs**. Most
pages are one of: file, diff, task, wiki page,
finding, dashboard, metric, settings, agent terminal, or
a panel-style index page (Files, Wiki, Local History, Git
History, Metrics, Dashboards, etc.).

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

Right-click a row for:

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
each line; hover for the message and right-click the row to
open that commit's page.

The chrome carries a per-snapshot history dropdown that lists
every local snapshot of the open file, in descending order,
labeled with the pinned commit (or `uncommitted`) and any
in-flight / just-completed effort. Pick an entry to jump to
the snapshot detail page scoped to that file — see
[Local History](local-history.md#file-page-integration).

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

Servers are installed explicitly — from Settings → **Language
Servers**, or by the agent via `lsp_install_server`. They land in
`.oxplow/lsp/` and the proxy hands the right binary to whichever
stream asked. Nothing installs merely because you opened a file.
See [Settings](../reference/settings.md#lsp-servers).

The agent can also reach LSP through the same bridge — same
servers as the editor, so answers stay consistent.

## Change Analysis

A separate top-level page (++cmd+p++ → **Change Analysis**)
for understanding diffs. Ranks files by interestingness, supports
drilldown by extension / directory / status, and shows per-function
before/after metrics. See [Change Analysis](change-analysis.md).

## Duplication findings

Duplicate-block detection runs as a Rust tree-sitter scanner
against the worktree, persisted in the project SQLite store. It
surfaces inside [Change Analysis](change-analysis.md) — on the
diff view, Uncommitted, and commit pages — rather than as a
standalone index; an individual block opens as its own page with
a *Jump to source* action.

Complexity and other per-function metrics feed the Change
Analysis cards directly. There is no separate Code quality page.

## What's deliberately missing

- **Extensions.** Oxplow doesn't host VS Code extensions.
- **Multi-root workspaces.** One stream, one root.
- **Light mode.** Dark only.

If you want a richer editor experience, you can still open the
project in your usual IDE alongside oxplow — the worktree on
disk is just files.
