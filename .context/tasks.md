# Tasks

Core has no tasks. A task is a row of the shared model `oxplow/task_record`
(declared by `oxplow-bundled`), filled by a **store**: an extension
(oxtask, the shipped one; any other that `fills:` the model, a collector
pulling an outside tracker's data included). Everything that reads or acts on a
task goes through the model, the datasets bundled builds over it, or the
action points bundled declares — never through a core table, a core verb or
a core tracker. This doc is the map; `.context/models.md` holds the model
mechanics and `.context/extensions-slots.md` the points.

## Who owns what

| Piece | Owner |
|---|---|
| The model `oxplow/task_record` (stage, title, body, parent, children `links` and `comments`), the `task` resource (`oxplow/task`), the read datasets (`task` → `v_oxplow_task`, `task_link`, `task_comment`, `search_task`, `request_link`, `commit_task`, `thread_tasks`, `deviation`, …), the pages, the Work board, assignment (`assignee`) and queue order (`rank`) fields, request links, the task points, the store-agnostic `tasks` skill | `oxplow-bundled` (it names no store's command) |
| A store's own data, commands and forms | The store: oxtask keeps its own SQLite file (`.oxplow/extensions-data/oxtask.sqlite`), its commands `oxtask.task.*`, its `oxtask/new-task` form and an `edit-task` part |
| How a store's tasks are worked: the task workflow setting (`tasks.workflow`, `tasks.definitionOfDone`, `oxtask.tasks.set_workflow`, `v_oxtask_task_workflow`, Settings → Integrations), the ready queue (`v_oxtask_ready_queue`, the `next-task` page and `/next-task`), the skills `oxtask`, `track-work`, `work-tasks`, the agent context `task-state`, the `untracked-work` advisory | The store (oxtask); another store brings its own. See extensions-bundled.md |
| The recognizers (loose ids) | Core (`ModelRegistry`, from each store's `fills.id_pattern`) |

Core's `task` datasets, `oxplow.task.*` commands, the `Tasks` client and
`TasksProjection`, the task conformance suite, the tracker registry, the
`task_tracker` capability kind (and with it the external task provider and
the `oxplow-oxtask` crate) are gone. Bundled off
means no task model: pages, tools and skills that `need` the model
(`needs: [model:oxplow/task_record]`, met while the model has a live store)
are hidden.

## The model and its stores

A store declares what it fills in its manifest (`fills:`): the model, its
store id (the `provider` param of a task's URI), `id_pattern` (how its ids
look), `stages` (id, title, bucket) and its own `fields`. The stages and
fields reach pages as `v_model_stage`, `v_model_field_decl` and
`v_model_store` (id patterns). A task's URI is
`oxplow:oxplow/task?id=<id>&provider=<store>`; its **bucket** (`backlog`,
`todo`, `in_progress`, `blocked`, `done`, `canceled`) is the one thing
shared code may act on — the stage is the store's word.

A store writes the model by returning rows (a collector's `models` output,
or the read adapter); the model's change event `oxplow.task_record.changed`
(`uri`, `changed`, `before`, `after`, `cause`) is what everything that
reacts to a task hears. Details: `.context/models.md`.

## Acting on a task: points, not verbs

Bundled declares item points, and a store's command joins with
`ui.at` (and `ui.accepts: { store }`, or `about.accepts` when it is also an
item command):

- `oxplow/task.move {uri, stage}`, `task.edit {uri, title?, body?,
  priority?, parent?}`, `task.unparent`, `task.delete`, `task.unlink {uri,
  to, type}` — run by a page or viz action (`point:` / `context:`) or by the
  Work board (`invokePoint`, `custom_components[].action_points`); the
  store whose `accepts` passes the item answers, one command or an error;
  a bulk row nothing answers is skipped, not failed;
- `oxplow/task.comment`, `task.link`, `task.new` — form points, drawn as an
  `action_point` node: the store's own form;
- a point command binds its item's id with `{{uri.id}}`.

Assignment (`oxplow.tasks.assign`), queue order (`oxplow.tasks.reorder`)
and clearing Finished are bundled's own commands over the model's
`assignee` / `rank` fields (`oxplow.model.set_fields`). The workflow setting
is the store's (oxtask's `oxtask.tasks.set_workflow`), not bundled's.

## Reactions (events, not core code)

- `thread.closed` and `stream.archived` release the work of the thread or
  stream's threads; a taken handoff (`handoff.settled`) moves its task to
  the recipient; starting a task (the first move to `in_progress` by an
  agent) assigns it to the agent's thread. These are bundled effects over
  the model.
- **Request links** (`request_link`, the model `request_task`): what a
  request worked on, with the basis (`filed`, `edited`, `transition`,
  `assigned`, `prompt`) and the first evidence kept; bundled collectors on
  `oxplow.task_record.changed`, `agent.prompt.submitted` and
  `agent.turn.ended` write it.
- A core command that names a task checks it through the task resource's
  `resolve:` view (`ref_resolver::resolve_view`): `oxplow.handoff.send`'s
  `task`, link checks, the agent context. No task resource, nothing is
  known.

## Recognizers (loose ids)

Text and commands name a task by a bare id (`tsk42`) when a live store
declared the id pattern: `ModelRegistry` (`fills.id_pattern`) is the only
source. There is no active tracker; every live store's pattern applies.

## External providers

There is no task provider contract: a JSON-RPC provider cannot implement
tasks. A tracker outside oxplow (a Linear-like service) comes in
as an ordinary extension whose collector pulls its data into
`oxplow/task_record` through `fills:`; the extension declares whatever
commands it wants like any other.

## Reading tasks

`v_oxplow_task` (bundled's `task` dataset) has the columns the pages use:
`uri`, `native_id`, `provider`, `title`, `body`, `stage`, `stage_title`,
`state` (the bucket), `parent_uri`, `assignee`, `rank`, `native` (the
store's own fields as JSON), `created_at`, `updated_at`, `closed_at`.
`v_oxplow_task_link`, `v_oxplow_task_comment`, `v_oxplow_request_link`
(`request`, `task`, `basis`, `turn`, `thread_id`, `at`, `turns`,
`started_at`, `last_ended_at`) and `v_oxplow_commit_task` complete it. An
agent reads them with `oxplow_query_sql`, or through the tools the
extensions declare (bundled's `oxplow_thread_tasks`, oxtask's
`oxtask_next_task`).

## From the command line

`oxplow-dev task …` works over oxtask's own file when the project has oxtask
enabled; without it there is no task tracker to reach (the command says
so).

## Testing

A store's behavior is tested by fixtures in its extension
(`extensions/oxtask/fixtures`, `requires: [oxplow-bundled, oxtask]`); the
shared pages, parts and the board by fixtures in bundled. Core's Rust tests
have no task store; a generic mechanism that needs "an item with a title
and a command with an inverse" uses a thread.
