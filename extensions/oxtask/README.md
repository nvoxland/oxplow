# oxtask

oxplow's own task tracker, as an extension in its own namespace
(`oxtask`). It requires `oxplow-bundled` and is off by default.

- **A store.** Tasks, their links and comments are rows of the extension's
  SQLite file (`storage/migrations/V1__tasks.sql`). Triggers give every
  insert, update and soft delete the next change number (`seq`), so a write
  from any source is seen by the collector.
- **Fills `oxplow/task_record`** as the store `oxtask`: the columns and the
  `links` / `comments` children of bundled's model, the field `priority`
  (enum) and the read-only field `author`, ids like `tsk12`.
- **Stages** are oxtask's own: `backlog`, `ready` (bucket `todo`),
  `in_progress`, `blocked`, `done`, `canceled`. Each row carries `stage`,
  `stage_title` and the bucket as `state`.
- **Collector `tasks`** (`start`, `{sqlite: own}`): `start` states the whole
  store; a later run states the tasks after its cursor (`{seq}`) as
  upserts, with their whole link and comment sets, and the deleted ones as
  deletes.
- **Commands** `oxtask.task.create | update | move | link | unlink | comment
  | delete | restore`. Each composes `oxplow.extension_store.write`.
  `update`, `move`, `link`, `unlink`, `delete` and `restore` return an
  `inverse`, so they are undoable. `create` and `comment` are not: they mint
  their id in the insert, so the script cannot name the row it makes.
- **A minted id** is in the write's `rows` (the run's children), not in the
  command's `result`. A caller that needs it reads the run's children, or
  finds the task by title.
- Assignment and rank are bundled's fields, not oxtask's.

Fixtures in `fixtures/` run on a throwaway oxplow with bundled on:
`bun run test:ext oxtask`.
