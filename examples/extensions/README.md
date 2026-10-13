# Example extensions

Each folder is a working extension built only on oxplow's public
extension services. Copy one to your project's `oxplow/extensions/` to
use it, and run `oxplow extension test oxplow/extensions/<name>` to run
its examples on a throwaway oxplow. `crates/oxplow-sdk/tests/examples.rs`
runs them all, some against a real in-memory oxplow too.

| Example | Sharing | What it shows |
|---|---|---|
| [github](github/) | shared | An outside data source: an exec collector syncing pull requests with a credential kept in the keychain, a ref kind (`[[pr:12]]`), a page and a custom component. |
| [prompt-efforts](prompt-efforts/) | shared | Replacing a core policy: an effort policy script that opens one effort per prompt. |
| [task-views](task-views/) | private | Replacing core views: the Board, the Tasks list and the work-item editor, for any work-items provider. |
| [triage](triage/) | shared | Data, AI and workflow state together: a collector that classifies TODO comments with a model, a durable per-TODO decision in private state, a custom board with forms, and a summary job. |
| [effort-review](effort-review/) | shared | A cancellable AI job over collected evidence: on each finished effort, a model reads its tests, diff coverage, changed functions and findings through a tool, the review streams and can be cancelled, and it is commented on the work item and exported as a model. |
| [review-badges](review-badges/) | private | Building on another extension: an optional import of effort-review's exported model, shown as a list and on the task page, withheld with a warning when effort-review isn't there. |

## What the trials prove

Three of these are the extension foundation's trials, each with tests
that run through public commands and the SDK only:

- **AI review over evidence** (effort-review). An effect starts a job on
  `effort.finished`; the job's tool reads the effort's evidence; the
  answer streams into `v_job_output`; oxplow's Jobs command cancels it and
  the partial review is kept; an effect comments it on the work item.
  Test: `the_effort_review_example_reviews_evidence_and_can_be_cancelled`.
- **A durable workflow with its own UI** (triage). A decision survives a
  restart, a revoked and re-approved board, and an edited bundle that
  asks for approval again; a stale compare-exchange is refused.
  Tests: `crates/oxplow-sdk/tests/trials.rs`; the browser spec
  `tests-e2e/specs/components/triage.spec.ts` drives the board and the
  lens form.
- **Producer and consumer** (effort-review → review-badges). The import
  resolves, reads real reviews, and degrades — lenses and the task-page
  slot withheld, the rest loaded, the reason printed — when the producer
  is disabled, missing or exports another version.
  Tests: the review-badges tests in `examples.rs`.

None of them needs a machine credential: models are reached through the
person's own AI provider by role, and the only credential (github's) is
declared by name and stored in the keychain, never in the folder.

## Limitations and authoring friction

What building the trials ran into, as of October 2026. Some are fixed;
the rest are open work.

Fixed while building them:

- Nothing linked a job to what it was about: a job's intent key is now
  its subject, read back as the snapshot's `intentKey` and
  `v_job.intent_key`.
- A read command's fixture could only check what it composed; it can
  now check `expect: { result }`.
- An author couldn't see how an import resolved; `extension check` and
  `extension test` now print one line per import.
- A local variable named like a service builtin (`files = …`) counted as
  using it, and a command script was refused for it; a name the script
  binds itself (parameter, assignment, loop or comprehension variable,
  def) is no longer a use.
- A fixture runs only when its name is in `intent.examples`, and an
  unlisted one failed with a bare "not found"; `extension check` now
  warns for an unlisted fixture, and `extension test` for an example with
  no fixture, each naming the file and the fix.

Still open:

- **No job starts on an event by itself.** An effect composes the
  extension's own enqueue command (which must admit `agent`), and the
  job still needs its own approval; until then it waits `queued`.
- **Event order matters and isn't obvious.** `effort.closed` comes before
  the effort's evidence is refreshed; a reviewer must listen for
  `effort.finished`.
- **Three fixed AI roles.** A job asks for `main`, `summarize` or
  `decide`; an extension can't declare its own profile yet.
- **No diff text** in any `v_*` model: a review sees per-file and
  per-function statistics, not the hunks.
- **Job fixtures run on an empty throwaway**: a job's tool finds no
  evidence there, so a real-data run needs a Rust test. A fixture
  model's replies are a queue, not matched to the round.
- **`extension test` needs the folder under `oxplow/extensions/`.**
- **Names:** an extension's command namespace is its name with `-` as `_`
  (`effort_review.review.ask`), while its contribution ids and model
  views keep or translate the dash differently
  (`effort-review/job/review`, `v_effort_review_reviews`).
- **Withheld contributions are reported unevenly.** A consumer's lens
  fixture fails with "not found" rather than "withheld: import
  unavailable"; a dropped slot gets no warning of its own; the import's
  warning and each withheld lens's are worded differently for one cause.
- **A command script can't read `metric_grid()` / `metric_findings()`**:
  it runs inside the bus transaction, so triage reads `v_fact` instead.
- **The compare-exchange stamp isn't readable** through a public read, so
  a stale write can only be tested as `expected: null` against existing
  state.
- **No credential lint** for a shared extension, no reload or uninstall
  command, and uninstall keeps an extension's private state with nothing
  reading it.
