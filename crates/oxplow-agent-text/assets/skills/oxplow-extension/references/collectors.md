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
  typed columns and key, and when it runs: `trigger: { every: 15m }` or
  `trigger: manual` (the default).
- `examples/extensions/github/` in the oxplow repo is a complete, working
  example; copy its shape.
- The entity becomes `v_<extension>_<entity>`. Join it to core views in
  lenses.

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
- **A collector that runs a program needs `permissions.process`** (and the
  `network` hosts and `secrets` it uses), which a person approves when the
  extension is deployed; you can't approve. Once it is deployed,
  `run_command` `oxplow.collector.sync { collector: "<extension>/<id>" }`
  re-runs it.
- `v_collector_run` shows each collector's last run and error, and
  `v_collector_state` whether this OS enforces its `network` list
  (`network_enforced`) and why failures disabled it, if they did.
- **Check a collector before you deploy it** with
  `oxplow_preview_collector(owner, id, stream_id)`: it runs your worktree's
  version and returns the rows it would store, storing nothing. For a
  fact collector (`facts:`) it returns the facts it would record
  (`{ owner, id, facts }`), computed over the stream's latest snapshot.
  `oxplow.collector.sync` always runs the primary's copy (collected data is
  shared by the whole project), so in a worktree stream it won't see your
  changes. An exec collector's preview needs the same `permissions.process`.
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
  every run, so the extension declares `permissions.ai` and the user approves
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
  over the `v_*` models.
- `tree.read`: `files()`, `source_files()`, `duplicate_blocks()`, the
  snapshot's files. A fact collector has it without declaring it.
- `ai.compute`: the `ai_*` builtins. The extension declares `permissions.ai`. An
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
- `oxplow extension check <name> --services` lists each script's
  services and whether it declared them.
- `examples/extensions/triage/` in the oxplow repo is a fact collector
  that uses all three.

**Incremental sync.** For a big or slow upstream, set `sync: upsert`:

- **What a run returns.** Only new and changed rows, plus
  `"deleted": {"<name>": [key, …]}` for removed ones.
- **Unmentioned entities.** One the run doesn't return is left as it was.
- **Default.** `sync: replace` makes each run restate everything.
