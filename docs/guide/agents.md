# Agents

A thread is a line of work, not the home of one fixed agent. It can have
no agent session or several. Sessions have their own agent implementation,
process, resume identity, activity, and status; they share the thread's
context, tabs, and writer role.

Oxplow ships harness integrations for Claude Code, Codex, OpenCode, and
ACP, with implementations declared by its foundation extension. An ACP
session uses a structured conversation; terminal harnesses use a PTY in
the stream's worktree.

## Choose available harnesses

Project Settings or `.oxplow/project.yaml` can limit and order the
harnesses offered for new sessions:

```yaml
agents:
  - claude
  - codex
  - opencode
```

Names are registered harness keys. Omitting the setting offers the
registered harnesses; an explicit list must be nonempty and contain no
duplicates. Its first available entry is the default. Additional
implementations can extend the registry.

ACP agent commands are configured separately. Enabling an implementation
does not install its CLI or authenticate it. See
[Settings](../reference/settings.md#agents).

## Sessions inside a thread

Open a session from the session picker or **New Agent Session…**. Choose
its implementation and, for ACP, the configured ACP agent. Each open
session has its own tab. A personal new-thread preference can seed a
session when a thread is created; otherwise a thread can start empty.

To use a different agent, open another session. Closing or renaming a
session does not replace the thread or discard its recorded work. A
session's resume identity belongs to that session and harness.

Several agents can run in one thread. They share the same worktree;
separate streams provide separate worktrees. Writer status is per thread,
so running several writer sessions requires coordinating their edits.

## Shared observations and services

Harness integrations feed turns, tool calls, status, usage, and changes
into the shared record. Agents can read the same semantic models and use
the same audited commands through MCP. Generated integration files live
under `.oxplow/runtime/` and are rewritten as needed; configure the
project rather than editing those generated files.

Only the writer thread may change its stream's worktree. No session needs
an in-progress task to make an edit, and work tracking never gates Stop.
Efforts belong to the thread and can span sessions. See
[Agent control](agent-control.md) and [Local History](local-history.md).

## Input

The person sends terminal input and ACP prompts. Oxplow context and
advisories supply steering information without automatically typing into
those interactive surfaces. Recorded AI API computations are separate;
see [AI models](ai-models.md).
