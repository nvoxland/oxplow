# Terminal

The bottom pane is a pty terminal. Each stream gets its own.
There are two things you'll do with it: drive the agent, and
run shells.

## Agent terminal

By default, the bottom pane runs Claude Code in the stream's
worktree. Type your prompt, hit enter, watch the agent work.

A few oxplow-specific bits:

- **Stop hook integration.** When the agent finishes a turn,
  oxplow's Stop hook fires. That's how snapshots and the
  open-turn row in the Plan pane get populated. You don't need
  to do anything — Claude Code is configured automatically when
  the stream is created.
- **Write guard.** If the active thread is read-only (not the
  writer), file edits are denied at the hook level. The agent
  can read, search, and answer questions, but its edit tools
  return an error. This is enforced in the hook, not in the UI.
- **Per-stream isolation.** The agent's CWD is the stream's
  worktree. It cannot see other streams' working trees.

## Shell terminals

Click **+** in the terminal pane header to open a regular shell
in the same worktree. Use it for `git status`, `npm install`,
running tests, whatever — it's just a shell.

Multiple shells per stream are fine. They share the worktree but
are otherwise independent processes.

## tmux mode

For long-running agent sessions, oxplow can wrap the agent
terminal in a tmux session so detaching and reattaching survives
oxplow restarts. The session is per-stream and persists between
launches. Toggle it from the terminal pane header.

The tmux integration is invisible most of the time — it just
means killing oxplow doesn't kill your in-flight agent.

## Copy / paste

Standard terminal copy/paste behavior. Selecting text copies on
release. `Cmd/Ctrl+V` pastes. There's no exotic key remapping —
the pty is real, the shell is real, your `.zshrc` works.
