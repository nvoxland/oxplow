# Jobs and AI profiles

## Jobs

When the answer needs a model to read, think and maybe look things up —
a review, a summary over many items — declare a **job** rather than
calling a model from a collector:

```yaml
jobs:
  - { id: review, description: Review the TODOs., version: 1, operation: ai.complete_typed,
      needs: [ai.compute, sql.read], tools: [todos.in_file], max_tool_rounds: 4, timeout: 120 }
commands:
  - { name: review.ask, summary: Ask for a review., scope: jobs.write, op: enqueue, invokers: { human: true, agent: true, lens: true } }
  - { name: review.cancel, summary: Stop it., scope: jobs.write, op: cancel, invokers: { human: true, agent: true, lens: true } }
  - { name: review.result, summary: Read it., scope: jobs.read, op: snapshot, invokers: { human: true, agent: true, lens: true } }
  - name: todos.in_file              # a tool: reads, admits a job, says its input
    summary: The TODO comments in one file.
    entry: commands/todos_in_file.star
    needs: [sql.read]
    input_schema: { type: object, required: [path], properties: { path: { type: string } } }
    access: read
    invokers: { human: false, agent: false, lens: false, job: true }
effects:
  - { id: publish, on: [contribution.job.completed], entry: effects/publish.star, needs: [jobs.read] }
```

- `review.ask` takes `{ job, intentKey, request: { schemaVersion: 1,
  role, messages } }` — no model: the role's is used. The `intentKey` is
  the job's **subject**, one job per key: put what it's about in it
  (`review:effort:eff12`), and read it back as the snapshot's `intentKey`
  (or `v_job_run.intent_key`) in the effect that publishes the answer.
- To start one on an event, an effect composes your own `ask` command
  (give it `invokers.agent: true`: an effect acts as an agent). Keep the
  prompt in one script command that composes `ask`, so the effect and a
  lens button share it.
- A tool only reads; the model calls it as `todos_in_file`, and with
  `sql.read` it also has `query_sql` over the `v_*` models. A job never
  writes: the effect on its end does (`scope("jobs.read", { job_run })` gives
  it the result, `{ parts, … }`; its event's `outcome` is `succeeded`,
  `failed`, `cancelled` or `interrupted`). Declare `replay: idempotent`
  when running it again is harmless: an attempt a restart cuts off is
  then retried, up to three attempts, and the effect hears only the
  final end.
- Show it running with a lens over `v_job_run_output` (`WHERE job_id = (SELECT id FROM v_job_run WHERE ref = :job_run)`):
  it updates as the model writes. oxplow's own Job Runs lens lists every run
  with Cancel.
- `examples/extensions/triage/` in the oxplow repo has one, with its
  fixtures (a tool round, a cancel); `examples/extensions/effort-review/`
  starts one per finished effort, its tool reading the effort's evidence,
  and comments the answer on the work item.
- Your command namespace is your name with `-` as `_`: extension
  `effort-review` has commands `effort_review.review.ask`, while its
  its contributions' refs keep the dash (`job:effort-review/review`).

## AI profiles

When a job or script needs "a model for X" that a person may want to
choose, declare `ai_profiles: [{ id, description, fallback: main|summarize|decide,
features? }]` rather than hard-coding a role, and name it by its id: a job
request's `role: <id>`, a collector's `ai_classify(..., role = "<id>")`.
`oxplow extension check` shows what serves it. Until a person binds
`<extension>/<id>` in Settings → AI, the fallback role's model is used.
