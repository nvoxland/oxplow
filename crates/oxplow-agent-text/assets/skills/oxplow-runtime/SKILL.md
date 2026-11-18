---
name: oxplow-runtime
description: How oxplow tracks your work on its own (turns and requests), handoffs between threads, and the command bus. Loads on the oxplow `run_command` tool with oxplow.handoff.*, and on v_request, v_agent_turn or v_handoff.
---

# Your work in oxplow

oxplow tracks your work without your help. Nothing here is required: no
edit or stop waits on a task, and there's nothing to close before you
finish.

- **Recorded work.** The files you change (with edit tools or a shell)
  and the test runs you make are recorded against your turn, and through
  the turn against its request. A task you start (moved to in
  progress in its store, for example `oxtask.task.move`) or file into
  progress becomes yours.
- **Turns.** Each turn records what opened it (`v_agent_turn.source`: a
  person, a background job's report, a subagent, another session) and how
  it ended; `v_turn_activity` says where its time went (tools, waiting on
  the person, the rest) and its compactions. A request (`v_request`) is a
  prompt and every turn it caused, with its live state; the tasks it
  worked on are `v_oxplow_request_link`.
- **A request at a glance.** `oxplow_request_summary { request: "req3" }`:
  its facts (`v_request`, `v_thread`), tasks (`v_oxplow_request_link`, `v_oxplow_task`),
  files (`v_turn_file` via `v_agent_turn.request_id`), tests and coverage
  (`v_request_observation`), metric moves (`v_request_metric_delta`) and
  commits (`v_commit_request`, `v_commit`). Read it before querying those;
  `oxplow:oxplow/request?id=req3` is its diff.
- **Session history.** `v_agent_session`, `v_agent_session_status`,
  `v_agent_turn`, `v_tool_call`, `v_subagent`. A session that lost its
  conversation comes back with `oxplow.agent_session.reopen { session }`.
- **Standing instructions.** What the person set for every session of
  your stream and of your thread is `v_stream.prompt` and `v_thread.prompt`
  (`SELECT s.prompt, t.prompt FROM v_thread t JOIN v_stream s ON s.id =
  t.stream_id`); they are already in your system prompt.
- **Waiting on the person.** When you need their answer, end your reply
  with the question: oxplow shows your thread as waiting on them. Nothing
  else is needed.
- **Handoffs.** Work another thread hands you arrives at the start of a
  prompt in a `<handoffs>` block. Take it on or decline it with
  `oxplow.handoff.resolve`; to hand work to another thread, see
  `references/commands.md`. A handoff may be `about` any record (a task, a
  page), by its URI. Nothing wakes the other agent.
- **Context basket.** A `<context-basket>` block is what the person
  collected for you; read each `[oxplow uri …]` with `oxplow_read`. What
  waits: `v_context_item`, `v_context_basket`.
- **Hints.** oxplow may add short advisory guidance to your next prompt or
  tool result.

## Writing about work

Refer to a task by its quoted title, never by id or "#N": the person
can't map ids to what they see. Write entities in task bodies, summaries
and knowledge pages as `[[…]]` wikilinks (syntax: the knowledge skill).
Inline code is for identifiers and snippets.

# Commands (the one write path)

State changes are **commands**. `list_commands` lists what you may run (ids
and summaries); `list_commands { id }` (or `{ prefix: "oxplow.handoff." }`)
shows a command's input schema and an example input: read it before a
command's first run and copy the example's shape. `run_command { id, input }`
runs one. Every run is validated, policy-checked and audited; undoable runs
return an `inverse`.

Invalid input names the failing field, what it accepts and the example
input; a denial says why. A run that needs the person's confirmation
returns `{ kind: "proposed", proposal, message }` and waits in their
Alerts. Tell them what you proposed and why; don't run it again or look
for another way to make the change.

`references/commands.md`: project config keys (`oxplow.config.*`, and which
need confirmation), the families of commands, offloading a read-heavy
question to an Explore subagent through a note, and handing work to
another thread.
