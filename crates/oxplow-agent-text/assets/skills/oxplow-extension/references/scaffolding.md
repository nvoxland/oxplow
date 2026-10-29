# Scaffolding an extension and the words it uses

A **page** is named queries plus a tree that shows them. The user opens it
(Cmd+P), and it updates as the work changes. Pages live in **extensions**:
folders of plain files in `.oxplow/extensions/<name>/`. A source runs only
after `oxplow.extension.deploy { name }`. The user can use a page
themselves or share the extension with their team (`oxplow.extension.share`).

You build them with your normal file tools. `$OXPLOW_BIN extension new <kind>
<name> --origin <your effort ref>` scaffolds the folder with a v2 manifest,
an intent, one example and its fixture, and a working starter for the
kind; then edit. `$OXPLOW_BIN extension new --list-examples` names the
working examples oxplow ships, and `new --from <example>` copies one (it
keeps its name; rename after copying). Each passes `check` and `extension test` as written, so a
failure after your edit is yours:
- `page` — a page over open tasks with a row action (Start);
- `collector` — a Starlark collector of an entity, a dataset over it
  (`ref('item')`) and a page over the dataset;
- `metric` — a code metric: a per-path measure, a Starlark collector
  counting each file's functions over a complexity limit, the dimension
  its facts carry, the headline metric and a page linking to the file and
  line (`--language`, `--glob`; the `oxplow-metrics` skill teaches it);
- `command` — a command whose script comments on a work item, on a work
  item's Commands menu;
- `provider` — an external provider's declarations and a stub program
  (red until you write the program); `--capability <cap>` picks what it
  implements: `work_items` (the default), `effort_policy`,
  `snapshots`, `knowledge`, `filesystem` or `agent_harness` (a harness launching agent sessions: its command runs in
  the person's terminal, so the deploy that admits it (`process`) says that; if it can
  tell its subagents apart it declares the `subagents` feature and answers
  the `prompt` and `subagent` verbs).
  A `snapshots` provider answers `mark { stream, worktree,
  trigger, parent? }` → `{ handle, unchanged, file_count }`, `changed { stream,
  from, to }` → `{ changes: [{ path, kind: added|modified|deleted, identity
  (xxh3-128 hex), size }] }` (`from: null` is the empty tree) and, with the
  `contents` feature, `read_at { handle, path }` → `{ bytes }` (base64).
  A `filesystem` provider serves files under the id `<extension>:<instance id>`
  (a file URI's `filesystem` param) and answers `stat { path, rev? }` → `{ entry }`
  (null, or `{ kind: file|dir, size?, modified_at?, is_text? }`), `read { path, rev?,
  offset?, len? }` → `{ bytes }` (base64, null when absent), `list { rev?, parent? }` →
  `{ entries: [{ path, kind }] }` (no `parent`: the whole tree, `""`: the root) and, with
  the `writable` feature, `write { path, content }` (base64) → `{}`; `oxplow:` ids are
  core's.
  A `knowledge` provider answers `write_page { slug, title?,
  body, verified_files, removed_files }` → `{ page }`, `delete_page { slug }` and
  `link { page, target }`, each also answering the page's
  `knowledge.page.recorded@2` event `{ page: { uri: "oxplow:oxplow/wiki?slug=<slug>", title,
  body, uris, updated_at, deleted? } }` (`uris`: the body's links as `oxplow:oxplow/file?path=…` /
  `oxplow:oxplow/wiki?slug=…`), and a `knowledge_pages` collector of the same rows;
- `effect` — a script reacting to a logged event by composing commands
  (private; runs under the extension's permissions);
- `job` — model work that runs on its own, outside any transaction: the
  starter has the job and its model profile, `job.ask|status|cancel`
  commands, a `note.ask` command that holds the prompt, a page over
  `v_job_run_output` that follows the reply live, and a `publish` effect on
  `contribution.job.completed` that comments the reply on a work item. Its
  fixtures run the job on a fixture model (`model: { replies }`); a person
  can cancel it (`references/jobs-and-ai.md`);
- `component` — a custom component (private): a page with its component
  node and a web bundle on oxplow's client library;
- `policy` — an effort policy written as a script (`implementations:`,
  `needs: [sql.read]`) that opens an item's effort when it starts on a
  thread and closes it when it ends; it passes the effort-policy suite
  as written, and runs under the extension's permissions;
- `advisory` — guidance for you: a query run at a turn's end whose rows
  reach the agent (`audience: person` raises them in Alerts instead); a
  an extension's advisories need `agent.write`;
- `skill` — a skill (`SKILL.md` with `name:` and `description:`
  frontmatter) you load when it's relevant; `kind: command` makes it a
  slash command;
- `panel` — a page compact in the left nav (a panel's `scope` is the param
  its page declares);
- `extension` — the manifest only.

Words used exactly: an **extension** is the folder; a **contribution** is
one thing it declares (a page, collector, command, effect, provider …); a
**capability** is a swappable slot a project chooses an implementation of
(`work_items`, `effort_policy`, `snapshots`); an **implementation** fills
one (a built-in, a script, or a provider's instance); a **provider** is
the process implementing a capability (an *external provider* speaks the
provider protocol). A **need** is what a contribution says it requires (a
capability, a feature, a scope); a **grant** is what the extension's
`permissions:` allow, approved when it was deployed. A need is never a
grant: declaring `needs: [sql.read]` doesn't give a program that scope
unless `permissions` has `sql: read`; the loader refuses the extension.
