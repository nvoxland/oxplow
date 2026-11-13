# Scaffolding an extension and the words it uses

A **page** is named queries plus a tree that shows them. The user opens it
(Cmd+P), and it updates as the work changes. Pages live in **extensions**:
folders of plain files in `.oxplow/extensions/<name>/`. A source runs only
after `oxplow.extension.deploy { name }`. The user can use a page
themselves or share the extension with their team (`oxplow.extension.share`).

You build them with your normal file tools. `$OXPLOW_BIN extension new <kind>
<name> --origin <your request ref>` scaffolds the folder with a v2 manifest,
an intent, one example and its fixture, and a working starter for the
kind; then edit. `$OXPLOW_BIN extension new --list-examples` names the
working examples oxplow ships, and `new --from <example> [<name>]` copies one (with a
`<name>` it rewrites the example's own name everywhere to it). Each passes `check` and `extension test` as written, so a
failure after your edit is yours. `$OXPLOW_BIN extension add <kind>
<id> [<extension>]` adds one more of the `page`, `collector`, `command`,
`effect`, `component`, `advisory`, `context` or `skill` starters below to an extension
that exists (its files and its manifest entry, comments kept; an id or file
that exists is refused):
- `page` — a page over open tasks with a row action (Start);
- `query` — `queries/<name>.yaml`, rows for a choice's `options_from` (`value`, `label`); no view;
- `collector` — a Starlark collector of an entity, a dataset over it
  (`ref('item')`) and a page over the dataset;
- `metric` — a code metric: a per-path measure, a Starlark collector
  counting each file's functions over a complexity limit, the dimension
  its facts carry, the headline metric and a page linking to the file and
  line (`--language`, `--glob`; the `oxplow-metrics` skill teaches it);
- `command` — a command whose script comments on a task, on a task's Commands menu;
- `provider` — an external provider's declarations and a stub program
  (red until you write the program); `--capability <cap>` picks what it
  implements: `snapshots` (the default),
  `knowledge`, `filesystem` or `agent_harness` (a harness launching agent sessions: its command runs in
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
  `knowledge.page.recorded@2` event `{ page: { uri: "oxplow:oxplow/knowledge?id=<slug>&store=oxwiki", title,
  body, uris, updated_at, deleted? } }` (`uris`: the body's links as `oxplow:oxplow/file?path=…` /
  `oxplow:oxplow/knowledge?id=…&store=…`), and a `knowledge_pages` collector of the same rows;
- `implementation` — a capability implemented by a script instead of a
  program: an `implementations:` entry whose `entry` is a Starlark file
  (`implementations/<id>.star`) with one function per verb, for the same
  `--capability` choices as `provider` (its verbs are stubs that fail
  until you write them).
  Each function gets `x = { input, config, id, key }` and answers `{ result,
  events?, inverse? }`, as a provider's verb does; it runs in the
  sandbox, calling only the scopes in its `needs:` (granted in
  `permissions:`), and keeps what it needs between calls in what it can
  read back. The entry also says what a provider declares: `features:`
  (one left out is off), `data:`. Choose it in Settings → Capabilities
  (`oxplow.capability.choose`); `extension test` runs its capability's
  conformance suite;
- `effect` — a script reacting to a logged event by composing commands
  (private; runs under the extension's permissions);
- `job` — model work that runs on its own, outside any transaction: the
  starter has the job and its model profile, `job.ask|status|cancel`
  commands, a `note.ask` command that holds the prompt, a page over
  `v_job_run_output` that follows the reply live, and a `publish` effect on
  `contribution.job.completed` that comments the reply on a task. Its
  fixtures run the job on a fixture model (`model: { replies }`); a person
  can cancel it (`references/jobs-and-ai.md`);
- `component` — a custom component (private): a page with its component
  node and a web bundle on oxplow's client library;
- `advisory` — guidance for you: a query run at a turn's end whose rows
  reach the agent (`audience: person` raises them in Alerts instead); a
  an extension's advisories need `agent.write`;
- `context` — what you should know now: a query whose `text` (one row) reaches
  you on your first prompt, then only when it changes, and again after a
  compaction; at most 1,500 characters; it needs `agent.write`;
- `skill` — a skill (`SKILL.md` with `name:` and `description:`
  frontmatter) you load when it's relevant; `kind: command` makes it a
  slash command; `when: { config: /a/key, in: [value, …] }` offers it
  only while that key of the extension's own `configuration:` is one of
  the values (a scalar key; the values in its `enum` if it has one), and a
  settings change rewrites what's offered;
- `panel` — a part compact in the left nav, written to `parts/<name>.yaml` (a
  panel's `scope` is the param its part declares);
- `extension` — the manifest only.

Words used exactly: an **extension** is the folder; a **contribution** is
one thing it declares (a page, collector, command, effect, provider …); a
**capability** is a swappable slot a project chooses an implementation of
(`snapshots`, `agent_harness`); an **implementation** fills
one (a built-in, a script, or a provider's instance); a **provider** is
the process implementing a capability (an *external provider* speaks the
provider protocol). A **need** is what a contribution says it requires (a
capability, a feature, a scope); a **grant** is what the extension's
`permissions:` allow, approved when it was deployed. A need is never a
grant: declaring `needs: [sql.read]` doesn't give a program that scope
unless `permissions` has `sql: read`; the loader refuses the extension.
