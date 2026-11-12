# Example extensions

Each folder is a working extension built only on oxplow's public
extension services. Copy one to your project's `.oxplow/extensions/`,
deploy it (`oxplow.extension.deploy { name }`) to use it, and run `oxplow extension test .oxplow/extensions/<name>` to run
its examples on a throwaway oxplow. `every_example_scaffolds_checks_and_tests_clean`
(`crates/oxplow-sdk/tests/just_works.rs`) copies each one with `oxplow extension new --from <example>`
and runs its fixtures; `tests/examples.rs` adds only what the SDK reports about some of them.

| Example | What it shows |
|---|---|
| [github](github/) | An outside data source: an exec collector syncing pull requests with a credential kept in the keychain, a resource kind (`[[pr:12]]`), a page and a custom component. |
| [task-views](task_views/) | Replacing core views: the Board, the Tasks list and the task editor, for any task store. |
| [triage](triage/) | Data, AI and workflow state together: a collector that classifies TODO comments with a model, a durable per-TODO decision in private state, a custom board with forms, and a summary job. |
| [schema-review](schema_review/) | A collector's facts about schema migrations (DDL per file, by its own dimension) and a page on a request's diff that lists the migration files it changed. |
| [session-history](session_history/) | Adding a section to a core page: earlier prompts from the current thread, mounted on New Session after its start form. |
| [commit-history](commit_history/) | A page with no code: typed params in the toolbar (a date range, a search, a switch) narrow the commits, and clicking one fills the next section with its files. |

## What the trials prove

One of these is the extension foundation's trial, each with tests
that run through public commands and the SDK only:

- **A durable workflow with its own UI** (triage). A decision survives a
  restart, a board whose extension is disabled and enabled again, and an
  edited bundle that has to be redeployed; a stale compare-exchange is
  refused.
  Tests: `crates/oxplow-sdk/tests/trials.rs`; the browser spec
  `tests-e2e/specs/components/triage.spec.ts` checks that a decision survives
  a reload; the board and the page form are the example's fixtures'.

Another shows facts of your own beside core's evidence:

- **A collector of your own facts** (schema-review). Its collector counts
  DDL per migration file, by its own dimension, and a page mounted on a
  request's diff joins those facts to the request's files: the migration
  files it changed and what each creates, alters and drops.

None of them needs a machine credential: models are reached through the
person's own AI provider by role, and the only credential (github's) is
declared by name and stored in the keychain, never in the folder.

## Limitations and authoring friction

What building the trials ran into, as of October 2026. Some are fixed;
the rest are open work.

Fixed while building them (some of the trials have since been removed):

- Nothing linked a job to what it was about: a job's intent key is now
  its subject, read back as the snapshot's `intentKey` and
  `v_job_run.intent_key`.
- A read command's fixture could only check what it composed; it can
  now check `expect: { result }`.
- An author couldn't see how an import resolved; `extension check` and
  `extension test` now print one line per import.
- Job fixtures ran on an empty throwaway, so a job's tool found no
  evidence; a job fixture's `setup` now runs oxplow's own commands first,
  and a fixture model's reply can be matched to the request (`when`).
- Withheld contributions were reported unevenly: a page fixture failed
  with "not found", a dropped slot got no warning, and one cause got two
  differently worded warnings. Now each unavailable import is one warning
  naming everything it withheld, and `extension test` skips a withheld
  target's fixture, saying why.
- A local variable named like a service builtin (`files = …`) counted as
  using it, and a command script was refused for it; a name the script
  binds itself (parameter, assignment, loop or comprehension variable,
  def) is no longer a use.
- A fixture runs only when its name is in `intent.examples`, and an
  unlisted one failed with a bare "not found"; `extension check` now
  warns for an unlisted fixture, and `extension test` for an example with
  no fixture, each naming the file and the fix.
- Three fixed AI roles: a job asked for `main`, `summarize` or `decide`.
  An extension now declares `ai_profiles:` (triage's `sort`); a person binds `<extension>/<id>` in Settings → AI,
  else its fallback role's model is used.
- A command script couldn't read `metric_grid()` / `metric_findings()`,
  so triage rebuilt the latest TODOs from `v_fact`; a command's script
  now reads through the SQL gateway, as an effect's does, and triage
  reads `metric_findings('triage.todos')`.

Still open:

- **No job starts on an event by itself.** An effect composes the
  extension's own enqueue command (which must admit `agent`), and the
  job runs under the extension's `permissions` once it is deployed.
- **No diff text** in any `v_*` dataset: a review sees per-file and
  per-function statistics, not the hunks.
- **Not all evidence can be made in a fixture**: `setup` reaches what
  oxplow's commands make (tasks, requests, test runs), not collected
  reports like diff coverage, so that still needs a Rust test.
- **`extension test` needs the folder under `.oxplow/extensions/`.**
- **The compare-exchange stamp isn't readable** through a public read, so
  a stale write can only be tested as `expected: null` against existing
  state.
- **Removing an extension** (`oxplow.extension.remove`) forgets its
  deployments, private state and settings, and never deletes its source.
