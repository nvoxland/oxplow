# Agent control

Oxplow observes agent activity, supplies context and guidance, and exposes
shared data and actions. It does not require task bookkeeping before an
edit or keep an agent working after Stop to reconcile the work queue.

## Human input and recorded AI computations

Interactive terminal input comes from the person. Chat prompts are sent
when the person submits the prompt box; visible oxplow context can
accompany that prompt. Neither path automatically types prompts or
answers into a coding agent.

Oxplow can separately call a configured model's API for recorded
computations such as classification, scoring, summarization, and
extraction. Those computations contribute data; they are distinct from
driving the agent doing the development work. See [AI models](ai-models.md).

## Hooks and observations

Each harness translates its activity into oxplow's shared record. Turns,
tool calls, status, snapshots, test evidence, and usage can be associated
with the session and thread that produced them.

At turn end, oxplow records the turn and requests a snapshot. Optional
effort policies group the observed activity. Guidance is advisory:
there is no filing-before-editing gate or in-progress-task Stop audit.

## Writer boundary

Each stream has one writer thread. Other threads may research, review,
and record work items, notes, knowledge, and other project records, but
may not change that stream's worktree. Sessions inherit their thread's
writer role. If several sessions run in the writer thread, the person
coordinates their concurrent work.

This is an enforced worktree ownership rule, not a general operating
system sandbox around the coding agent. It does not make every shell
command or every file outside the stream read-only.

## MCP and commands

Agents read published models through SQL and use commands for changes.
Commands validate inputs and apply actor permissions, confirmation,
audit, and transaction rules. The session/thread context identifies
where a call came from.

Wiki writes use `oxplow.knowledge.write_page`, which maintains the page
and its references together. Capturing understanding works from
non-writer threads too; it needs an active knowledge store to retain it.

## What you choose

- Available agent harnesses and defaults for new sessions.
- Which sessions a thread uses; a thread can have none or several.
- Which thread is the stream's writer.
- Optional work-item, effort-policy, and knowledge implementations.
- Which permissions a repository-supplied extension gets: you approve them
  when you deploy it, and a deploy that adds one asks again.

See [Agents](agents.md) and [Extensions](../reference/extensions.md).
Commits remain user-driven. Commit/wait queue markers and automatic
queue progression are removed.
