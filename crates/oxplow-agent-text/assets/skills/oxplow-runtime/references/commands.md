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
