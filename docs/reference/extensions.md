# Extensions

An extension is a folder under `oxplow/extensions/<name>/` with an
`extension.yaml`. Everything it adds is declared there as data:
lenses, models, collectors, commands, effects, providers and the rest.
Each of those is a *contribution*.

The goal is personal tools that agents can help you build, with useful
customizations shareable with a team and eventually more broadly. Core
provides the shared data and action contracts; an extension adds its own
analysis, presentation, integration, or workflow.

Repository code is untrusted regardless of who authored or shared it.
Access declarations, version-bound approval, command gates, and component
isolation provide guardrails. Current in-process script timeouts stop
waiting for a result but may leave computation running: they are not yet
CPU/memory containment. External-program isolation also depends on the
platform; see [collector limits](../guide/lenses.md#collectors-bringing-in-outside-data).

```sh
oxplow extension new extension my-review   # scaffold one that checks clean
oxplow extension check my-review           # every problem, as file:line
oxplow extension test my-review            # run its examples on a throwaway oxplow
```

`new` takes a kind instead of `extension` to start from that kind's
starter: `lens`, `collector`, `command`, `effect`, `provider`,
`component`, `policy`, `advisory`, `skill`, `page` or `panel`. A provider takes `--capability` for what it
implements: `work_items` (the default), `effort_policy`,
`agent_harness` (a harness that tells its subagents apart also declares
the `subagents` feature; see the provider protocol), `snapshots` (its
example marks a one-file tree under `fixtures/tree`; the `contents`
feature adds `read_at`) or `knowledge` (its example writes a page). Each
scaffold checks clean and passes `test` as written; an `advisory` or
`skill` has no example `test` can run (it runs lenses, collectors,
commands, effects and providers), so `test` notes that and nothing more.

## Editor support

The scaffolded files start with a schema line, so an editor running
yaml-language-server (VS Code's YAML extension, for one) completes
keys and flags mistakes as you type:

```yaml
# yaml-language-server: $schema=https://nvoxland.github.io/oxplow/reference/schemas/extension.schema.json
```

Lens files use [`lens.schema.json`](schemas/lens.schema.json) the same
way. Add the line to files you wrote by hand. The loader is still the
authority: `oxplow extension check` catches what a schema can't, like a
lens naming a column its query doesn't return.

## `extension.yaml`

```yaml
manifest: 2
name: my-review          # must equal the folder name
description: Lenses for reviewing agent work
sharing: private         # private (default) or shared
intent:
  purpose: What question it answers, or what job it does
  examples:
    - { name: blocked, input: { lens: blocked } }
```

- `manifest: 2` and `intent` are required. Unknown keys are errors.
- `intent.examples` are the inputs `oxplow extension test` runs, each
  with a fixture under `fixtures/<example>.yaml` saying what to expect.
- `sharing: shared` is for an extension committed for a team or
  installed from git. A shared extension needs `engine: ">=0.7"` (the
  oxplow it targets) and its own `version: 1.2.0` (`MAJOR.MINOR.PATCH`),
  A private one may declare a version too.
- The folder name is lowercase letters, digits and single dashes,
  starting with a letter.
- A metric, measure, dimension or fact collector's key is shared by every
  extension in the project: when two declare the same one, the first
  (alphabetically) keeps it and the other gets a load error naming
  both.

### Public imports and exports

Export a contribution explicitly, using its canonical ID and a positive
integer contract version:

```yaml
exports:
  - contribution: review-data/model/items
    version: 1
    visibility: stable
```

The consumer chooses a local binding:

```yaml
imports:
  - id: review_items
    contribution: review-data/model/items
    version: 1
    optional: true
import_guards:
  - contribution: review-ui/collector/summarize
    imports: [review_items]
```

A model or lens reads that binding with
`SELECT * FROM import_ref('review_items')`. SQL references infer their own
guard. An unavailable optional binding withholds the referring contribution;
independent contributions remain usable. Explicit guards can name models,
lenses, collectors, commands, effects, advisories, skills or providers. Their
exports and dependent mounts are withheld too. Each unavailable optional
import is one warning naming why and everything it withheld:
`` optional import `items` for `maker/model/items` v1 is unavailable: maker
is disabled; withheld: `review-ui/lens/items`, `review-ui/slot/work_item.detail.body-0` ``.
Missing required imports and
dependency cycles block the extension with source-located errors.

Versions must match exactly. A model or event export must use its native
contract version. Package `version` and host `engine` are separate.
Visibility defaults to `private`, which other extensions cannot import.
`experimental` exports accept private consumers; `stable` exports accept either
sharing mode. Shared packages cannot expose experimental exports. Import targets resolve by canonical
ID; published reference metadata is returned only after resolution.

Imports grant no SQL, network, command or other authority. Existing foreign-event
subscriptions retain their behavior. There is no general extension service-call
surface in this contract. Authoring checks and change review resolve the candidate
alongside current project dependencies. A required consumer broken by an update
is reported before installation replaces files. Review can inspect a disabled
package without enabling it; update commands still honor configuration disabling.
Both impact sides use current dependencies. Coherent runtime generation
activation is still in progress.

### Configuration declarations

The loader validates a versioned settings declaration. Storage of overrides and
extension host access are still under development.

```yaml
configuration:
  version: 1
  schema:
    type: object
    properties:
      label: {type: string}
      pageSize: {type: integer, minimum: 1, maximum: 100}
    required: [label]
    additionalProperties: false
  defaults:
    pageSize: 20
```

`version` is a positive settings schema version. Defaults can omit required
object fields; resolved settings must satisfy the whole schema. Arrays are
complete replacement values, so their objects must include required fields.
Branch constraints such as `anyOf` still apply to defaults as written. Schema
`default` annotations do not supply values.

Schema compilation is offline. Diagnostics contain locations and error codes,
with values omitted. Use non-secret settings only: credentials belong to the
host's secret service. Settings declarations grant no permissions.

Host secret references are identity-only objects:
`{schemaVersion: 1, id: "<UUIDv7 returned by the host>"}`. A person supplies the
value through the host's settings API; reads return only whether it is set.
References can be stored as non-secret settings, and copying them to another
extension or project does not copy access.

The first supported consumer is a remote MCP HTTP provider. Declare
`auth: OXPLOW_MCP_BEARER` in `adapter.mcp`, and a credential with
`name: OXPLOW_MCP_BEARER` and `secretRef: <the reference object>`. A person must
approve that exact provider declaration. The shipped HTTP transport sends the
bearer; extension scripts cannot read its value. References are refused for
package programs, stdio servers and OAuth credentials. A human settings form
for managing references is not implemented yet.

### What it can declare

<!-- kinds:begin (generated from oxplow_domain::contribution::KINDS; bless with OXPLOW_BLESS=1) -->
| Key | What it adds | More |
|---|---|---|
| `models` | SQL views over oxplow's models, published as `v_<ext>_<name>` |  |
| `measures` | what a metric's facts measure | [Metrics](../guide/metrics.md) |
| `metrics` | a number tracked over a measure | [Metrics](../guide/metrics.md) |
| `dimensions` | a way to slice metrics | [Metrics](../guide/metrics.md) |
| `collectors` | outside data brought in as entities, or facts for metrics | below |
| `event_types` | its own namespace's event types |  |
| `ref_kinds` | kinds of thing a ref can name |  |
| `commands` | commands on oxplow's command bus | below |
| `jobs` | durable host operations scheduled by an extension |  |
| `ai_profiles` | a named model need an extension declares: bound by the person, or falling back to a built-in role |  |
| `effects` | scripts that react to logged events | below |
| `advisories` | guidance queries for the coding agent |  |
| `skills` | skills and slash commands for the coding agent |  |
| `implementations` | an implementation of a capability: a built-in, or an AI provider or effort policy written as a script | below |
| `providers` | an external program implementing a capability, such as the work list | [Provider protocol](provider-protocol.md) |
| `lenses/*.yaml` (files, not a key) | a page or panel over a SQL query | [Lenses](../guide/lenses.md) |
| `pages` | where its lenses show as pages |  |
| `panels` | where its lenses show as panels |  |
| `ui.slots` | a lens mounted into a core page |  |
| `ui.decorators` | labels on core refs |  |
| `ui.replacements` | a lens in place of a core component |  |
| `custom_components` | sandboxed HTML components for `viz: custom` lenses |  |
<!-- kinds:end -->

### Services

A script reaches oxplow through services. Declare each one your script
uses in its `needs:`:

```yaml
collectors:
  - id: todos
    runtime: starlark
    entry: collectors/todos.star
    needs: [tree.read, sql.read]
    facts: [acme.todos]
```

<!-- services:begin (generated from oxplow_domain::service::SERVICES; bless with OXPLOW_BLESS=1) -->
| `needs` | What it gives | Builtins | Approval | In a script that composes |
|---|---|---|---|---|
| `jobs.read` | Read this extension's durable job status and result: scope("jobs.read", {id}). |  | none | yes |
| `extension_config.read` | Read the resolved defaults, project and personal settings with their provenance: `scope("extension_config.read", {})`. |  | none | yes |
| `state.read` | Read this extension's workflow state or cache: `scope("state.read", {scope, area})` returns a snapshot or null. |  | none | yes |
| `sql.read` | Read-only SQL over the published models (`v_*`): `scope("sql.read", {sql, params?, page?})` returns every row, or one page. | `scope` | none | yes |
| `tree.read` | The files of the snapshot the run is anchored to, with their syntax, metrics and duplicate blocks. | `files`, `source_files`, `duplicate_blocks` | none | yes |
| `ai.compute` | Model calls on your AI provider: classify, score, summarize, extract, typed completion. Each answer is recorded, so the same question on the same text costs one call. | `ai_classify`, `ai_score`, `ai_summarize`, `ai_extract`, `ai_complete` | yours | no |
| `ai.discover` | Declared model identities, features and limits; no credentials or paid probes. | `ai_capabilities`, `ai_models` | none | yes |
<!-- services:end -->

- A fact collector reads the tree without declaring it, and a
  collector whose script calls a model builtin asks a model without
  declaring it. Everything else is declared.
- Declare `needs: [ai.discover]` to read `ai_capabilities(role = "main")`
  or `ai_models(role = "main")` in a collector. Missing metadata is unknown.
  Discovery reads no keys, makes no paid calls and works in pure previews.
- `ai_complete({"schemaVersion": 1, "messages": [...]}, role = "main")`
  sends ordered text or tool parts through the configured role. The host
  chooses the model; supplying `model` is an error. Image parts are currently
  refused. Providers may opt into `completeTyped` transport; legacy adapters
  accept only the conversation shape they can preserve. The response includes
  optional token counts and whether usage is observed, estimated or unknown.
- A script that uses a service it doesn't declare doesn't load: `check`
  says which builtin and which `needs` entry fixes it.
- A command's, an effect's or an effort policy's script composes
  commands and may run more than once, so it never asks a model. A
  command's script reads the semantic layer but not the tree: oxplow
  composes it inside a database transaction.
- `oxplow extension check --services` lists what each script uses and
  whether it declared it. Your coding agent has the same list through
  the `list_services` MCP tool.

### Collectors

```yaml
collectors:
  - id: prs
    runtime: starlark        # starlark | jaq | exec
    entry: collectors/prs.star
    input: "SELECT ref, title FROM v_work_item"
    trigger: { every: 15m }  # or manual (the default), or { on: [event types] }
    entities:
      - { name: pr, key: number, columns: { number: int, title: text } }
```

`input` is read-only SQL whose rows the script gets as `input.rows`.
More than 10,000 rows fails the run rather than handing over part of
them. To take a bounded read on purpose, declare the bound:
`input: { sql: "SELECT …", limit: 500 }`; the script then gets at most
that many rows and `input.complete` saying whether they were all of
them.

A Starlark or jaq collector runs sandboxed: no files, network,
environment or credentials beyond the services it declares. Every Starlark and jaq script — collectors,
commands, effects, policies — runs in its own worker process, which is
stopped when it runs past its time budget or its memory cap (`scripts:`
in Settings reference). It needs no approval, with one exception:
a Starlark script that calls a model (`ai_classify`, `ai_score`,
`ai_summarize`, `ai_extract`) spends your AI provider's key every run,
so you approve it in Settings → Data first, like an `exec` collector.
Any change to its files needs approving again.

### Commands

```yaml
commands:
  - name: review.finish             # its id is <namespace>.review.finish
    summary: Mark the task done.
    input_schema: { type: object, required: [ref], properties: { ref: { type: string } } }
    entry: handlers/finish.star     # transform(x) → { commands: [...] } | { refuse: "why" }
    needs: [sql.read]               # the scopes its script calls
    invokers: { human: true, agent: true, lens: true }
    ui: { label: Finish Review, group: Review, about: work_item, input: { ref: "{{ref}}" } }
```

- A script reaches oxplow only through the scopes in `needs`. Today a
  script can call `sql.read`; naming another scope is a load error.
- `scope("sql.read", { sql, params })` returns a list of row objects.
  More than 1,000 result rows fails instead of returning a partial list;
  exactly 1,000 is allowed. Aggregate in SQL to calculate over all
  matching rows, or explicitly filter/limit the query when you want a
  subset. To walk a large result a slice at a time, ask for a page:
  `scope("sql.read", { sql, params, page: { limit: 500, offset: 0 } })`
  returns `{ rows, more, next_offset }` (`limit` at most 1,000). Give
  the query an `ORDER BY`, or pages can overlap or skip rows; without
  one the answer carries a `warning`. This rule also applies to effects,
  effort policies and provider host calls. A complete result does not
  prove every source was collected.
- `ui` says how a person meets it. `about` offers it on that kind of
  ref's page and rows. `input` binds `{{stream}}`, `{{thread}}`,
  `{{ref}}` or `{{ref.id}}`, each as a whole string. `form` opens a
  page to gather the input instead; offered on a ref (`about`), the page
  opens with `?ref=<ref>`, which a `pages:` lens form prefills from with
  `defaults: { ref: "{{param.ref}}" }`. `open_after` opens a ref once it
  ran, taking `{{result.<field>}}` from its result.
- A mistake in `ui` is a load error naming the field, not a menu entry
  that never shows.

### Jobs

A job is model work that runs outside any transaction and survives a
restart. Its reply streams in as it comes, its model may call tools you
declare, and it can be cancelled mid-reply:

```yaml
jobs:
  - id: summary
    description: Summarize a review
    version: 1
    operation: ai.complete_typed
    needs: [ai.compute, sql.read]   # sql.read also gives the model query_sql
    tools: [todos.in_file]          # your commands its model may call
    max_tool_rounds: 4              # 1–16, default 4
    timeout: 120                    # seconds, 1–600, default 120
    replay: never                   # or idempotent: run again after a restart, up to 3 attempts
commands:
  - { name: summary.start, summary: Start a summary, scope: jobs.write, op: enqueue, invokers: { human: true, agent: true, lens: true } }
  - { name: summary.cancel, summary: Cancel a summary, scope: jobs.write, op: cancel, invokers: { human: true, agent: true, lens: true } }
  - { name: summary.status, summary: Read a summary, scope: jobs.read, op: snapshot, invokers: { human: true, agent: true, lens: true } }
  - name: todos.in_file
    summary: The TODO comments in one file.
    entry: commands/todos_in_file.star
    needs: [sql.read]
    input_schema: { type: object, required: [path], properties: { path: { type: string } } }
    access: read
    invokers: { human: false, agent: false, lens: false, job: true }
```

`enqueue` takes `{job, intentKey, request}`. For this operation, `request`
is `{schemaVersion: 1, role, messages, tools?}`: `role` is `main`,
`summarize`, `decide` or the id of one of your `ai_profiles`; the host
chooses the model for it, and a job that declares `tools` takes none here. The request
is private and limited to 1 MiB and 64 nesting levels. `cancel` and
`snapshot` take `{id}`. The `intentKey` is what the job is about, one job
per key: asking again with it returns the same job. Put the subject in it
(`review:effort:eff12`), and read it back as the snapshot's `intentKey` or
`v_job.intent_key`. The host records intent before a worker starts, and
a person approves each job declaration in Settings -> Data -> Programs
before it can spend an AI provider key; changing the declaration asks
again.

- **Tools.** Each is one of your commands that says `invokers: { job:
  true }`, only reads (`access: read`) and has an `input_schema`. The
  model calls it by its name with `.` as `_` (`todos_in_file`) and sees
  its `summary` and schema. Its input is checked against the schema; a
  refusal or error is answered to the model as `{error}` and the job goes
  on. A script tool answers its script's `result`. With `sql.read` the
  model also has `query_sql`, a read-only query over the published `v_*`
  models. After `max_tool_rounds` rounds the job fails. A job never
  writes: write from an effect on its end.
- **Output.** What the model writes, each tool call and its answer, and
  each round's tokens are the job's output as they happen: the
  `v_job_output` model (a lens over it updates live), and `text` in the
  `snapshot`. `v_job` lists every job and its state.
- **Its result** (`snapshot`'s `result`, once it ends): `{parts,
  finishReason, rounds, usage, aiCalls}`; a failure adds `error`; a
  cancelled job keeps what it had, with `stopped: cancelled`.
- **Cancel** stops a running job at once — its model call is dropped —
  and it ends `cancelled`. A person can stop any job from the Jobs lens.
  Past its `timeout` a job fails. An attempt cut off by a restart is
  `interrupted`, unless the job says `replay: idempotent`: then it runs
  again, up to three attempts in all, and only its final end is
  announced. A round it had finished costs nothing the second time; its
  tools run again, so they must only read (as every tool does).

A script declaring `needs: [jobs.read]` can call
`scope("jobs.read", {"id": job_id})` for its extension's result. The
`contribution.job.completed` event carries the job ID and `outcome`
(`succeeded`, `failed`, `cancelled` or `interrupted`), with no request or
result body; read the result through `jobs.read`. An agent or effect that
composes `enqueue` does not acquire the job's AI permission.

### AI profiles

A profile is a named model need your extension declares, so a job or
script can ask for "the model that reviews" without naming one:

```yaml
ai_profiles:
  - id: review                      # [a-z][a-z0-9_]*, unique in the extension
    description: Reviews an effort's evidence.   # at most 4096 bytes
    fallback: summarize             # main | summarize | decide (required)
    features: [tool_calling]        # optional: text | tool_calling | streaming | image
```

Name it by its id where your extension picks a role: a job request's
`role: review`, a collector's `ai_classify(text, labels, role = "review")`
(every `ai_*` builtin takes `role=`). Another extension's profiles aren't
yours to name. The person binds `<extension>/<id>` to a model in
Settings → AI (or a project in `.oxplow/project.yaml`'s `ai.roles`);
until they do, the `fallback` role's model serves it. `features` are what
the bound model must support: a job's typed call on a model that
reports one unsupported is refused, and Settings shows why. A profile
picks a model, nothing more — what a job's model may call is still its
`tools:`. `oxplow extension check` prints one line per profile saying
what serves it.

### Replacing a core view

A lens can stand in for one of these core views (`ui.replacements: [{
target, lens, version? }]`). Its lens declares every prop as a `param`
and gets them instead of any host state; `version`, when given, must be
the contract's (a changed contract refuses an old declaration instead of
handing it props it doesn't know). An unbound view may be replaced by
any extension — a person picks whose shows in Settings → Integrations
(the project's `replacements: { <target>: <extension> | oxplow }`); a
bound one only by the extension of the provider whose item it shows.
The same table is the `v_replaceable` model.

<!-- replaceables:begin (generated from oxplow_domain::replaceable::REPLACEABLE; bless with OXPLOW_BLESS=1) -->
| Target | What it is | Bound to a provider | Version | Props |
|---|---|---|---|---|
| `work_item.board` | board | no | 1 | `scope`, `thread_id` |
| `work_item.list` | list | no | 1 | `scope`, `thread_id` |
| `work_item.detail` | detail editor | no | 1 | `ref` |
| `work_item.detail.state` | state control | yes | 1 | `ref` |
| `vcs.history.graph` | history graph | no | 1 | `head_sha`, `reveal_sha`, `limit` |
<!-- replaceables:end -->

A view of work items reads `v_work_item` and changes items only through
the interface commands (`oxplow.work_item.update`, `transition`,
`reorder`, `move`, …), so it works with whichever list is active; the
active list's `features` (`v_capability_provider`) say which of them it
takes.

### Effects

```yaml
effects:
  - id: announce-done
    summary: Note a finished item on its thread.
    on: [work_item.state_changed]
    where: { to: done }
    needs: [sql.read]
    entry: effects/announce.star    # transform({ event }) → { commands } | { skip: "why" }
```

An effect runs as a program you approve in Settings → Data, and only
on events logged after the approval. Any edit to the extension's files
stops it until it's approved again.

### The event your script gets

An effect, an effort policy and an `on:` collector all get the same
event object. A Starlark or jaq collector gets it as `input.event`, and
an exec collector as `{"event": …}` on stdin (`{}` when nothing
triggered it):

```json
{
  "id": "0199…", "type": "work_item.state_changed", "v": 1, "seq": 812,
  "at": "2026-10-09T14:02:11.120000Z", "source": "agent:thr3",
  "subject": ["work_item:oxplow:tsk4"],
  "anchors": { "stream_id": "str1", "thread_id": "thr3", "effort_id": null,
               "turn_id": 41, "agent_session_id": "ses2", "snapshot_id": null },
  "payload": { "work_item": "work_item:oxplow:tsk4", "to": "done" },
  "cause": null
}
```

- Every key is always there; what the event doesn't have is `null`.
- `payload` is always at `v`, the type's newest version: an event
  logged before its type gained a version is carried forward through
  the type's upcasts first.
- An example's `event:` is `{ type, v?, payload, subject?, anchors? }`
  (and optionally `id`, `seq`, `at`, `source`, `cause`). Your script
  gets it the same way: what you leave out is `null` (`source` is
  `fixture`), and an older `v` is carried forward, so a v1 fixture tests
  what your script sees of an old event. A type oxplow doesn't know, a
  newer `v`, a payload that doesn't match its schema, a malformed anchor
  (`thread_id: thr1`, `turn_id: 7`) or a key the event doesn't have
  fails the example and says which.

### AI providers

An AI provider is a Starlark script: oxplow's own (Anthropic, OpenAI,
OpenRouter, TypeSafe) are written this way. The script builds each request
and reads each reply; oxplow makes the HTTP call.

```yaml
implementations:
  - capability: ai_provider
    id: acme                       # the `kind:` a provider in Settings → AI names
    title: Acme
    entry: providers/acme.star
    config: { baseUrl: "https://api.acme.example", ops: [complete] }
```

```python
def request(x):     # { op: "complete", model, system, prompt, json }
    return {
        "path": "/v1/chat",                                 # under the base URL
        "headers": {"authorization": "Bearer {{key}}"},      # oxplow fills in the key
        "body": {"model": x["model"], "prompt": x["prompt"]},
    }

def response(x):    # { op, model, body } — a 2xx reply
    return {"text": x["body"]["text"], "usage": {"input": 0, "output": 0}}
```

- `path` is always under the base URL: the one the person configured,
  else `baseUrl`.
- `{{key}}` is the provider's key from the keychain. oxplow puts it in
  after `request` returns, so the script never sees it.
- oxplow reads the status itself: 401 and 403 are a key problem, 429 a
  rate limit.
- `ops: [complete, decide]` answers typed questions natively. Without
  `decide`, oxplow asks them as a chat. For `decide`, `request` gets
  `{ op: "decide", model, state, questions }` and returns `None` to ask a
  given model as a chat instead; `response` returns
  `{ answers: { <name>: { type: noul, probability } | { type: choice, choice, probabilities } | { type: score, score, probabilities } } }`.
- A provider runs only once a person approves its script in Settings →
  Data → Programs, and any change to the script asks again.

### Effort policies

An effort policy decides when a thread's effort opens, closes and links
to a work item. oxplow's own opens one per commit or task switch; a
script can be yours instead. `oxplow extension new policy <name>`
writes one that opens an item's effort when an agent starts it on a
thread and closes it when the item is done. For a fuller one, see
`examples/extensions/prompt-efforts`: it gives every prompt that changes
files an effort of its own, reacting to each turn's `thread.checkpoint`.

```yaml
implementations:
  - capability: effort_policy
    id: acme                       # what Settings → Capabilities offers
    title: Acme's policy
    entry: policies/acme.star      # transform({ event }) → { commands } | { skip: "why" }
    needs: [sql.read]              # the scopes it calls
```

- The script gets one of core's events (`type`, `payload`, `subject`,
  and `anchors` such as the `thread_id` an agent moved an item on) and
  returns the commands to run, or a skip. It can't append events.
- It reads oxplow only through `scope("sql.read", { sql, params })`,
  and only when it `needs` it.
- It runs once a person approves it in Settings → Data → Programs, and
  any change to the extension's files asks again. Until then, every
  event it hears fails, visibly.
- An intent example dry-runs it: `input: { implementation: acme,
  event: { type, payload, anchors? }, answers? }`, `expect: { commands:
  [names] }` or `{ skip: $any }`.

## Checking and testing

`oxplow extension check` loads the extension the way oxplow does and
dry-runs its models, commands, lenses and advisories. Each finding is
one line: `file:line: what — fix`. Each of its `imports:` gets a line
saying how it resolved here: `` import `items` → maker/model/items v1:
available ``, or `unavailable (maker exports v2)` and the like, which is
when an optional import's guarded contributions are withheld. `extension
test` skips an example whose lens, job, command or other target was
withheld, with a warning saying which import withheld it and why.
A fixture runs only when an intent
example names it, so `check` warns about a `fixtures/<name>.yaml` no
example lists (`model-*` and `provider-*` fixtures belong to their model
or provider), and `test` warns about a listed example with no fixture;
each names the file and the fix. For a `sharing: shared` extension, `check`
also warns about text that looks like an embedded credential in
`extension.yaml`, a script or a fixture: a known token prefix (`ghp_`,
`github_pat_`, `sk-`, `sk-ant-`, `xoxb-`, `AKIA`, `glpat-`, …), a private
key header, or a long random-looking literal (hex digests and uuids
excepted). The warning names the file and line and shows the prefix and
length, never the value; a collector's `credentials:` keeps a real one
in the keychain, and one that was ever committed should be rotated.
`--impact` adds what your working
tree's version changes against git `HEAD`. `--services` lists the
services each script uses.

`oxplow extension test` runs on a throwaway oxplow: the check, then
each intent example's fixture, then each declared provider's
handshake, examples and conformance suite, and each policy script's
conformance suite. Its report carries the same import lines. A
command's fixture may check what the command answers, `expect: { result:
{...} }`, compared on the keys you name. A job's fixture can make the
data its tools read first: `setup: [{ command, input?, as? }]` runs
oxplow's own commands in order (as a person, or `as: agent` for the agent
of `thread:thr1`), and `rows` seeds your own entities as a lens fixture
does. A fixture model's reply may say `when: "<text>"` to answer only the
request whose last message carries that text, such as a tool's result;
other replies answer in turn. `--bless` writes a provider's golden
transcript.

Your coding agent has the same checks as MCP tools
(`validate_extension`, `run_collector`, `preview_collector`), and the
`oxplow-extension` skill walks it through building one.
