# Terminal

Each thread has an **Agent** tab that wraps a tmux pane running
the thread's assigned agent — Claude Code, Codex, or OpenCode — in the stream's
worktree. The pane survives oxplow restarts because tmux owns the process —
killing oxplow doesn't kill your in-flight agent.

## Agent terminal

The project-level `agents` list controls which agents are available;
the first enabled agent is the default for new threads. When both are
enabled, choose an agent while creating the thread. The assignment is
fixed after creation. See [Agents](agents.md) for configuration and
runtime details.

Type a prompt, hit enter, and watch the agent work. The thread tab's
status dot flips yellow (working) or red (waiting on you).

Per-thread bits worth knowing:

- **Lifecycle hook integration.** Each agent loads its own
  generated hook configuration. Stop and tool events drive the
  in-progress audit, filing enforcement, and snapshot tracking. Oxplow
  installs the per-agent files under `.oxplow/runtime/` automatically.
- **Write guard.** If the active thread is read-only,
  Edit / Write / MultiEdit / NotebookEdit are denied at the
  hook level. The agent can still read, search, and answer
  questions.
- **Filing enforcement.** Even on the writer thread, edits are
  denied if there's no `in_progress` task. The agent
  files one and re-issues.
- **Per-stream isolation.** The agent's CWD is the stream's
  worktree. It cannot see other streams' working trees.

## Tab actions

Right-click a terminal's tab in the tab strip to rename or close
it. Copy is automatic on selection and paste is the platform
shortcut — see [Keybindings](../reference/keybindings.md).

## Drag-to-add-context

Drag rows from the rail's Work and Go To panes, backlinks lists,
task rows, or the file tree
onto the agent terminal to inject them into the agent's
context. Multi-select drag works for task lists.

## Shells

Open a shell tab from the **+** in any tab strip — it's just a
pty in the stream's worktree. Use it for `git status`, `npm
install`, running tests. Multiple shells per thread are fine;
they share the worktree but are otherwise independent.

## tmux mode

The agent terminal runs in tmux by default so detaching and
reattaching survives oxplow restarts. Toggle it from the agent
tab's context menu if you want a plain pty instead. Sessions are
per-thread.

## Copy / paste

Standard terminal copy/paste. Selecting text copies on release.
`Cmd/Ctrl+V` pastes. The pty is real, the shell is real, your
`.zshrc` works.
