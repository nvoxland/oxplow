# Efforts: correcting, generated files, reading

## Correcting an effort

- `oxplow.effort.update { effort, title }`: rename it.
- `oxplow.effort.link { effort | thread, work_item }`: link it to a work
  item, or unlink it (`null`).
- `oxplow.effort.close { effort | thread }` / `oxplow.effort.open { title?,
  work_item? }`: split work oxplow grouped together.
- `oxplow.effort.report { summary?, impacts? }`: other words than your last
  answer, or outcomes beyond edits. Each impact is `{ kind, id, action? }`,
  where `kind` is `wiki`, `work_item` (its URI, or its id as the work list
  writes it), `git_commit`, `file` or `directory`. The result's
  `link_warnings` flags `[[…]]` links that don't resolve.

## Generated files

If build output keeps showing among your effort's files, it's probably
generated. Tells: it says so in a header; it lives under `generated/`,
`dist/`, `build/` or `target/`; it's a lockfile or codegen artifact; it
appears in effort after effort. Tell the person and offer to add it to
`generated.exclude` in `.oxplow/project.yaml`. Don't add it silently: that
file is the project's shared config.

## Reading work

Efforts are `v_effort` (`id`, `work_item`, `thread_id`, `title`,
`started_at`, `ended_at`, `closed_by`, `summary`), their files
`v_effort_file`. For one effort, `oxplow_effort_files { effort }` lists its
files and `oxplow_effort_summary { effort }` gives its title, item, files,
runs, claims and decisions in one call.

## Sessions and what they did

- `oxplow_thread_sessions { thread? }` — a thread's agent sessions (every open
  thread's when none is named): name, harness, status, how its conversation was
  bound, last activity, the efforts it worked in, and its subagents beneath.
- `oxplow_session_transcript { session, turn?, page? }` — a session's turns
  oldest first, 50 a page, with the thread's notes where they fall and a
  subagent's hand-back marked; each row is a turn URI. A subagent's last
  message is `oxplow_read { uri: "oxplow:oxplow/subagent?agent=<id>&session=ses3#part=result" }`.

In SQL the same history is `v_agent_session` (the slots opened on a
thread), `v_agent_session_status` (each open session's last status),
`v_agent_turn` (each prompt and answer), `v_tool_call` and `v_subagent`.
An effort's evidence is `v_effort_observation`, its claims `v_claim`, its decisions
`v_decision`, and its work item a `v_work_item` row.
