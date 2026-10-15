---
name: oxplow-runtime
description: Oxplow efforts, all optional — how oxplow tracks your work on its own, when reporting is worth it, the effort.* commands, decisions, stating and reading claims, and the command bus. Loads on the oxplow `run_command` tool with effort.* or oxplow.knowledge.add_note, and on v_effort.
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
  with the question. oxplow shows your thread as waiting on them. This is
  the one way to ask; nothing else is needed.
- **Hints.** oxplow may add short guidance to your next prompt or tool
  result (work grown large with nothing committed, metric movement). Each
  is advisory.

## Efforts

`references/efforts.md`: correcting one (`oxplow.effort.update`, `link`,
`close`, `open`, `report` with its impacts), generated files that keep
showing among an effort's files, and reading efforts (`v_effort`,
`v_effort_file`). A work list, when the project has one, has its own skill.

## Decisions and claims

The person reviewing your work checks these first.

- `oxplow.effort.record_decision { question, choice, alternatives?,
  confidence?, why? }` when you resolve a real fork without asking (where
  something lives, which approach, what you left out). Record it when you
  make it.
- Claims ("tests pass", "no behavior change"): state the ones you can back
  in `oxplow.effort.report`'s `claims` (`{ statement, kind, evidence_ref? }`,
  citing the run or test). oxplow reads the rest in your answers when the
  effort finishes, and checks each against what it observed (a green test
  run). Say them plainly either way.

## Writing about work

Refer to a work item by its quoted title, never by id or "#N": the person
can't map ids to what they see. Write entities in task bodies, summaries
and wiki pages as `[[…]]` wikilinks (syntax: the oxplow-wiki-capture skill).
Inline code is for identifiers and snippets.

# Commands (the one write path)

State changes are **commands**. `list_commands` lists what you may run (ids
and summaries); `list_commands { id }` (or `{ prefix: "oxplow.effort." }`)
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
need confirmation), the families of commands, and offloading a read-heavy
question to an Explore subagent through a note.
