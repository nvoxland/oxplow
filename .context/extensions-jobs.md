# Extensions: Jobs, collectors and metrics

Sources (collectors), contributing metrics, jobs and AI profiles. Part of the extensions docs; the terms and the index of docs are in [extensions.md](./extensions.md).

## What works today: Sources

- **Sources (collectors).** Declared under `collectors:` in `extension.yaml` (v1: `sources:`) and parsed
  into `Extension.sources`. A bad source is reported in `errors` without
  hiding the extension's pages. Settings → Data shows each source
  (runtime, schedule, row counts or failure, Sync Now);
  Settings → Data keeps each extension's source credentials.

## What a declaration says about itself

Every contribution kind a person sees in a list has a one-sentence
description, published as a `description` column on its own dataset and on
`v_contribution`, so a page can show any kind the same way: a collector's
`doc:` (`v_collector.description`), a resource's `description:`
(`v_resource`), a dimension's (`v_dimension`) and an implementation's
(`v_capability_provider`). Metrics and measures carry theirs
(`v_metric_spec`, `v_measure`); commands, skills, slot mounts and custom
components are read from `v_command`, `v_skill`, `v_slot` and
`v_custom_component`. `oxplow extension check` does not warn about a missing
description: there is no check for one on any of these kinds yet (a collector's
`doc:` is required, the rest are optional).

## Contributing metrics (current)

`extension.yaml` takes `measures:`, `metrics:` and `dimensions:` (the
catalog schema), checked with the same `oxplow_config::validate_*` functions
given `CatalogLayer::Extension(ns)` (`Project` and `Global` say where a block
is written elsewhere), and fact collectors (a `collectors:` entry with
`facts:`, parsed by `oxplow_config::collectors` with a `Declarer`), plus:

- Metrics must be `key:` definitions; `use:` belongs to the project's
  `.oxplow/project.yaml`.
- Entity metrics (`entity:` + `where` / `time` / `value`) work
  here too, usually over the extension's own source views. Their fragments
  are checked at seed time, so one over a view whose source hasn't synced
  yet stays out of the catalog until the next reseed.
- Dimensions, fact or entity (`entity` / `expr` / `join`), layer
  like metrics (`resolve_dimensions` takes an `extensions` layer) and slice
  an extension's entity metrics over the same view. They are stored as
  `scope = 'global'` plus the `extension` column (V89).
  - `promote` is refused unless the extension is the project's own or
    bundled: toggling an installed extension would rebuild the metric cube
    each time.
  - A disabled or removed extension's dimensions are deleted
    (`delete_extension_dimensions_not_in`, never a promoted one). Facts
    don't reference dimension rows, so nothing else is lost.
- Fact collectors run `starlark` / `jaq` only, with no `env` /
  `credentials` / `network`. `exec` is refused unless the extension is the project's own: an own
  extension's exec program needs `process.run` like any extension
  collector and runs from its deployment.
  `entry` must exist in the extension.
- Precedence is global (the person's) < each extension, and the project's
  `use:` layers thresholds
  (`oxplow_config::resolve_{metrics,measures,dimensions}` take an
  `extensions` layer; scope `global` or `extension:<name>`; there is no
  built-in or project scope). `MetricsService::fact_collectors()` is the
  enabled extensions' fact collectors. An enabled extension's
  metrics are **on** unless the project mentions the key (a `use:`
  override or disable marker, or its own definition); a `default: off`
  one is listed inactive until the project `use:`s it. A metric's
  `collector:` names the collector in its extension that computes it: a
  collector metrics name runs only while one of them is active, one no
  metric names whenever its extension is enabled. A fact collector with
  `whole_tree: true` reads the whole tree as of the snapshot on every run.
  `oxplow.*` measure, dimension, metric and collector keys are the
  `oxplow` namespace's (`oxplow_config::reserved_key`).
- `MetricsService::extension_catalog` loads the live extensions
  on each resolve; fact-collector scripts are read through
  `extensions::read_extension_file` (bundled or disk), and a collector's
  `report` path resolves against the project root (the checkout).
- Storage: the scope CHECK allows only a fixed set, and
  rebuilding `measure` / `metric_spec` would cascade-delete facts (V54),
  so the store writes `extension:<name>` as `scope = 'global'` plus the
  `extension` column (V84) and reads it back. `v_metric_spec` shows
  `scope = 'extension'` and the `extension` name.
- A generation prunes extension specs no longer declared (a disabled or
  removed extension) with `delete_extension_specs_not_in`; its measures
  stay, since deleting one would take its facts. A new generation follows when the live set changes (a deploy).

## Jobs

A **job** is what an extension declares (`job:<extension>/<id>`); a **job
run** is one execution of it (`job_run:<id>`, a row of `v_job_run`). A job
is model work an extension asks for that runs outside any
transaction and outlives a restart (`operation: ai.complete_typed`). It
streams its reply as it comes, may call tools the extension declares,
can be cancelled mid-reply, and says how it ended with an event. The
declaration is `extensions/jobs.rs`; intent, snapshot, cancel and the
end event `extension_jobs.rs`; what runs `extension_job_runner.rs`;
claiming, supervising and publishing `extension_job_dispatch.rs`,
`extension_job_scheduler.rs` and `extension_job_authority.rs`.

```yaml
jobs:
  - id: review
    description: Review the change's TODOs.
    version: 1                        # the declaration's own contract version
    operation: ai.complete_typed
    needs: [ai.compute, sql.read]     # ai.compute; sql.read adds the query_sql tool
    tools: [evidence.lookup]          # this extension's commands the model may call
    max_tool_rounds: 4                # model rounds at most, 1–16 (default 4)
    timeout: 300                      # seconds, 1–600 (default 120)
    replay: never                     # or idempotent: run again when cut off
commands:
  - { name: review.ask, summary: …, scope: jobs.write, op: enqueue }
  - { name: review.cancel, summary: …, scope: jobs.write, op: cancel }
  - { name: review.result, summary: …, scope: jobs.read, op: snapshot }
  - name: evidence.lookup             # a tool: reads, admits a job, says its input
    summary: The TODOs in one file.
    entry: commands/evidence_lookup.star
    needs: [sql.read]
    input_schema: { type: object, required: [path], properties: { path: { type: string } } }
    access: read
    job: true
```

- **A tool** is one of the extension's own commands that admits a job
  (`job: true`), only reads (`access: read`) and has an
  `input_schema`; anything else is a load error. The model calls it by
  its local name with `.` as `_` (`evidence_lookup`: dataset APIs allow no
  dots); its description is the command's `summary`, its parameters the
  schema. `query_sql` is reserved: with `sql.read` the dataset gets it, a
  read-only query over the published datasets (as MCP `oxplow_query_sql`; 200
  rows). Core's commands admit no job, so a dataset reads oxplow through
  `query_sql` and never writes: a job's writes come from an effect on
  its end.
- **Permissions.** A job runs under its extension's `permissions:`
  (extensions-permissions.md "Permissions"): the commands its tools name outside the extension's
  namespace need `commands:<pattern>`, and its model calls need `ai`;
  the loader refuses the extension otherwise. A person approved them when
  the extension was deployed, and a deployment is immutable, so a job
  runs the declaration that was deployed — a new tool arrives with a new
  deploy. Every model call also needs its AI provider's approval.
- **Enqueue** (`jobs.write/enqueue`): `{ job, intentKey, request }`,
  `request` `{ schemaVersion: 1, role, messages, tools? }` — never a
  dataset, endpoint or credential: the role's dataset is bound when it runs.
  A job that declares `tools` takes none in its request. One intent key
  is one job (asking again returns it). At most 1024 queued or running
  in a project, 256 per extension. The request stays private (it isn't
  in the audit or any dataset).
- **Running.** The scheduler (four at a time, polling each second)
  claims a queued job, resolves its authority afresh — the declaration
  and permissions as they are now, the thread's open-thread gate (a Write-scope need must be bound to an open thread: `JobExecution.needs_thread`) — and runs it
  as `Actor::Job` (`job:<id>:<attempt>:<fence>`) on a run of its own,
  linked to the run that asked. Each model round streams on the role
  (`AiCompute::stream_typed`, a recorded computation: the same request
  reads its recorded answer, replayed at once). What comes is the
  attempt's **output** (`extension_job_output`, `v_job_run_output`): text
  coalesced into a row every 100 ms (or 4 KiB), each tool call and what
  it answered, each round's usage. Each write is its own short
  transaction — none is held while the dataset writes — and the table
  refuses rows for an attempt that has closed, so a worker that lost
  its attempt adds nothing.
- **Tool rounds.** A reply with tool calls (from a job that declares
  tools) runs them, at most 16 a round: `query_sql`, or the command
  through `commands.run_in` on the job's run — the bus admits the job
  as a job, validates the input against the schema and re-checks every
  child a script composes (none of core's admits a job). A refusal, an
  invalid input, an unknown tool or a failure answers the model with
  `{ error }`, and the job goes on; a script tool answers its script's
  `result`; more than 64 KiB is refused. The calls and their answers go
  back to the model as its next round. Past `max_tool_rounds` the job
  fails (`tool rounds exhausted`). A job without tools keeps any tool
  calls in its reply as data.
- **Its result**: `{ parts, finishReason, rounds, usage: { inputTokens,
  outputTokens }, aiCalls }` — `parts` is the last round's reply; a
  failure adds `error`; a cancelled job keeps the parts it had, with
  `stopped: cancelled`. Enqueue returns `{ jobRun, state, created }`;
  `jobs.read/snapshot { job_run }` returns `{ jobRun, job, intentKey,
  version, state, attempt, cancellationRequested, text, toolCalls, result
  }`, `text` being what the current attempt has streamed so far (`jobRun`
  the run's URI, `job` the job's). The intent key is the run's subject (one
  run per key and job), so an effect on `contribution.job.completed` knows
  what the answer is about; `v_job_run.intent_key` publishes it.
- **Cancel.** `jobs.write/cancel` (the extension's own jobs) or
  `oxplow.job_run.cancel { job_run }` (any run).
  A queued job ends at once. A running one has its run cancelled: the
  dataset's connection is dropped within 100 ms
  (`oxplow_ai::client::unless_cancelled`), the attempt closes
  `cancelled` with what had come, and the call's `v_ai_call` row is
  `error: cancelled` with that partial reply. Disabling its
  extension stops it the same way.
- **Limits.** Past its `timeout` a job fails (`timed out after Ns`),
  keeping its output; the dispatcher stops a worker still running 15 s
  later. The lease is 30 s, renewed every 10 s: an attempt whose lease
  ran out (oxplow stopped) is fenced when recovery finds it. A `never`
  job then ends `interrupted`. An `idempotent` one is queued again in
  the same transaction (`extension_job_dispatch::requeue_tx`,
  `JobEvent::RetryInterrupted`), so it is never seen `interrupted`, and
  runs as its next attempt, up to `MAX_ATTEMPTS` (3) in all; the third
  cut-off is final. On the new attempt a round that had finished reads
  its recorded answer (a recorded computation is keyed by its request,
  not its attempt); the cut-off round is asked again, and its tools run
  again (they only read). Cancelling a job queued for its retry ends it
  `interrupted`, not `cancelled`: the attempt before may still have an
  effect in flight. A queued job survives a restart and runs after it;
  one that was running is fenced once its lease is out, then replayed or
  ended as above. A running job whose
  declaration changes or whose extension is disabled is cancelled with
  what it had, never completed under the new declaration.
- **A job belongs to its extension's deployment, not to a catalog
  generation.** Its owner is the extension, its package version, the
  job's contract `version` and the deployment's hash
  (`JobOwner.deployment`, `extension_job.deployment_hash`); nothing records
  the generation it was queued under, so another extension's change
  neither strands a queued job nor stops a running one. A redeploy of its
  own extension is another deployment whatever its version says (a
  private extension may have none; a bundled one carries oxplow's): a
  running job's `live` gate fails and it ends `cancelled`; a queued one —
  nothing of its attempt ran — is rebound to the new deployment of the
  same contribution and contract (`JobEvent::Rebind`, the only change the
  job row's owner may take, queued to queued) and runs there. A changed
  contract leaves it queued with no operation to run. Pinned by
  `a_queued_job_is_rebound_across_a_redeploy_of_its_extension` and
  `a_running_job_stops_across_a_same_version_redeploy`. A job holds no generation pin while it runs (it
  can run for minutes, and an activation would wait on it): authorizing
  its claim, each tool call (the bus pins per command) and its
  publication each pin briefly, and each checks through the job's `live`
  gate that its own extension is still that deployment and enabled. Its run's `ContributionRef.generation` is `None`. The dataset
  call holds none either: `AiService::bind` resolves the role under a
  pin and returns a `Bound` (provider, model, the declaring profile), the
  `*_on` calls stream on it unpinned, and a recorded computation is keyed
  by and computed on that one binding. Pinned by
  `a_queued_job_runs_after_another_extension_changes` and
  `an_activation_does_not_wait_for_a_running_job`.
- **Collector and effect runs hold no generation either.** The same
  pattern: a run's `ContributionRef.generation` is `None`
  (`run_context::unpinned_contribution`) and `run_context::live_owner` is a
  `guarded` check that its own extension is still enabled, the deployment
  the run started under (`ContributionRef.deployment`; a same-version or
  unversioned redeploy is another one) and still declaring the contribution, run at each
  host call and at publication (a collector's rows commit inside
  `read_generation` after `check_authority`; a fact collector's capture
  likewise). A change to or the revocation of the run's extension stops it
  publishing under the old declaration; another extension's change doesn't
  touch it. The state host decides a run with no generation by the version,
  enabled and declaration checks. `oxplow.collector.sync` is an `External`
  command, which the bus runs after releasing its pin (`.context/commands.md`),
  and an effect's `handle` no longer pins the whole event: its script runs
  unpinned and the commands it composes pin per command. Pinned by
  `an_activation_does_not_wait_for_a_running_collector`,
  `a_collector_does_not_publish_across_a_change_to_its_extension`,
  `a_collector_does_not_publish_after_its_extension_is_disabled` and
  `an_effects_run_holds_no_generation_and_ends_with_its_extension`.
- An extension has at most 256 queued or running jobs (a
  repeat of an intent it already has is not a new one). No database
  transaction is held while a model streams (`Database::write_transactions_open`
  is zero where a model is streaming, asserted in the tests). These are
  pinned in `extension_job_hardening_tests.rs` and
  `extension_job_crash_tests.rs`.
- **The replay decision.** An `idempotent` job cut off by a crash or a lost
  lease runs again, up to three attempts in all; a `never` job (the default)
  ends `interrupted` and is never run again on its own. Either way the
  end is announced once, in the transaction that ends it. A crash at any
  persistence boundary leaves the job `running` with its lease, and
  recovery (after the lease) decides as above: after the claim (nothing
  started), before the publish transaction (the worker's answer is lost,
  and a replay reads it back from its recorded computation), and inside it
  (the transaction rolls back whole, so the end and its event are both
  unwritten). A worker that returns after recovery fenced its attempt
  changes nothing: its result is kept once as an observation
  (`v_job_run_observation`: `attempt`, `fence`, `outcome`, `evidence`).
  Extension disabled, or the declaration changed, between the worker
  returning and the publish transaction cancels the job at once, with the
  result it had as a `stopped: cancelled` one and the same late
  observation: the worker has returned, so there is no lease to wait out.
  Two dispatchers racing for a queued job: the revision check lets one
  claim it. A repeat of an intent is the same job; the same intent with
  other input is refused as a conflict, even when the queue is full.
- **Its end** is the event `contribution.job.completed@2 { job_run, job,
  version, attempt, fence, outcome: succeeded | failed | cancelled |
  interrupted }` (`job_run` and `job` URIs; an effect keeps to one job with
  `where: { job: "job:<extension>/<id>" }`), logged once, at the real end, in the
  transaction that ended it (however it ended: completion, cancel,
  recovery; an attempt queued again logs nothing) with the anchors and
  cause the job was asked with. It carries no request or result: an
  effect reads the result with `scope("jobs.read", { job_run })` and writes
  what it says (the triage example's `publish-summary`).
- **Seeing job runs.** `v_job_run` (state, attempt, rounds, tool calls,
  tokens so far, its `uri`) and `v_job_run_output` are published datasets, so
  a page or component over them follows a run live. oxplow-bundled's **Job
  Runs** page (Activity) lists the running and the last day's, with Cancel;
  a row opens its **Job Run Output**. A job run acts as `Actor::Job`, its
  source the run's URI (`job_run:<id>`); attempt and fence are on the row.
- **What a job can't do**: drive an agent session, ask a person (a
  command that needs confirmation is refused, never proposed), write
  (its tools read), run core's commands, or call a model its extension's
  permissions and the AI provider's don't allow.

### AI profiles (`ai_profiles:`)

`extensions/ai_profiles.rs` parses `ai_profiles:` (`id` `[a-z][a-z0-9_]*`
unique in the extension, `description` ≤ 4096 bytes, `fallback` one of
the built-in roles `main | summarize | decide` (required), `features` of
`text | tool_calling | streaming | image`) into `Extension.ai_profiles`
(`AiProfileDecl`), each a contribution `<ext>/ai_profile/<id>`
(`ContributionKind::AiProfile`, `RunsOn::Declared`). It is a declaration
only: the person binds `<extension>/<id>` to a dataset, else the fallback
role's dataset serves it; `features` are what a bound dataset must support.
The extension names it by its bare id — a job request's `role`
(`extension_jobs::enqueue` qualifies it and stores `<ext>/<id>`, refusing
a name the extension doesn't declare), a collector's `role=` (qualified
with the run's extension by `CollectorOracle`). `ai_profiles::profiles_of`
feeds `AiService::set_profiles` from the catalog; resolution, features
and Settings are [ai-providers.md](./ai-providers.md) "Extension AI
profiles". `extension check` and `extension test` print a line per
profile (`oxplow_sdk::profile_lines`: bound, or falling back), and
`check` warns when the binding serving a profile forbids a feature it
declares (`profile_warnings`, a person's `restrict:`).
