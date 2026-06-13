---
name: oxplow-subagent-work-protocol
description: Standing protocol for subagents executing an oxplow task. Loads on any task id in a brief or on mcp__oxplow__update_task / add_thread_note calls.
---

# Subagent protocol

- Mark the item `in_progress` on entry; `done` on exit.
- Return ONE line: `oxplow-result: {"ok":true,"itemId":"id","…":…}`.
- Keep notes terse: what you did, not how.
- On blocker, set `blocked` and leave a note — do not retry silently.
