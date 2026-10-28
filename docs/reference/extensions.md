# Extensions

An extension is a folder under `.oxplow/extensions/<name>/` with an
`extension.yaml`. Everything it adds is declared there as data:
pages, models, collectors, commands, effects, providers and the rest.
Each of those is a *contribution*.

The goal is personal tools that agents can help you build, with useful
customizations shareable with a team and eventually more broadly. Core
provides the shared data and action contracts; an extension adds its own
analysis, presentation, integration, or workflow.

Repository code is untrusted regardless of who authored or shared it.
Declared permissions that a person approves at deploy, immutable deployments,
command gates, and component isolation provide guardrails. A script runs in
its own worker process, which oxplow kills when the script passes its time
limit or its memory cap. External-program isolation depends on the
platform; see [collector limits](../guide/pages.md#collectors-bringing-in-outside-data).

```sh
oxplow extension new extension my-review   # scaffold one that checks clean
oxplow extension check my-review           # every problem, as file:line
oxplow extension test my-review            # run its examples on a throwaway oxplow
```

`new` takes a kind instead of `extension` to start from that kind's
starter: `page`, `collector`, `command`, `effect`, `provider`,
`component`, `policy`, `advisory`, `skill` or `panel`. A provider takes `--capability` for what it
implements: `work_items` (the default), `effort_policy`,
`agent_harness` (a harness that tells its subagents apart also declares
the `subagents` feature; see the provider protocol), `snapshots` (its
example marks a one-file tree under `fixtures/tree`; the `contents`
feature adds `read_at`) or `knowledge` (its example writes a page). Each
scaffold checks clean and passes `test` as written; an `advisory` or
`skill` has no example `test` can run (it runs pages, collectors,
commands, effects and providers), so `test` notes that and nothing more.

## Sources, deploying and sharing

An extension has a **source** (the files you edit) and **deployments**
(the copies oxplow runs). Nothing runs a source. An edit changes nothing
until you deploy it.

- The source is `.oxplow/extensions/<name>/`, or the folder that
  `extensionSources: { <name>: <folder> }` names in `.oxplow/project.yaml`
  or `.oxplow/personal.yaml`. `.oxplow/.gitignore` keeps a source out of
  git, so a new extension is personal.
- `oxplow.extension.deploy { name }` (or `oxplow extension deploy <name>`)
  copies the source into `.oxplow/runtime/extensions/<name>/<hash>/` and
  makes it live. It returns once the extension runs. If the extension does
  not load, the deploy fails and the old deployment keeps running.
  `oxplow.extension.rollback { name, to? }` makes an earlier one live.
- A source inside the project deploys only from the main checkout (the
  one the project was opened from), uncommitted changes included. A copy in
  another worktree is a draft: check and test it there; it deploys once the
  change reaches the main checkout's branch. A source outside the project
  has one copy and deploys from anywhere.
- Oxplow deploys nothing at start: it runs the deployments recorded on this
  machine. An extension that is on but never deployed here runs nothing
  until someone deploys it.
- Which extensions are on: `extensions: [..]` in `.oxplow/project.yaml`
  (absent means `[oxplow-bundled]`), plus `enableExtensions` and
  `disableExtensions` in `.oxplow/personal.yaml`.
  `oxplow.extension.enable`, `disable` and `remove` change the lists.
  `remove` never deletes the source.
- `oxplow.extension.share { name }` un-ignores the source and moves the name
  to the team list; you commit the files. `oxplow.extension.unshare { name }`
  reverses it.
- To use someone else's extension, clone it into
  `.oxplow/extensions/<name>/` (or point `extensionSources` at it), list it
  and deploy it.
- `oxplow extension check` and `test` read the source, so they need no deploy.

## Editor support

The scaffolded files start with a schema line, so an editor running
yaml-language-server (VS Code's YAML extension, for one) completes
keys and flags mistakes as you type:

```yaml
# yaml-language-server: $schema=https://nvoxland.github.io/oxplow/reference/schemas/extension.schema.json
```

Add the line to files you wrote by hand. The loader is still the
authority: `oxplow extension check` catches what a schema can't, like a
page whose loader reads one declared after it.

## `extension.yaml`

```yaml
manifest: 2
name: my-review          # must equal the folder name
description: Pages for reviewing agent work
intent:
  purpose: What question it answers, or what job it does
  examples:
    - { name: blocked, input: { page: blocked } }
```

- `manifest: 2` and `intent` are required. Unknown keys are errors.
- `intent.examples` are the inputs `oxplow extension test` runs, each
  with a fixture under `fixtures/<example>.yaml` saying what to expect.
- `version: 1.2.0` (`MAJOR.MINOR.PATCH`) and `engine: ">=0.7"` (the oxplow
  it targets) are optional.
- The folder name is lowercase letters, digits and single dashes,
  starting with a letter.
- A metric, measure, dimension or fact collector's key is shared by every
  extension in the project: when two declare the same one, the first
  (alphabetically) keeps it and the other gets a load error naming
  both.

### Public imports and exports

Export a contribution explicitly, using its canonical URI (the resource
`oxplow/<kind>` with the kind's own name as the key param) and a positive
integer contract version:

```yaml
exports:
  - contribution: "oxplow:oxplow/model?model=review-data/items"
    version: 1
    visibility: stable
```

The consumer chooses a local binding:

```yaml
imports:
  - id: review_items
    contribution: "oxplow:oxplow/model?model=review-data/items"
    version: 1
    optional: true
import_guards:
  - contribution: "oxplow:oxplow/collector?collector=review-ui/summarize"
    imports: [review_items]
```

A model or page reads that binding with
`SELECT * FROM import_ref('review_items')`. SQL references infer their own
guard. An unavailable optional binding withholds the referring contribution;
independent contributions remain usable. Explicit guards can name models,
pages, collectors, commands, effects, advisories, skills or providers. Their
exports and dependent mounts are withheld too. Each unavailable optional
import is one warning naming why and everything it withheld:
`` optional import `items` for `oxplow:oxplow/model?model=maker/items` v1 is unavailable: maker
is disabled; withheld: `oxplow:oxplow/page?page=review-ui/items`, `oxplow:oxplow/slot?slot=review-ui/oxplow-foundation.work-item.sections-0` ``.
Missing required imports and
dependency cycles block the extension with source-located errors.

Versions must match exactly. A model or event export must use its native
contract version. Package `version` and host `engine` are separate.
Visibility defaults to `private`, which other extensions cannot import.
`experimental` and `stable` exports can be imported. Import targets resolve by canonical
ID; published reference metadata is returned only after resolution.

Imports grant no SQL, network, command or other authority. Existing foreign-event
subscriptions retain their behavior. There is no general extension service-call
surface in this contract. Authoring checks and change review resolve the candidate
alongside current project dependencies. A required consumer broken by a change
is reported by `check --impact` before you deploy it. Coherent runtime generation
activation is still in progress.

### Configuration declarations

The loader validates a versioned settings declaration.

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

### Migrating settings

When a later version of the extension changes the settings schema, bump
`version` and list how stored settings move forward:

```yaml
configuration:
  version: 3
  schema: { ... }
  migrations:
    - { from: 1, to: 2, entry: config/migrate_1_2.star }
    - { from: 2, to: 3, entry: config/migrate_2_3.star }
```

Each `entry` is a Starlark file in the extension folder defining
`def transform(values):` that takes a layer's stored settings (an object) and
returns the settings for the next version. It is pure: it cannot read files or
call services. The steps must each go forward, chain with no gap or overlap, and
end at `version`. When oxplow starts the new version it carries every stored
layer (project and personal) from its version to `version`, and checks the result
against the schema. If a step fails, or its result does not satisfy the schema,
nothing is changed: the extension is held back with the step's error shown in
Settings → Extensions, and the previous version, if any, keeps running on the
settings it reads. Fix the step and activate again.

Schema compilation is offline. Diagnostics contain locations and error codes,
with values omitted. Use non-secret settings only: credentials belong to the
host's secret service. Settings declarations grant no permissions.

Host secret references are identity-only objects:
`{schemaVersion: 1, id: "<UUIDv7 returned by the host>"}`. The value is set with
`oxplow.extension.set_secret` (Settings → Extensions → Provider secrets); reads
(`v_extension_secret`) return only whether it is set.
References can be stored as non-secret settings, and copying them to another
extension or project does not copy access.

The first supported consumer is a remote MCP HTTP provider. Declare
`auth: OXPLOW_MCP_BEARER` in `adapter.mcp`, and a credential with
`name: OXPLOW_MCP_BEARER` and `secretRef: <the reference object>`. A person must
approve the extension's permissions when it is deployed. The shipped HTTP transport sends the
bearer; extension scripts cannot read its value. References are refused for
package programs, stdio servers and OAuth credentials. Settings → Extensions
lists each declared reference with Set… and Revoke.

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
| `resources` | kinds of thing a URI can name |  |
| `commands` | commands on oxplow's command bus | below |
| `jobs` | durable host operations scheduled by an extension |  |
| `ai_profiles` | a named model need an extension declares: bound by the person, or falling back to a built-in role |  |
| `effects` | scripts that react to logged events | below |
| `advisories` | guidance queries for the coding agent |  |
| `skills` | skills and slash commands for the coding agent |  |
| `implementations` | an implementation of a capability: a built-in, or an AI provider or effort policy written as a script | below |
| `providers` | an external program implementing a capability, such as the work list | [Provider protocol](provider-protocol.md) |
| `pages/*.yaml` (files, not a key) | a page: its data loaders and the view that shows them | [Pages](../guide/pages.md) |
| `panels` | a rail panel showing its pages compact |  |
| `ui.slots` | a page mounted at another page's slot |  |
| `ui.decorators` | labels on core refs |  |
| `ui.replacements` | a page in place of a core component |  |
| `custom_components` | sandboxed HTML components a page draws, fed its loaders |  |
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
| `needs` | What it gives | Builtins | In a script that composes |
|---|---|---|---|
| `jobs.read` | Read this extension's durable job status and result: scope("jobs.read", {id}). |  | yes |
| `extension_config.read` | Read the resolved defaults, project and personal settings with their provenance: `scope("extension_config.read", {})`. |  | yes |
| `state.read` | Read this extension's workflow state or cache: `scope("state.read", {scope, area})` returns a snapshot or null. |  | yes |
| `sql.read` | Read-only SQL over the published models (`v_*`): `scope("sql.read", {sql, params?, page?})` returns every row, or one page. | `scope` | yes |
| `tree.read` | The files of the snapshot the run is anchored to, with their syntax, metrics and duplicate blocks. | `files`, `source_files`, `duplicate_blocks` | yes |
| `ai.compute` | Model calls on your AI provider: classify, score, summarize, extract, typed completion. Each answer is recorded, so the same question on the same text costs one call. | `ai_classify`, `ai_score`, `ai_summarize`, `ai_extract`, `ai_complete` | no |
| `ai.discover` | Declared model identities, features and limits; no credentials or paid probes. | `ai_capabilities`, `ai_models` | yes |
<!-- services:end -->

- A fact collector reads the tree without declaring it, and a
  collector whose script calls a model builtin asks a model without
  declaring it. Everything else is declared.
- An effect or command that declares `needs: [ai.discover]` can ask
  whether a model serves a role without spending anything:
  `scope("ai.discover", {"role": "review"})` answers `{role, model:
  {provider, model}, problem: null}` when one does (a bare profile id is
  your extension's; an unbound profile reports its fallback's model), or
  `{role, model: null, problem}` saying why not. Use it to skip when no
  model is bound.
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
  whether it declared it. Your coding agent reads the scopes from
  `v_scope`.

### Collectors

```yaml
collectors:
  - id: prs
    runtime: starlark        # starlark | jaq | exec
    entry: collectors/prs.star
    input: "SELECT uri, title FROM v_work_item"
    on: { every: 15m }  # or manual (the default), or { on: [event types] }
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
in Settings reference). It needs no permission, with one exception:
a Starlark script that calls a model (`ai_classify`, `ai_score`,
`ai_summarize`, `ai_extract`) spends your AI provider's key every run,
so the extension must declare `permissions: [ai.compute]`. The loader
refuses it otherwise.

A collector that records facts declares `facts:` (the measures it writes)
instead of `entities:`. Add `whole_tree: true` when it must read every file
on every run (a duplication scan) rather than only what the snapshot changed.

The metrics over those facts are declared under `metrics:`:

```yaml
metrics:
  - key: acme.todos
    sourceMeasure: acme.todo
    aggregation: count
    collector: acme.todo_scan   # the collector in this extension that computes it
    default: off                # on only once the project lists it with `use:`
```

A metric is on unless the project turns it off; `default: off` makes it
opt-in. A collector that metrics name runs only while one of them is on,
so an opt-in metric costs nothing until someone wants it. Keys starting
`oxplow.` are oxplow's own.

The project's own measures, collectors and metrics are an extension too:
`.oxplow/extensions/project/extension.yaml`. Every extension that does not
ship with oxplow is the project's own. Only a non-bundled extension may
declare an `exec` fact collector, a `records:` report collector (test,
coverage and analysis reports) or `promote:`, and it declares the permissions
those need, like any extension. `.oxplow/project.yaml` takes only `use:` entries and
`enabled: false` markers for them.

### Commands

```yaml
commands:
  - name: review.finish             # its id is <namespace>.review.finish
    summary: Mark the task done.
    input_schema: { type: object, required: [uri], properties: { uri: { type: string } } }
    entry: handlers/finish.star     # transform(x) → { commands: [...] } | { refuse: "why" }
    needs: [sql.read]               # the scopes its script calls
    ui: { label: Finish Review, group: Review, about: oxplow/work_item, input: { uri: "{{uri}}" } }
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
- URIs are carried, not assembled. Every script has `uri`:
  `uri.build(resource, **params)` makes one (`uri.build("oxplow/file",
  path = f["path"])`), and `uri.parse(u)` (`{resource, params, view}`),
  `uri.param(u, name)`, `uri.id(u, resource=)`, `uri.resource(u)` and
  `uri.is_uri(v)` read one. In SQL, select a model's `uri` column and
  compare URIs to URIs (`WHERE uri = :uri`); build one with
  `uri('oxplow/effort', 'id', json_extract(e.payload, '$.effort'))` and
  read a param with `uri_param(uri, 'id')`. Don't concatenate
  `'oxplow:oxplow/effort?id=' || id` or slice a URI. A field named for one
  resource (a command input `thread`, `effort`, `session`, `claim`, an event
  payload field) carries that resource's bare id, not a URI: a page or form
  that feeds one selects `uri_param(x.uri, 'id')` (or the model's integer
  `id` for a claim or decision), and a script builds a URI with
  `uri.build("oxplow/effort", id = e)` only where it becomes a link target
  or is compared with a `uri`. A URI can outlive what it names, so store it even when
  the thing may go.
- `ui` says how a person meets it. `about` names a resource
  (`oxplow/work_item`, `acme/pr`) and offers the command on a URI of it, its page and rows. `input` binds `{{stream}}` and `{{thread}}` (bare ids),
  `{{uri}}` (the page's URI) or `{{uri.id}}` (its key), each as a whole string. `form` opens a
  page to gather the input instead; offered on a URI (`about`), the page
  opens with `uri=<uri>`, which a page's form prefills from with
  `defaults: { uri: "{{param.uri}}" }`. `open_after` opens a URI once it
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
  - { name: summary.start, summary: Start a summary, scope: jobs.write, op: enqueue }
  - { name: summary.cancel, summary: Cancel a summary, scope: jobs.write, op: cancel }
  - { name: summary.status, summary: Read a summary, scope: jobs.read, op: snapshot }
  - name: todos.in_file
    summary: The TODO comments in one file.
    entry: commands/todos_in_file.star
    needs: [sql.read]
    input_schema: { type: object, required: [path], properties: { path: { type: string } } }
    access: read
    job: true
```

`enqueue` takes `{job, intentKey, request}`. For this operation, `request`
is `{schemaVersion: 1, role, messages, tools?}`: `role` is `main`,
`summarize`, `decide` or the id of one of your `ai_profiles`; the host
chooses the model for it, and a job that declares `tools` takes none here. The request
is private and limited to 1 MiB and 64 nesting levels. `cancel` and
`snapshot` take `{id}`. The `intentKey` is what the job is about, one job
per key: asking again with it returns the same job. Put the subject in it
(`review:eff12`), and read it back as the snapshot's `intentKey` or
`v_job_run.intent_key`. The host records intent before a worker starts, and
the extension must declare `permissions: [ai.compute]` before a job can
spend an AI provider key, and a person approves that when the extension is
deployed.

- **Tools.** Each is one of your commands that says `job: true`, only reads (`access: read`) and has an `input_schema`. The
  model calls it by its name with `.` as `_` (`todos_in_file`) and sees
  its `summary` and schema. Its input is checked against the schema; a
  refusal or error is answered to the model as `{error}` and the job goes
  on. A script tool answers its script's `result`. With `sql.read` the
  model also has `query_sql`, a read-only query over the published `v_*`
  models. After `max_tool_rounds` rounds the job fails. A job never
  writes: write from an effect on its end.
- **Output.** What the model writes, each tool call and its answer, and
  each round's tokens are the job's output as they happen: the
  `v_job_run_output` model (a page over it updates live), and `text` in the
  `snapshot`. `v_job_run` lists every job and its state.
- **Its result** (`snapshot`'s `result`, once it ends): `{parts,
  finishReason, rounds, usage, aiCalls}`; a failure adds `error`; a
  cancelled job keeps what it had, with `stopped: cancelled`.
- **Cancel** stops a running job at once — its model call is dropped —
  and it ends `cancelled`. A person can stop any job run from the Job Runs page.
  Past its `timeout` a job fails. An attempt cut off by a restart is
  `interrupted`, unless the job says `replay: idempotent`: then it runs
  again, up to three attempts in all, and only its final end is
  announced. A round it had finished costs nothing the second time; its
  tools run again, so they must only read (as every tool does).

A script declaring `needs: [jobs.read]` can call
`scope("jobs.read", {"job_run": payload["job_run"]})` for its extension's result. The
`contribution.job.completed` event carries the run (`job_run`), the job
(`job`, which `where: { job: "job:<extension>/<id>" }` matches) and
`outcome` (`succeeded`, `failed`, `cancelled` or `interrupted`), with no
request or result body; read the result through `jobs.read`. An agent or effect that
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

### Resources

A resource is a kind of thing your extension adds: pull requests, tickets,
deployments. `resources:` declares it, and every thing of it gets a URI,
`oxplow:<extension>/<resource>?<params>` (`oxplow:github/pr?id=12`).

```yaml
resources:
  - name: pr
    label: Pull request
    params: { id: { required: true, pattern: '^\d+$' } }
    resolve: pull_request     # a model with `uri` and `title` columns
    searchable: pull_request  # optional: a model with `uri`, `title`, `body`
    icon: git-pull-request
    wikilink: { prefix: pr, param: id }   # [[pr:12]]
```

- `params` are the named values that identify one. A `pattern` is
  anchored (`^…$`) and limited to plain characters, classes, `\d`, `\w`,
  escapes and quantifiers: no groups, alternation or `.`.
- `resolve` gives each thing a title in links and backlinks. `searchable`
  puts the model's rows in search; a row whose `uri` isn't a URI of this
  resource is skipped.
- A URI that nothing resolves (the extension is off, or the thing is
  gone) shows as its raw text. It never errors.
- A resource has no page of its own to name. A URI opens as itself, and
  a page you declare with `for:` draws it ("Showing a resource" below).
- `about:` on a command or a prompt, and `resource:` on a decorator, take
  a resource name: `oxplow/work_item`, `github/pr`.

`oxplow extension check` refuses an unknown model, a pattern
outside that subset, an icon oxplow doesn't draw, and a resource or
wikilink prefix that another extension already uses.

#### Showing a resource

A page with `for:` (in its page file) is a **renderer**: it draws the URIs of a resource.
When a person opens `oxplow:github/pr?id=12`, oxplow picks one of the
pages that name `github/pr` in `for:`.

```yaml
# pages/pr.yaml
title: Pull request
icon: git-pull-request
for: github/pr          # a resource, or a list of them
default: true           # the page a plain open uses
accepts: { id: '^\d+$' }   # optional: only URIs whose params fit
label: "PR #{{param.id}}"  # optional: what its tab says
params: [{ name: uri }, { name: id }]   # filled from the URI
data:
  pr: { sql: "SELECT * FROM v_github_pr WHERE number = :id" }
view: { viz: table, from: pr }
```

- `for` names a resource of your own, one of oxplow's (`oxplow/file`),
  another extension's, or `https` / `http` to draw web pages.
- `accepts` has one entry per param of the resource. The value is
  `present`, `absent` or an anchored pattern like the ones in `params`.
  The page draws a URI only when every entry holds.
- `default: true` is for the resources your extension declares, and one
  page per resource. A page that draws another extension's resource is
  never its default: the person picks it with **Open With** in the page's ☰ menu.
- `pinned: true` pins the tab the page opens in.
- `label` is what the page's tab says, if not its `title`.
  `{{param.<name>}}` is replaced with that param of the URI (of a
  route page: one of its `params`). `title` stays the page's name in the
  launcher and in **Open With**.
- A page that renders may declare a `uri` param. It runs with the
  URI of the thing. A param it declares with the same name as one of the
  URI's params (`id`, `path`) gets that value, so the query can use it
  directly. Any other param must be a filter (`role: filter`).
- Nothing draws a URI? oxplow shows its label and params, or the raw URI
  in a muted style if the thing is gone. This is not an error.

### Replacing a core view

A page can stand in for one of these core views (`ui.replacements: [{
target, page, version? }]`). Its page declares every prop as a `param`
and gets them instead of any host state; `version`, when given, must be
the contract's (a changed contract refuses an old declaration instead of
handing it props it doesn't know). An unbound view may be replaced by
any extension — a person picks whose shows in Settings → Integrations
(the project's `replacements: { <target>: <extension> | oxplow }`); a
bound one only by the extension of the provider whose item it shows.
The same table is the `v_replaceable` model.

<!-- replaceables:begin (generated from oxplow_domain::replaceable::REPLACEABLE; bless with OXPLOW_BLESS=1) -->
| Target | What it is | Bound to a provider | Version | Props | View |
|---|---|---|---|---|---|
| `work_item.board` | board | no | 1 | `scope`, `thread_id` | — |
| `work_item.list` | list | no | 1 | `scope`, `thread_id` | — |
| `work_item.detail` | description editor | no | 1 | `uri` | — |
| `work_item.detail.state` | state control | yes | 1 | `uri` | — |
| `vcs.history.graph` | history graph | no | 2 | `head_sha`, `reveal_sha`, `limit` | `q`, `author`, `branch` |
| `effort.verdict` | effort verdict | no | 1 | `effort_id` | — |
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

An effect reacts only to events logged after it started. It starts when
the deployment that first runs it goes live. **Backfill** reaches earlier
events. A changed file needs a redeploy, which asks you only if it adds a
permission.

### Ordering

When one event reaches several effects or collectors, `after:` says
which run first:

```yaml
effects:
  - id: notify
    on: [work_item.state_changed]
    entry: effects/notify.star
    after: [react, "collector:acme/gather", change.analyze]
```

An entry is one of:
- a bare id: this extension's own effect or collector;
- a short URI: `effect:<extension>/<id>` or `collector:<extension>/<id>`,
  any extension's (the short forms of `oxplow:oxplow/effect?effect=…` and
  `oxplow:oxplow/collector?collector=…`);
- one of oxplow's own steps, by name (`change.analyze`, `collection`,
  `page_ref.work_item`, …), or as `consumer:<name>`.

A target nothing declares, or contributions that wait on each other, is
a load error, and the contribution doesn't run. A target in a disabled
extension is a warning: it runs without waiting. `oxplow extension
check` prints each one's place (`order effect:acme/notify → level 2,
after …`), and `v_contribution` holds it (`after`, `order_level`).

Ordering is only about one event's handling. It isn't a dependency
(`imports:` holds an extension back when a contract it reads is
missing) and it isn't causality (an event's `cause` says which event's
handling logged it).

### The event your script gets

An effect, an effort policy and an `on:` collector all get the same
event object. A Starlark or jaq collector gets it as `input.event`, and
an exec collector as `{"event": …}` on stdin (`{}` when nothing
triggered it):

```json
{
  "id": "0199…", "type": "work_item.state_changed", "v": 1, "seq": 812,
  "at": "2026-10-09T14:02:11.120000Z", "source": "agent:thr3",
  "subject": ["oxplow:oxplow/work_item?id=tsk4&provider=oxplow"],
  "anchors": { "stream_id": "str1", "thread_id": "thr3", "effort_id": null,
               "turn_id": 41, "agent_session_id": "ses2", "snapshot_id": null },
  "payload": { "work_item": "oxplow:oxplow/work_item?id=tsk4&provider=oxplow", "to": "done" },
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

An AI provider names one of oxplow's provider kinds. Each kind is Rust
code in oxplow, built on [Rig](https://github.com/0xPlaygrounds/rig) for
all but TypeSafe. oxplow's own extension declares one of each. Your
extension can declare another under its own id, for example to point a
kind at a proxy by default.

```yaml
implementations:
  - capability: ai_provider
    id: acme                       # the `kind:` a provider in Settings → AI names
    title: Acme
    provider: openai_compatible    # the kind it is
    config: { baseUrl: "https://llm.acme.example/v1" }
```

- The kinds: `anthropic`, `openai`, `openai_compatible` (any server that
  speaks OpenAI's Chat Completions), `openrouter`, `gemini`, `mistral`,
  `groq`, `deepseek`, `xai`, `ollama`, `llamacpp` and `typesafe`.
- `baseUrl` is the URL a provider uses when the person names none in
  Settings → AI. It is the only `config` key.
- The key always comes from the person's keychain.
- Declaring a provider needs `ai.compute`: it points a person's key
  at its URL. A person approves it when the extension is deployed.

### Effort policies

An effort policy decides when a thread's effort opens, closes and links
to a work item. oxplow's own opens one per commit or task switch; a
script can be yours instead. `oxplow extension new policy <name>`
writes one that opens an item's effort when an agent starts it on a
thread and closes it when the item is done. For a fuller one, see
the `prompt-efforts` example (`oxplow extension new --from prompt-efforts`): it gives every prompt that changes
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
- It runs from the extension's deployment, under its declared
  permissions. A changed file needs a redeploy, which asks only if it adds
  a permission.
- An intent example dry-runs it: `input: { implementation: acme,
  event: { type, payload, anchors? }, answers? }`, `expect: { commands:
  [names] }` or `{ skip: $any }`.

## Checking and testing

`oxplow extension check` loads the extension the way oxplow does and
dry-runs its models, commands, pages and advisories. Each finding is
one line: `file:line: what — fix`. Each of its `imports:` gets a line
saying how it resolved here: `` import `items` → model:maker/items v1:
available ``, or `unavailable (maker exports v2)` and the like, which is
when an optional import's guarded contributions are withheld. `extension
test` skips an example whose page, job, command or other target was
withheld, with a warning saying which import withheld it and why.
A fixture runs only when an intent
example names it, so `check` warns about a `fixtures/<name>.yaml` no
example lists (`model-*` and `provider-*` fixtures belong to their model
or provider), and `test` warns about a listed example with no fixture;
each names the file and the fix.
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
of `oxplow:oxplow/thread?id=thr1`), and `rows` seeds your own entities as a page fixture
does. A fixture model's reply may say `when: "<text>"` to answer only the
request whose last message carries that text, such as a tool's result;
other replies answer in turn. `--bless` writes a provider's golden
transcript.

A page's fixture is `input: { page: <slug>, params? }`, with `given:` beside
`input` (steps run first, in order: `{ command, input?, as? }` or `{ rows: {
<entity>: [...] } }`) and `expect: { loaders: { <name>: { rows: n | $any |
[ { col: val } ], columns? } }, nodes: [{ id, kind? }], text: { contains: [...]
} }`. A form node's fixture is `input: { form: <page>, node? }`. Every page's
`answers:` entry also runs: the page must open with its params and no loader
may fail.

Your coding agent has the same checks as MCP tools
(`oxplow_validate_extension`, `oxplow_preview_collector`), and the
`oxplow-extension` skill walks it through building one.
