# The command bus: families, config, notes

- `.oxplow/project.yaml` keys: `oxplow.config.list_keys`,
  `oxplow.config.get { key }`, `oxplow.config.set { key, value }`,
  `oxplow.config.unset { key }`. Keys that run a program or pick the model
  (`agents`, `lsp`, `testing`, `ai`, `agentModels`,
  `extensions`, `agentPromptAppend`, …) need the person's confirmation.
- Efforts `effort.*`; work items `work_item.*` (when a list is active); test
  evidence `oxplow.test.record_run`, `oxplow.collector.sync`; notes and
  comments `oxplow.knowledge.add_note`, `oxplow.knowledge.reply_comment`, ….

## Offloading a read-heavy question to an Explore subagent

Allocate its note first (`oxplow.knowledge.add_note {}` returns `note.id`)
and have it write its finding once, at the end, with
`oxplow.knowledge.update_note { note, body }`. Read it back with `oxplow_query_sql`
over `v_thread_note`.

## Handing work to another thread

`oxplow.handoff.send { to_thread, body, work_item? }` hands work to another
open thread (find it by title in `v_thread`; any stream's). Say what is
done, what is left and what its agent needs to know; refer to items by
their quoted title. Its agent gets it at the start of its next prompt, and
the person sees it at once. Nothing wakes that agent: don't wait for a
reply in this turn.

A handoff to your thread: `oxplow.handoff.resolve { handoff, outcome:
taken | declined, note? }`. Taking one moves its work item onto your
thread. The sender withdraws an open one with `oxplow.handoff.settle {
handoff, state: withdrawn }`. Read them in `v_handoff`.
