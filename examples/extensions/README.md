# Example extensions

Each folder is a working extension built only on oxplow's public
extension services. Copy one to your project's `.oxplow/extensions/`,
deploy it (`oxplow.extension.deploy { name }`) to use it, and run `oxplow extension test .oxplow/extensions/<name>` to run
its examples on a throwaway oxplow. `crates/oxplow-sdk/tests/examples.rs`
runs them all, some against a real in-memory oxplow too.

| Example | What it shows |
|---|---|
| [github](github/) | An outside data source: an exec collector syncing pull requests with a credential kept in the keychain, a ref kind (`[[pr:12]]`), a page and a custom component. |
| [prompt-efforts](prompt-efforts/) | Replacing a core policy: an effort policy script that opens one effort per prompt. |
| [task-views](task-views/) | Replacing core views: the Board, the Tasks list and the work-item editor, for any work-items provider. |
| [triage](triage/) | Data, AI and workflow state together: a collector that classifies TODO comments with a model, a durable per-TODO decision in private state, a custom board with forms, and a summary job. |
| [effort-review](effort-review/) | A cancellable AI job over collected evidence: on each finished effort, a model reads its tests, diff coverage, changed functions and findings through a tool, the review streams and can be cancelled, and it is commented on the work item and exported as a model. |
| [schema-review](schema-review/) | A review policy of its own: a collector's facts about migrations, joined with oxplow's test, coverage and code-analysis evidence into the effort's verdict, in place of oxplow-bundled's. |
| [session-notes](session-notes/) | Adding to a core page: a section on the New Session page, placed after its "Start a new session" section by that section's stable id. |
| [commit-history](commit-history/) | A page with no code: typed params in the toolbar (a date range, a search, a switch) narrow the commits, and clicking one fills the next section with its files. |
| [review-badges](review-badges/) | Building on another extension: an optional import of effort-review's exported model, shown as a list and on the task page, withheld with a warning when effort-review isn't there. |

## What the trials prove

Three of these are the extension foundation's trials, each with tests
that run through public commands and the SDK only:

- **AI review over evidence** (effort-review). An effect starts a job on
  `effort.finished`; the job's tool reads the effort's evidence; the
  answer streams into `v_job_run_output`; oxplow's Jobs command cancels it and
  the partial review is kept; an effect comments it on the work item.
  Test: `the_effort_review_example_reviews_evidence_and_can_be_cancelled`.
- **A durable workflow with its own UI** (triage). A decision survives a
  restart, a board whose extension is disabled and enabled again, and an
  edited bundle that has to be redeployed; a stale compare-exchange is
  refused.
  Tests: `crates/oxplow-sdk/tests/trials.rs`; the browser spec
  `tests-e2e/specs/components/triage.spec.ts` drives the board and the
  lens form.
- **Producer and consumer** (effort-review → review-badges). The import
  resolves, reads real reviews, and degrades — lenses and the task-page
  slot withheld, the rest loaded, the reason printed — when the producer
  is disabled, missing or exports another version.
  Tests: the review-badges tests in `examples.rs`.

A fourth proves a review isn't oxplow's to decide:

- **An alternative review policy** (schema-review). Its collector's facts
  (DDL per migration file, by its own dimension) join core's evidence —
  the effort's files, its latest test run and its provenance, its diff
  coverage, the functions its change added — in its own model, and its
  lens replaces the effort's verdict. A migration that starts dropping a
  table flips it. Test:
  `the_schema_review_example_joins_its_facts_with_core_evidence`.

None of them needs a machine credential: models are reached through the
person's own AI provider by role, and the only credential (github's) is
declared by name and stored in the keychain, never in the folder.

## Limitations and authoring friction

What building the trials ran into, as of October 2026. Some are fixed;
the rest are open work.

Fixed while building them:

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
- Withheld contributions were reported unevenly: a lens fixture failed
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
  An extension now declares `ai_profiles:` (effort-review's `review`,
  triage's `sort`); a person binds `<extension>/<id>` in Settings → AI,
  else its fallback role's model is used.
- A command script couldn't read `metric_grid()` / `metric_findings()`,
  so triage rebuilt the latest TODOs from `v_fact`; a command's script
  now reads through the SQL gateway, as an effect's does, and triage
  reads `metric_findings('triage.todos')`.

Still open:

- **No job starts on an event by itself.** An effect composes the
  extension's own enqueue command (which must admit `agent`), and the
  job runs under the extension's `permissions` once it is deployed.
- **Event order matters and isn't obvious.** `effort.closed` comes before
  the effort's evidence is refreshed; a reviewer must listen for
  `effort.finished`.
- **No diff text** in any `v_*` model: a review sees per-file and
  per-function statistics, not the hunks.
- **Not all evidence can be made in a fixture**: `setup` reaches what
  oxplow's commands make (work items, efforts, test runs), not collected
  reports like diff coverage, so that still needs a Rust test.
- **`extension test` needs the folder under `.oxplow/extensions/`.**
- **Names:** an extension's command namespace is its name with `-` as `_`
  (`effort_review.review.ask`), while its contribution ids and model
  views keep or translate the dash differently
  (`oxplow:oxplow/job?job=effort-review/review`, `v_effort_review_reviews`).
- **The compare-exchange stamp isn't readable** through a public read, so
  a stale write can only be tested as `expected: null` against existing
  state.
- **Removing an extension** (`oxplow.extension.remove`) forgets its
  deployments, private state and settings, and never deletes its source.
