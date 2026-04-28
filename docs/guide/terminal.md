# Terminal

Each thread has an **Agent** tab that wraps a tmux pane running
Claude Code (or `copilot`) in the stream's worktree. The pane
survives oxplow restarts because tmux owns the process —
killing oxplow doesn't kill your in-flight agent.

## Agent terminal

Default kind: Claude Code. Type a prompt, hit enter, watch the
agent work. The thread tab's status dot flips yellow (working)
or red (waiting on you).

Per-thread bits worth knowing:

- **Stop hook integration.** Every Stop fires the runtime's
  hook — that's how the in-progress audit, filing-enforcement,
  and snapshot tracking get driven. You don't configure
  anything; oxplow installs a per-project Claude Code plugin
  under `.oxplow/runtime/claude-plugin/` automatically.
- **Write guard.** If the active thread is read-only,
  Edit / Write / MultiEdit / NotebookEdit are denied at the
  hook level. The agent can still read, search, and answer
  questions.
- **Filing enforcement.** Even on the writer thread, edits are
  denied if there's no `in_progress` work item. The agent
  files one and re-issues.
- **Per-stream isolation.** The agent's CWD is the stream's
  worktree. It cannot see other streams' working trees.

## Header kebab

The agent tab's header kebab carries:

- Copy / Paste / Clear (replaces the legacy xterm right-click
  menu — oxplow uses kebabs, not context menus)
- Restart agent
- Toggle tmux mode
- Switch agent kind (Claude Code / copilot)

## Drag-to-add-context

Drag rows from the rail's recent files, active item, up-next,
backlinks lists, work-item rows, or code-quality file groups
onto the agent terminal to inject them into the agent's
context. Multi-select drag works for work-item lists.

## Shells

Open a shell tab from the **+** in any tab strip — it's just a
pty in the stream's worktree. Use it for `git status`, `npm
install`, running tests. Multiple shells per thread are fine;
they share the worktree but are otherwise independent.

## tmux mode

The agent terminal runs in tmux by default so detaching and
reattaching survives oxplow restarts. Toggle from the agent
tab kebab if you want a plain pty instead. Sessions are
per-thread.

## Copy / paste

Standard terminal copy/paste. Selecting text copies on release.
`Cmd/Ctrl+V` pastes. The pty is real, the shell is real, your
`.zshrc` works.
