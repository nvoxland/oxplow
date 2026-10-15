# Efforts: correcting, generated files, reading

## Correcting an effort

- `oxplow.effort.update { effort, title }`: rename it.
- `oxplow.effort.link { effort | thread, work_item }`: link it to a work
  item, or unlink it (`null`).
- `oxplow.effort.close { effort | thread }` / `oxplow.effort.open { title?,
  work_item? }`: split work oxplow grouped together.
- `oxplow.effort.report { summary?, impacts? }`: other words than your last
  answer, or outcomes beyond edits. Each impact is `{ kind, id, action? }`,
  where `kind` is `wiki`, `work_item` (its ref, or its id as the work list
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
`v_effort_file`.
