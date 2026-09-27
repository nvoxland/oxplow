# Semantic layer

This doc covers the layer that **all oxplow data** flows through: the
sources that produce data, the dimensions that slice it, the metrics that
aggregate it, and the read-only SQL contract (`v_*`) that views, extensions
and agents query.

> **Status: partly built (epic tsk275).**
> - **Current:** the `v_*` read contract for core data, plus `query_sql` and
>   `describe_schema` over IPC and MCP (tsk282), and the **fact substrate**
>   (`measure`, `dimension`, `metric_spec`, `metric_capture`, `fact`, cube),
>   documented in [metrics.md](./metrics.md).
> - **Target:** everything else in this doc (sources that emit entities,
>   expression/join dimensions, entity-level metrics, extension sources, the
>   remaining shipped sources), tracked in tsk277.
>
> When a piece ships, move it from "target" to "current" here, in the same
> commit.

## Why

Oxplow's UI grew one bespoke page per question: metrics, dashboards,
change analysis, usage. Each page has its own IPC calls and its own opinion
of what matters. The direction ([architecture.md](./architecture.md) →
"Skeleton + semantic layer") is to stop hand-building answers and instead:

- ship **the data** as a stable, queryable layer;
- ship **common sources** of that data;
- let users and their agents build the **lenses** they actually want
  ([extensions.md](./extensions.md)).

A lens can only be as good as the data it can reach. So **nothing may be
reachable only through a bespoke IPC**. If core has it, the semantic layer
exposes it.

## The three pluggable parts

### 1. Sources produce data

A **source** emits:

- **entities**: typed rows with a key and relations (a task, a commit, an
  LSP diagnostic, a GitHub PR); and/or
- **facts**: atomic measurements on the existing fact substrate (a
  function's complexity, a test case's outcome, a token count).

Core ships built-in sources. Extensions declare more with the same format.
A source runs on a `schedule`: `every <dur>`, `on-snapshot`,
`on-effort-complete` or `manual`. The existing `CollectorRegistry` runtimes
(`BuiltinRust`, `Jaq`, `Starlark`, `Exec`, in `crates/oxplow-collect-plugin`)
become the source runtimes.

### 2. Dimensions slice data

A **dimension** is a SQL expression or a join over an entity. Examples:
`task.priority`, `file → package`, `file → owner via CODEOWNERS`,
`pr.author`, `issue.cycle`. A dimension can be declared anywhere (core,
project config, any extension). It applies to every metric whose entity has
the columns it needs. So an extension can add a dimension over data that a
different source owns.

Today dimensions are ad hoc keys in `fact.dims_json`, resolved in
`metric_engine.rs::dim_value_cached`. That model survives for facts;
expression/join dimensions are added for entities.

### 3. Metrics aggregate data

A **metric** is an aggregation over an entity **or** a fact set, sliced by
dimensions:

- `count(v_task) where status = 'done'`
- `median(v_github_pr.merged_at - opened_at)`
- `avg(fact oxplow.coverage)`

This generalizes today's `metric_spec`, which aggregates facts only.

## Shipped sources (core)

| Source | Entities | Facts |
|---|---|---|
| work | stream, thread, task, effort, decision, claim | cycle time, steering, lifecycle |
| knowledge | wiki_page, comment, correction | freshness |
| git | commit, branch, file_change, cochange, blame_line | churn |
| snapshots | snapshot, change_file, test_change | — |
| lsp | diagnostic, symbol, reference | diagnostic counts by severity |
| tests & coverage | test_run, test_case | coverage, pass/fail |
| code metrics | function, file | complexity, length, params |
| agent | session, turn, tool_call, hook_event, context_read | tokens, struggle |
| ai | ai_call | cost, latency |
| usage | page_visit | — |

New primitives that don't exist yet:

- `decision`: the forks an agent resolved during an effort (fork, choice,
  alternatives, confidence, why). Written via a `record_decision` MCP tool.
- `claim`: statements like "tests pass", joined to observations so each one
  reads as verified or unverified. Written via `record_claim`.
- `context_read`: which `.context/*.md` docs the agent read before touching
  a subsystem.
- `struggle`: retries, repeated reads of the same file, reverted edits.

These exist because what a human reviewing agent work needs most is
**exceptions and decisions**, not trends and totals.

## The `v_*` contract (current)

Every shipped entity is exposed as a stable **read-only SQL view**. They
are the **versioned contract**: lenses, extensions and agents read these,
never the physical tables, which stay internal and free to change.

**Shipped today** (migrations `V73__semantic_layer_views.sql`, `V74__semantic_layer_activity_views.sql`):

| View | What it is |
|---|---|
| `v_stream` | streams (worktrees) |
| `v_thread` | threads within a stream |
| `v_task` | tasks, excluding deleted; carries the thread's `stream_id` |
| `v_effort` | in_progress → done spans of work on a task |
| `v_comment` | comment threads, with first-message `body` and `message_count` |
| `v_wiki_page` | wiki pages (excerpt; full body is on disk) |
| `v_snapshot` | worktree snapshots |
| `v_measure` | fact-type catalog |
| `v_capture` | the scan/run that produced facts |
| `v_fact` | atomic measurements, joined to `measure_key` and capture context |
| `v_effort_file` | files each effort touched, with change kind (V74) |
| `v_task_note` | task / thread notes (V74) |
| `v_task_link` | typed links between tasks (V74) |
| `v_task_event` | task history log (V74) |
| `v_agent_turn` | human prompt → agent answer, per thread / task (V74) |
| `v_token_usage` | model tokens per thread / effort / model (V74) |
| `v_page_visit` | pages the human opened, and for how long (V74) |

Still target: `v_commit`, `v_branch`, `v_diagnostic`, `v_test_run`,
`v_decision`, `v_claim` and the rest of the shipped-sources table above.

**Column docs live in code, not here.** `CATALOG` in
`crates/oxplow-db/src/semantic_layer.rs` documents every column, and
`describe_schema` serves it. The test `schema_docs_match_the_views_exactly`
fails if a view's columns and its docs disagree, in name or in order. So
changing a view means:

1. a new migration that drops and recreates it;
2. updating its `CATALOG` entry;
3. noting the change here if it breaks readers (removed or renamed
   columns).

An extension entity `<entity>` owned by extension `<ext>` will be exposed
as `v_<ext>_<entity>` (target).

### Querying (current)

- `query_sql(sql, params?, limit?)` over IPC (`querySql` in `api.ts`) and
  MCP. Mechanics (`SemanticLayer` in `crates/oxplow-db/src/semantic_layer.rs`):
  - First gate: the statement must start with `SELECT` or `WITH`, after
    comments.
  - Real gate: rusqlite `prepare` rejects multiple statements, and
    `Statement::readonly()` rejects anything that writes (including
    `WITH … DELETE`).
  - Runs on a pooled connection under `PRAGMA query_only = ON`, which is
    always reset afterwards. A test proves the pooled connection stays
    writable.
  - A 5 s interrupt timer (`InterruptHandle`) stops runaway queries.
  - The row cap defaults to 500 and can be raised to at most 10 000.
    `truncated: true` means more rows existed.
  - Caller mistakes (bad SQL, writes, timeouts) are `Invalid` errors: the
    IPC `INVALID` code, and MCP `invalid_params`.
  - Physical tables are technically readable too, but they aren't part of
    the contract. Lenses and extensions must use `v_*`; a missing column is
    a reason to extend a view, not to reach around it.
- Values are `SqlCell`: an untagged `null | boolean | number | string`,
  so the TS binding is a plain scalar union. Blobs come back as
  `"<blob N bytes>"`.
- `describe_schema` returns each entity's name, description, owner
  (`core` or an extension) and columns, with docs and SQL types taken from
  the live schema.
- Lenses never call models or run sources on render; they read what sources
  already produced.
- Everything an extension adds (entities, relations, dimensions, metrics)
  will appear in the same `describe_schema` / `query_sql` /
  `list_dimensions` / `list_metrics` tools as core data. Agents never need
  an extension-specific tool. Full agent surface:
  [extensions.md](./extensions.md) → "Agents: the MCP surface".

## User and extension sources

Example: a GitHub source that brings in PRs and joins them to core data.

```yaml
sources:
  - id: github
    runtime: exec            # or starlark / jaq over a payload the host fetches
    entry: bin/github-sync
    schedule: every 10m
    credentials: [github]    # keychain entry names; injected as env vars
    network: [api.github.com]
    entities:
      - name: pr
        key: number
        columns: {title: text, state: text, author: text, head_branch: text,
                  opened_at: time, merged_at: time}
        relations:
          - {to: branch, on: head_branch}
          - {to: task, via: "title ~ 'tsk\\d+'"}
dimensions:
  - {name: pr.author, entity: pr, expr: author}
metrics:
  - {key: gh.time_to_merge, entity: pr, agg: median,
     expr: "merged_at - opened_at", where: "state = 'merged'"}
```

Rules:

- **Storage.** Extension entities live in a **per-extension attached SQLite
  DB** (`.oxplow/ext/<ext>.sqlite`), never in core tables. An extension
  can't corrupt core data, and disabling it drops its data cleanly. Oxplow
  creates the tables from the declared schema. The source protocol is
  upsert-by-key plus tombstones.
- **Relations.** Declared joins are what make this a layer and not a pile
  of tables. They let one query cross sources, e.g. "PRs whose linked task
  has an effort with unverified claims".
- **Containment.** An `exec` source can reach only the hosts it lists in
  `network` and only the keychain entries it lists in `credentials`.
  Credentials are injected as environment variables and never written under
  the project dir. `starlark` and `jaq` sources stay pure: they transform a
  payload the host fetched. Budgets and timeouts reuse `SandboxBudget`.
- **AI.** A source may call AI roles ([ai-providers.md](./ai-providers.md)),
  for example to classify each new PR. Results are stored as columns or
  facts, never recomputed on read.
- **Visibility.** A Settings → Data page lists every source with its owner
  (core or extension), last sync, row counts and errors, plus "sync now".

## Relation to other docs

- [metrics.md](./metrics.md): the fact substrate this generalizes. Still
  authoritative for facts, captures, the cube and gauges.
- [extensions.md](./extensions.md): how lenses and extensions consume this
  layer.
- [ai-providers.md](./ai-providers.md): the AI functions sources and lenses
  can use.
- [collection.md](./collection.md): test-run and coverage ingest, which
  becomes the tests & coverage source.
