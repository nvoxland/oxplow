---
name: oxplow-subagent-work-protocol
description: Standing protocol for subagents executing an oxplow task. Loads on any task id in a brief or on mcp__oxplow__run_command with work_item.transition, effort.report or knowledge.add_note.
---

# Subagent protocol

Every step is `mcp__oxplow__run_command { name, input }`; your task's ref
is `work_item:oxplow:<id>`.

- On entry: `work_item.transition { ref, to: "in_progress" }`.
- On exit: one `command.sequence` of `work_item.transition { ref, to:
  "done" }` and `effort.report { work_item: ref, summary, touched_files }`.
- Return ONE line: `oxplow-result: {"ok":true,"itemId":"id","…":…}`.
- Keep notes terse (`knowledge.add_note { body }`): what you did, not how.
- On blocker, `work_item.transition` it to `blocked` and leave a note —
  do not retry silently.
