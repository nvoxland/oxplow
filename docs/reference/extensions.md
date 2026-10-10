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
  and may use stable kinds only. A private one may declare a version too.
- The folder name is lowercase letters, digits and single dashes,
  starting with a letter.
- A metric, measure, dimension or fact collector's key is shared by every
  extension in the project: when two declare the same one, the first
  (alphabetically) keeps it and the other gets a load error naming
  both.

### What it can declare

<!-- kinds:begin (generated from oxplow_domain::contribution::KINDS; bless with OXPLOW_BLESS=1) -->
| Key | What it adds | Stability | More |
|---|---|---|---|
| `models` | SQL views over oxplow's models, published as `v_<ext>_<name>` | stable |  |
| `measures` | what a metric's facts measure | stable | [Metrics](../guide/metrics.md) |
| `metrics` | a number tracked over a measure | stable | [Metrics](../guide/metrics.md) |
| `dimensions` | a way to slice metrics | stable | [Metrics](../guide/metrics.md) |
| `collectors` | outside data brought in as entities, or facts for metrics | stable | below |
| `event_types` | its own namespace's event types | stable |  |
| `ref_kinds` | kinds of thing a ref can name | stable |  |
| `commands` | commands on oxplow's command bus | stable | below |
| `effects` | scripts that react to logged events | stable | below |
| `advisories` | guidance queries for the coding agent | stable |  |
| `skills` | skills and slash commands for the coding agent | stable |  |
| `implementations` | an implementation of a capability: a built-in, or an AI provider or effort policy written as a script | stable | below |
| `providers` | an external program implementing a capability, such as the work list | stable | [Provider protocol](provider-protocol.md) |
| `lenses/*.yaml` (files, not a key) | a page or panel over a SQL query | stable | [Lenses](../guide/lenses.md) |
| `pages` | where its lenses show as pages | stable |  |
| `panels` | where its lenses show as panels | stable |  |
| `ui.slots` | a lens mounted into a core page | stable |  |
| `ui.decorators` | labels on core refs | stable |  |
| `ui.replacements` | a lens in place of a core component (private extensions only) | experimental |  |
| `custom_components` | sandboxed HTML components for `viz: custom` lenses | stable |  |
<!-- kinds:end -->

`ui.replacements` (a lens in place of a core component) is
experimental: a private extension only.

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
environment or credentials. Every Starlark and jaq script — collectors,
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
  page to gather the input instead. `open_after` opens a ref once it
  ran, taking `{{result.<field>}}` from its result.
- A mistake in `ui` is a load error naming the field, not a menu entry
  that never shows.

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
one line: `file:line: what — fix`. `--impact` adds what your working
tree's version changes against git `HEAD`.

`oxplow extension test` runs on a throwaway oxplow: the check, then
each intent example's fixture, then each declared provider's
handshake, examples and conformance suite, and each policy script's
conformance suite. `--bless` writes a
provider's golden transcript.

Your coding agent has the same checks as MCP tools
(`validate_extension`, `run_collector`, `preview_collector`), and the
`oxplow-extension` skill walks it through building one.
