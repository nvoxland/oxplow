# Collectors: outside data, derived data, host services

## Bringing in outside data (collectors)

When the user wants data oxplow doesn't have (GitHub PRs, a team's issues,
CI runs), add a **collector** to the extension:

- Write a script that prints
  `{"entities": {"<name>": [ {col: value, …} ]}}`. It may add
  `"events": [{"type": "<your_ns>.<name>", "payload": {…}}]` — only event
  types your extension declares (`event_types:`), at most 100 a run, and
  never a type the collector itself runs on. Use it to say what the
  collector saw; put what should *happen* in an effect that reacts to it.
- Declare it under `collectors:` in `extension.yaml`, with its entities'
  typed columns and key, and when it runs: `on: { every: 15m }` or
  `on: manual` (the default).
- `$OXPLOW_BIN extension new --from github` copies a complete, working
  collector; change it to fit.
- The entity becomes `v_<namespace>_<entity>` (the manifest's `namespace:`, default the extension's name). Join it to core views in
  pages.

Rules:

- Declare secrets as `credentials: [NAME]`; the script reads them as
  environment variables. The user sets the values in Settings → Extensions
  (they go to the keychain). You can't set or read them, and never
  hard-code them. `v_collector_credential` (`collector`, `name`, `is_set`) shows which
  are set, never the value. Use `env: [NAME]`
  only for non-secret settings from the user's environment.
- Declare every host the script talks to: `network: [api.github.com]`
  (`*.example.com` for subdomains). On macOS nothing else is reachable
  (the script goes through a proxy; `curl`, `gh` and most HTTP clients
  pick it up from `HTTPS_PROXY`). A missing host shows up as a `403` from
  the proxy or a connection failure.
- **A collector that runs a program needs `process.run`** (and the
  `network` hosts and `secrets` it uses), which a person approves when the
  extension is deployed; you can't approve. Once it is deployed,
  `run_command` `oxplow.collector.sync { collector: "<namespace>/<id>" }`
  re-runs it.
- `v_collector` lists enabled extensions' collectors with their trigger,
  `enabled`, `state` (`disabled`, `failing`, `never_run`, `ok`),
  `last_run_at` and `last_outcome`.
- `v_collector_run` lists runs with their outcome, error and usage. Filter
  by `owner` (the namespace) and `id` for a collector's history. Use
  `latest = 1` for its newest finished run.
- `v_collector_checkpoint` has the last successful cursor and row counts
  plus the last trigger sequence and finished run ID. `v_collector_state`
  shows whether this OS enforces the collector's `network` list
  (`network_enforced`) and why failures disabled it.
- **Test a collector before you deploy it** with `$OXPLOW_BIN extension test
  <name>` from your terminal: it runs your source (a worktree's draft
  included) in a throwaway oxplow,
  through the examples you wrote for it. Your source's code runs nowhere
  else until it is deployed from the main stream.
- `oxplow_preview_collector(owner, id, stream_id)` dry-runs the **deployed**
  entity collector against a stream's files and data and returns the rows
  it would store, storing nothing. A fact collector (`facts:`, Starlark or
  jaq: it only computes) runs that stream's own script over its latest
  snapshot and returns the facts it would record (`{ owner, id, facts }`). `oxplow.collector.sync` runs it for real
  (collected data is shared by the whole project). An exec collector's
  preview needs the same `process.run`.
- A declared entity's view exists once its collector first runs.

**Deriving data you already have (starlark / jaq collectors).** When the
entity can be computed from views that already exist (reshaping tasks,
combining two extensions' data), use `runtime: starlark` (a script
defining `def transform(input): ...`) or `runtime: jaq`, with `input:` set
to a read-only SQL query:

- **Input and output.** The script defines `def transform(x):` at the top
  level (`check` says so at the collector's line otherwise); it gets
  `{"rows": [...]}` and returns the same `{"entities": ...}` shape. It
  must handle **no rows** — return an empty list, don't index `rows[0]`
  — because a fresh project, a `extension test` throwaway and a filter that
  matches nothing all give it none; with `sync: replace` an entity it
  leaves out is emptied.
- **Permissions.** It runs sandboxed (no network, files, env or credentials),
  so it needs none and you can run `oxplow.collector.sync` on it yourself —
  unless it calls a model (`ai_*`, "Asking a model" below): that spends the user's key on
  every run, so the extension declares `ai.compute` and the user approves
  it when it is deployed. Without it the loader refuses the extension.
- **Input limits.** `input` can't read the collector's own views, and more
  than 10,000 input rows fails the run.
- **Asking a model.** A starlark collector can call `ai_classify(text,
  labels)` → `{label, probabilities}`, `ai_score(text, levels)` →
  `{level, score, probabilities}` (levels lowest first),
  `ai_summarize(text, focus = None)` → text, and `ai_extract(text,
  schema, instructions = "")` → JSON matching `schema`. Each is recorded
  (`v_ai_result`): the same question on the same text is one model call,
  ever, so a re-sync costs nothing for rows that didn't change. They run
  on the project's AI roles (`decide`, `summarize`, `main`; `v_ai_role` shows
  the model each would use now (`effective_provider`, `effective_model`) and
  whether its provider has a key (`key_set`, never the key), `v_ai_provider` the
  providers), or on one of
  your `ai_profiles` with `role = "<id>"`; with no model assigned the run
  fails saying which role. A fact collector calls them
  when it declares `needs: [ai.compute]`; a command's, an effect's or a
  policy's script never can ("Host services" below in this file). For example, what
  kind of work each turn was:

  ```yaml
  collectors:
    - id: turn_kind
      runtime: starlark
      entry: turn_kind.star
      input: "SELECT id, prompt FROM v_agent_turn WHERE prompt <> ''"
      entities:
        - { name: turn_kind, key: id, columns: { id: int, kind: text } }
  ```

  ```python
  KINDS = ["feature", "bug fix", "refactor", "question", "chore"]

  def transform(input):
      return {"entities": {"turn_kind": [
          {"id": r["id"], "kind": ai_classify(r["prompt"], KINDS)["label"]}
          for r in input["rows"]
      ]}}
  ```

**Host services.** A Starlark script reaches oxplow only through the
services it lists in `needs:` (`v_scope` lists every scope with its access
and summary):

- `sql.read`: `scope("sql.read", {sql, params?, page?})`, read-only SQL
  over the `v_*` datasets.
- `tree.read`: `files()`, `source_files()`, `duplicate_blocks()`, the
  snapshot's files; `change_diff(change_id, paths?)`, an analysed change's
  hunks with their added and removed lines (at most 200 files and 2 MB a call). A fact collector has it without declaring it.
- `ai.compute`: the `ai_*` builtins. The extension declares `ai.compute`. An
  entity collector that calls one has it without declaring it.
- `ai.discover`: `scope("ai.discover", {role})` says whether a model
  serves a role (a built-in or your extension's `ai_profiles` id) —
  `{role, model: {provider, model} | null, problem}` — without spending.
  An effect uses it to skip, with a reason, when no model is bound.

Rules:

- A script that uses a service it doesn't declare doesn't load. `check`
  names the builtin and the `needs` entry that fixes it.
- A command's, an effect's or a policy's script composes, so it never
  asks a model. A command's script can't read the tree, because oxplow
  composes it inside a transaction.
- `$OXPLOW_BIN extension check <name> --services` lists each script's
  services and whether it declared them.
- `$OXPLOW_BIN extension new --from triage` copies a fact collector that
  uses all three.

**Incremental sync.** For a big or slow upstream, set `sync: upsert`:

- **What a run returns.** Only new and changed rows, plus
  `"deleted": {"<name>": [key, …]}` for removed ones.
- **Unmentioned entities.** One the run doesn't return is left as it was.
- **Default.** `sync: replace` makes each run restate everything.

## A review check

A **review check** is an entity collector that runs when a change's analysis
is stored and adds rows to the change review's list. It is how a check too
costly or too textual for plain SQL over the stored analysis (a search of the
tree, a parse) joins the review.

```yaml
collectors:
  - id: acme.review.todo-left
    doc: "TODO markers a change added."
    runtime: starlark
    entry: collectors/review/todo_left.star
    on: [change.analyzed]
    where: { phase: files }          # the analysis, not its duplicate scan
    needs: [tree.read, sql.read]
    input: "SELECT :stream_id AS stream_id"
    sync: upsert
    entities:
      - name: review_todo
        key: id
        columns: { id: text, change_id: int, path: text, line: int, signal: text, title: text, why: text, uri: text }
datasets:
  - name: review_signal_todo
    adds_to: oxplow/review_signal    # the open list; fields by name, untyped in the list
```

- The script's tree (`files()`, `grep_tree()`) is the change's head, whatever the worktree
  holds now: the event's `anchors.revision`. `language_of(path)` names a file's language
  for `code_metrics`.
- The script gets `input["event"]["payload"]["change_id"]`; `change_diff(change_id)`
  gives the hunks with their added and removed lines, `grep_tree(pattern)` the
  tree, `scope("sql.read", …)` the semantic layer (`v_symbol` of the stream in
  `input["rows"][0]["stream_id"]`).
- Key the rows by change, so one rerun replaces the change's rows: read its
  earlier keys with `scope("sql.read", { "sql": "SELECT id FROM v_<ns>_<entity>
  WHERE change_id = :c", "params": … })` and return the ones it no longer
  produces in `"deleted": { "<entity>": [key, …] }`.
- The dataset (`datasets/<name>.sql`) selects the open list's fields from
  `ref('<entity>')`: `change_id, source, area, signal, severity, path, line,
  end_line, title, why, uri` (one you lack is NULL). `severity` is `high`,
  `medium` or `info`.
- A check runs after the analysis, so a Review page reads
  `v_oxplow_review_check` (per change and check: `pending`, `failed` or `ok`) to
  say a check is still running.
- The other ways to shape the review (a signal from SQL, a lint rule, ranking, a section) are in `references/recipes.md` "Customize the change review".
- Test it with a fixture: commits with real files (`- { files: { "<path>": "…" } }`,
  `oxplow.vcs.stage` and `.commit`), `observe: analyze`, `observe: symbols`, a
  `observe: snapshot` with each file's `body` (the tree the run searches),
  then `observe: event` `change.analyzed` and a `sql` read of the dataset.
