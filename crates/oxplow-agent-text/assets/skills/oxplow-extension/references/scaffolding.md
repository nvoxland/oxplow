# Scaffolding an extension and the words it uses

A **lens** is a saved query over oxplow's data plus how to show it. The user
opens it as a page (Cmd+P → Lenses), and it updates as the work changes.
Lenses live in **extensions**: folders of plain files in the repo. The user
can use a lens themselves, share it with their team by committing it, or
publish the extension for anyone.

You build them with your normal file tools. `oxplow extension new <kind>
<name> --origin <your effort ref>` scaffolds the folder with a v2 manifest,
an intent, one example and its fixture, and a working starter for the
kind; then edit. Each passes `check` and `extension test` as written, so a
failure after your edit is yours:
- `lens` — a lens over open tasks with a row action (Start);
- `collector` — a Starlark collector of an entity, a model over it
  (`ref('item')`) and a lens over the model;
- `metric` — a code metric: a per-path measure, a Starlark collector
  counting each file's functions over a complexity limit, the dimension
  its facts carry, the headline metric and a lens linking to the file and
  line (`--language`, `--glob`; the `oxplow-metrics` skill teaches it);
- `command` — a command whose script comments on a work item, on a work
  item's Commands menu;
- `provider` — an external provider's declarations and a stub program
  (red until you write the program); `--capability <cap>` picks what it
  implements: `work_items` (the default), `effort_policy`,
  `snapshots`, `knowledge` or `agent_harness` (a harness launching agent sessions: its command runs in
  the person's terminal, so a person approves it knowing that; if it can
  tell its subagents apart it declares the `subagents` feature and answers
  the `prompt` and `subagent` verbs).
  A `snapshots` provider answers `mark { stream, worktree,
  trigger, parent? }` → `{ handle, unchanged, file_count }`, `changed { stream,
  from, to }` → `{ changes: [{ path, kind: added|modified|deleted, identity
  (xxh3-128 hex), size }] }` (`from: null` is the empty tree) and, with the
  `contents` feature, `read_at { handle, path }` → `{ bytes }` (base64).
  A `knowledge` provider answers `write_page { slug, title?,
  body, verified_refs, removed_refs }` → `{ page }`, `delete_page { slug }` and
  `link { page, target }`, each also answering the page's
  `knowledge.page.recorded@1` event `{ page: { ref: "wiki:<slug>", title,
  body, refs, updated_at, deleted? } }` (`refs`: the body's links as `file:…` /
  `wiki:…`), and a `knowledge_pages` collector of the same rows;
- `effect` — a script reacting to a logged event by composing commands
  (private; runs only once a person approves it);
- `job` — model work that runs on its own, outside any transaction: its
  reply streams into `v_job_run_output`, its model may call tools you
  declare, a person can cancel it, and an effect on its
  `contribution.job.completed` writes what it found (runs once a person
  approves it; `references/jobs-and-ai.md`);
- `component` — a custom component (private): its `viz: custom` lens and
  a web bundle on oxplow's client library;
- `policy` — an effort policy written as a script (`implementations:`,
  `needs: [sql.read]`) that opens an item's effort when it starts on a
  thread and closes it when it ends; it passes the effort-policy suite
  as written, and runs once a person approves it;
- `advisory` — guidance for you: a query run at a turn's end whose rows
  reach the agent (`audience: person` raises them in Alerts instead); a
  shared extension's advisories run once a person approves them;
- `skill` — a skill (`SKILL.md` with `name:` and `description:`
  frontmatter) you load when it's relevant; `kind: command` makes it a
  slash command;
- `page` / `panel` — a lens as a full page in the launcher, or compact in
  the left nav (a panel's `scope` is the param its lens declares);
- `extension` — the manifest only.

Words used exactly: an **extension** is the folder; a **contribution** is
one thing it declares (a lens, collector, command, effect, provider …); a
**capability** is a swappable slot a project chooses an implementation of
(`work_items`, `effort_policy`, `snapshots`); an **implementation** fills
one (a built-in, a script, or a provider's instance); a **provider** is
the process implementing a capability (an *external provider* speaks the
provider protocol). A **need** is what a contribution says it requires (a
capability, a feature, a scope); a **grant** is what a person approved. A
need is never a grant: declaring `needs: [sql.read]` doesn't give a
program that scope; it runs only once a person approves it.
