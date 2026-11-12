# note-tracker

A task store over markdown notes in `notes/` of the project.

- Fills the shared task model `oxplow/task_record` (bundled's) for the store `notes`.
- `fills:` declares the store's own workflow: `jotted` (todo), `working`
  (in progress), `settled` (done), and how its ids look (`note-…`).
- One collector, triggers `start` and `{files: "notes/*.md"}`. A note is
  `# Title`, an optional `stage: working` line and the body. Start states a
  snapshot; a file change upserts, a removed file deletes.
- The collector only reads (`files.list`, `files.read`) and returns the
  `models` output. The host applies it.
- `note.new` joins the `oxplow/task.new` point and writes a note by
  composing `oxplow.extension_files.write`.

It replaces the old script `task_tracker` implementation: a task store is a
collector that fills the model, not a set of verbs.

`oxplow extension test` runs its fixtures.
