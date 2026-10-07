---
name: oxplow-runtime
description: Oxplow efforts, all optional — how oxplow tracks your work on its own, when reporting is worth it, the effort.* commands, decisions and claims, and the command bus. Loads on mcp__oxplow__run_command with effort.* or oxplow.knowledge.add_note, and on v_effort.
---

# Your work in oxplow

oxplow tracks your work without your help. Nothing here is required: no
edit or stop waits on a task, and there's nothing to close before you
finish.

- **Efforts.** An effort is a span of your thread's work. oxplow opens
  one when a turn changes files, closes it when a commit lands its work,
  and links it to an item you or the person start. The files you change
  (with edit tools or a shell) and the test runs you make are recorded
  against it. Its summary is your last answer.
- **Waiting on the person.** When you need their answer, end your reply
  with the question. oxplow shows your thread as waiting on them.
- **Hints.** oxplow may add short guidance to your next prompt or tool
  result (coverage below target, work grown large with nothing
  committed). Each is advisory.

## Correcting an effort

- `oxplow.effort.update { effort, title }`: rename it.
- `oxplow.effort.link { effort | thread, work_item }`: link it to a work item,
  or unlink it (`null`).
- `oxplow.effort.close { effort | thread }` / `oxplow.effort.open { title?, work_item? }`:
  split work oxplow grouped together.
- `oxplow.effort.report { summary?, impacts? }`: other words than your last
  answer, or outcomes beyond edits. Each impact is
  `{ kind, id, action? }`, where `kind` is `wiki`, `work_item` (its
  ref, or its id as the work list writes it), `git_commit`, `file` or
  `directory`. The result's `link_warnings`
  flags `[[…]]` links that don't resolve.

## Generated files

If build output keeps showing among your effort's files, it's probably
generated. Tells: it says so in a header; it lives under `generated/`,
`dist/`, `build/` or `target/`; it's a lockfile or codegen artifact; it
appears in effort after effort. Tell the person and offer to add it to
`generated.exclude` in `.oxplow/project.yaml`. Don't add it silently:
that file is the project's shared config.

## Reading work

Efforts are `v_effort` (`id`, `work_item`, `thread_id`, `title`,
`started_at`, `ended_at`, `closed_by`, `summary`), their files
`v_effort_file`. A work list, when the project has one, has its own skill.

## Decisions and claims

The person reviewing your work checks these first, so record them as
data:

- `oxplow.effort.record_decision { question, choice, alternatives?, confidence?, why? }`
  when you resolve a real fork without asking (where something lives,
  which approach, what you left out). Record it when you make it.
- `oxplow.effort.record_claim { statement, kind, evidence_ref? }` for "tests
  pass", "no behavior change" and the like, citing `evidence_ref`
  (`run:<id>`, a test name) when you have it. Unbacked claims show as
  unverified.

## Writing about work

Refer to a work item by its quoted title, never by id or "#N": the
person can't map ids to what they see. In task bodies, summaries and wiki pages,
write entities as `[[…]]` wikilinks: `[[src/foo.ts]]`,
`[[dir:src/components]]`, `[[some-slug]]` (wiki), `[[abc1234]]`
(commit), `[[tsk42]]` (never `#42`). Inline code is for identifiers and
snippets.

To offload a read-heavy question to an Explore subagent, allocate its
note first (`oxplow.knowledge.add_note {}` returns `note.id`) and have it write
its finding once, at the end, with `oxplow.knowledge.update_note { note, body }`.
Read it back with `list_thread_notes`.

# Commands (the one write path)

State changes are **commands**: `list_commands` shows what you may run,
with input schemas, and `run_command { name, input }` runs one. Every
run is validated, policy-checked and audited; undoable runs return an
`inverse`.

- `.oxplow/project.yaml` keys: `oxplow.config.list_keys`, `oxplow.config.get { key }`,
  `oxplow.config.set { key, value }`, `oxplow.config.unset { key }`. Keys that run a
  program or pick the model (`agents`, `lsp`, `testing`, `collectors`,
  `ai`, `acpAgents`, `agentModels`, `extensions`, `agentPromptAppend`, …)
  need the person's confirmation.
- Efforts `effort.*`; work items `work_item.*` (when a list is active); test evidence
  `oxplow.test.record_run`, `oxplow.collector.sync`; notes and comments
  `oxplow.knowledge.add_note`, `oxplow.knowledge.reply_comment`, ….

Invalid input names the failing field; a denial says why. A run that
needs the person's confirmation returns `{ kind: "proposed", proposal,
message }` and waits in their Alerts. Tell them what you proposed and
why; don't run it again or look for another way to make the change.
