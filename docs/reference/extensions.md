# Extensions

An extension is a folder under `oxplow/extensions/<name>/` with an
`extension.yaml`. Everything it adds is declared there as data:
lenses, models, collectors, commands, effects, providers and the rest.
Each of those is a *contribution*.

```sh
oxplow extension new extension my-review   # scaffold one that checks clean
oxplow extension check my-review           # every problem, as file:line
oxplow extension test my-review            # run its examples on a throwaway oxplow
```

`new` takes a kind instead of `extension` to start from that kind's
starter: `lens`, `collector`, `command`, `effect`, `provider` or
`component`. Each scaffold checks clean and passes `test` as written.

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
  oxplow it targets) and may use stable kinds only.

### What it can declare

| Key | What it adds | More |
|---|---|---|
| `lenses/*.yaml` (files, not a key) | a page or panel over a SQL query | [Lenses](../guide/lenses.md) |
| `models` | SQL views over oxplow's models, published as `v_<ext>_<name>` | |
| `measures`, `metrics`, `dimensions` | metric definitions, in `.oxplow/project.yaml`'s vocabulary | [Metrics](../guide/metrics.md) |
| `collectors` | outside data brought in as entities, or facts for metrics | below |
| `commands` | commands on oxplow's command bus | below |
| `effects` | scripts that react to logged events | below |
| `providers` | an external program implementing a capability, such as the work list | [Provider protocol](provider-protocol.md) |
| `event_types` | its own namespace's event types | |
| `ref_kinds` | kinds of thing a ref can name | |
| `pages`, `panels` | where its lenses show | |
| `ui` | lenses mounted into core pages (`slots`), labels on core refs (`decorators`) | |
| `advisories` | guidance queries for the coding agent | |
| `skills` | skills and slash commands for the coding agent | |
| `implementations` | built-in implementations of a capability it offers | |
| `custom_components` | sandboxed HTML components for `viz: custom` lenses | |

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

A Starlark or jaq collector runs sandboxed: no files, network,
environment or credentials. It needs no approval, with one exception:
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

## Checking and testing

`oxplow extension check` loads the extension the way oxplow does and
dry-runs its models, commands, lenses and advisories. Each finding is
one line: `file:line: what — fix`. `--impact` adds what your working
tree's version changes against git `HEAD`.

`oxplow extension test` runs on a throwaway oxplow: the check, then
each intent example's fixture, then each declared provider's
handshake, examples and conformance suite. `--bless` writes a
provider's golden transcript.

Your coding agent has the same checks as MCP tools
(`validate_extension`, `run_collector`, `preview_collector`), and the
`oxplow-extension` skill walks it through building one.
