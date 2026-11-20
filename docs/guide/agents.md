# Agents

A thread is a line of work, not the home of one fixed agent. It can have
no agent session or several. Sessions have their own agent implementation,
process, resume identity, activity, and status; they share the thread's
context, tabs, and writer role.

Oxplow ships harness integrations for Claude Code, Codex, and OpenCode,
with implementations declared by its foundation extension. Each runs in a
PTY in the stream's worktree.

## Choose available harnesses

Open **Settings → Agents** to enable or disable each harness and edit its
launch preferences. Each preference can apply locally, to this project, or to
all your projects. Additional implementations can extend the registry.

Choices are alphabetical. There is no agent priority or implicit default,
and you can disable every harness.

Enabling an implementation does not install its CLI or authenticate it.
See [Settings](../reference/settings.md#agents).

## Sessions inside a thread

Open a session from the session picker or **New Agent Session…**. Choose
its implementation. Each open
session has its own tab. A new thread opens the picker, which remembers your
last successful choice without opening a session automatically.

To use a different agent, open another session. Closing or renaming a
session does not replace the thread or discard its recorded work. A
session's resume identity belongs to that session and harness.

Several agents can run in one thread. They share the same worktree;
separate streams provide separate worktrees. Writer status is per thread,
so running several writer sessions requires coordinating their edits.

## Shared observations and services

Harness integrations feed turns, tool calls, status, usage, and changes
into the shared record. Agents can read the same semantic datasets and use
the same audited commands through MCP. Generated integration files live
under `.oxplow/runtime/` and are rewritten as needed; configure the
project rather than editing those generated files.

Only the writer thread may change its stream's worktree. No session needs
an in-progress task to make an edit, and work tracking never gates Stop.
Requests belong to the thread and can span sessions. See
[Agent control](agent-control.md) and [Local History](local-history.md).

## Collect context for the agents

Add to Context puts things in a context basket: the thread's own (**This
Thread**) or a named basket you keep for a topic.

- A page's ☰ menu → **Add to Context** adds the page. A right-click
  anywhere on a page with nothing selected opens the same menu.
- A selection in an editor, a diff, a terminal or on any page, and a page
  row's right-click menu, add that text (or row). Each add opens a prompt
  for an optional note that goes with the item, and the basket it goes in.
  In the editor with nothing selected, **Add File to Context** adds the
  file.

Every entry carries the @ icon, the same as the **Context** icon in the
bottom bar, which shows how many items the thread's next prompt carries.
Open it to see the baskets, take an item out with its ×, choose which
basket the next prompt sends (**Send Next**), or **Clear** This Thread. The arrow in the panel's
header opens the Context page.

The next prompt to any agent in the thread carries one basket: This Thread,
or the one you chose with Send Next. Then it is empty, and the thread sends
This Thread again. An item you removed is
not sent. See [Collecting context](context.md).

## Input

The person sends terminal input and chat prompts. Oxplow context and
advisories supply steering information without automatically typing into
those interactive surfaces. Recorded AI API computations are separate;
see [AI models](ai-models.md).
