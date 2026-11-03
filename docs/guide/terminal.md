# Terminal

Terminal agent sessions run their selected harness in a PTY in the
stream's worktree. A thread can have several sessions or none; each
session has its own tab and resume identity. See [Agents](agents.md).

## Agent terminal

Open a terminal session from the session picker or **New Agent Session…**,
then type a prompt and press Enter. Switching tabs, threads, or streams
leaves the session running. The session's status contributes to the
thread and stream status indicators.

Harness hooks record activity and turn boundaries. Snapshots, test
evidence, usage, and optional efforts help explain what happened. There
is no task-filing gate or Stop audit requiring an agent to keep working.

Non-writer threads may not change their stream's worktree. Other project
records remain available. This ownership rule is not a general sandbox
around the agent's shell.

## Context and tab actions

Use the session's tab actions to rename or close it. Selecting terminal
text copies it; paste uses the platform shortcut. See
[Keybindings](../reference/keybindings.md).

To give the agent something to read, right-click a row, a backlink or a
selection and choose **Add to Context**, or use a page's ☰ menu. It goes
in the thread's context basket, and the next prompt you send to any agent
in the thread carries it. Nothing is sent on its own.

## Shells

Open a shell tab from the **+** in the tab strip. It is a separate PTY in
the stream's worktree, useful for inspecting git state, installing
dependencies, and running tests. Multiple shells share that worktree.
