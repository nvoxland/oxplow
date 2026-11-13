# Knowledge

Pages of durable understanding — notes, walkthroughs, diagrams, what an
agent keeps for itself — kept in one or more **stores** at once. Core has
no knowledge capability and no table of pages: a page is a row of the shared
model `oxplow/knowledge_record` (`models.md`), kept by a store (an
extension's fill), and oxplow observes every store the same way, so the UI,
metrics and tracking are common to all of them. The store keeps the page;
the row, its links (`page_ref`), its pins and the `oxplow.knowledge_record.changed`
events are derived from what it kept.

## The model and its resource

- **`oxplow/knowledge_record`** is declared by oxplow-bundled (`models:`):
  columns `id`, `title`, `body` (markdown, `pins: true`: its file and
  directory refs are pinned to the primary stream's latest snapshot), `kind`,
  `url`, `author`, `updated_at`. A store fills it with `fills:` (its `stores:`
  ids, an `id_pattern` for a bare `[[id]]` in text, and `files:` when it
  keeps a row as a file).
- **The `oxplow/knowledge` resource** is bundled's too (as the task resource
  is): a page's URI is `oxplow:oxplow/knowledge?id=<id>&store=<store>`
  (`uri::build::knowledge_uri`). `resolve: knowledge_page`, `searchable:
  search_knowledge`. With bundled off the resource is undeclared and a `[[link]]`
  to a page is unrecognized, as a task's is.
- **Wikilinks.** `[[<store>:<id>]]` names a page of a knowledge store; a bare
  `[[id]]` names the first store whose fill's `id_pattern` matches it whole
  (`ResourceRegistry::with_knowledge_store`, fed from the model registry's
  fills by `vocabulary_reactor`, as `tsk42` is for tasks).

## Reading

All through bundled's datasets (SQL, `oxplow_query_sql`) and `oxplow_read`:

- **`v_oxplow_knowledge_page`** — one row per page of every store: `uri`,
  `store`, `id`, `kind`, `title`, `url`, `author`, `excerpt`, `body`,
  `body_size`, `outbound_uris` (JSON array of the URIs it links),
  `stale_ref_count`, `read_count` (agents' and people's), `agent_read_count`,
  `last_read_at`, `updated_at`. It is what the resource resolves through:
  `oxplow_read` answers its row (the `body` column is the content) and the
  ref resolver reads it by `resolve_view`.
- **`v_oxplow_search_knowledge`** — what site search indexes (title, body).
- **`v_oxplow_knowledge_ref`** — `v_ref_pin` of the pages: the files and
  directories each page pins and whether each changed since (`stale`).
- **`v_oxplow_knowledge_read`** — every read: an agent's (the model
  `oxplow/knowledge_read_record`, written by bundled's collector
  `oxplow.knowledge_read` from `record.read`, via `tool` or the page's `file`)
  and a person's (`page_visit`, core's).
- **`v_oxplow_knowledge_touch`** — which thread last wrote which page (the
  model `oxplow/knowledge_touch_record`, written by `oxplow.knowledge_touch`
  from the change event's thread anchor, so a person's edit of a file touches
  nothing). The Work panel's Finished list reads it.
- `v_model_store` (`model = 'oxplow/knowledge_record'`) lists the stores;
  `v_page_ref` has the backlinks.

A bundled dataset that reads a view that reads views, in a compound select,
trips the lineage check (`reads … without ref()`): read a model view directly
or use CTEs.

Agents read through bundled's `knowledge` skill (find before you create; read
with `oxplow_read`; the link rules). The metrics (`knowledge.pages`,
`knowledge.stale_pages`, `knowledge.read_requests`) are bundled's collectors
over these datasets, sliced by `oxplow.knowledge_store` and
`oxplow.knowledge_kind`.

## Writing

A store writes its own pages; core has no write command for them. A store is
an extension that fills the model: its commands write its pages, a collector
states the pages it holds (a snapshot or upserts), and `models.md` says what
core does with the rows (events, edges, pins, search). `links.check` (a
service) lets a store refuse a body whose `[[links]]` don't resolve. Bundled
declares two action points a store's commands join: `oxplow/knowledge.new`
(title?, body?) and `oxplow/knowledge.delete` (uri). The knowledge page's text (its `body` node)
is the replaceable target `knowledge.detail.body`, bound to the page's own
store (the URI's `store`): with no replacement it is oxplow's read-only
markdown, and the extension that fills the model as that store may draw an
editor in its place (`ui.mounts: { page: …, at: knowledge.detail.body }`; the
page takes `uri`). oxwiki does, so an oxwiki page shows one body, editable
with Save on `oxwiki.page.write`, and another store's page stays read-only.
The page keeps its `sections` slot (`oxplow/knowledge-page.sections`) for any
extension's extra part.

`oxplow.refs.verify { uri, files? }` re-pins a page's cited files to now (any
model row's pinned refs); `oxplow_ref_drift { uri, path }` shows one pinned
ref's diff against its pin.

### The write guard

A tool's write of a row file is refused where it would be lost: a thread in a
worktree may not write `<its worktree>/<folder>/<id><suffix>` of any fill's
`files:` (only the primary checkout's folder is watched; `.oxplow` is
git-ignored). The refusal names the store's extension, whose command writes to
the primary checkout from anywhere. In the primary checkout a direct write is
allowed: the store's collector converges it. `write_guard::record_file_reason`,
from the running generation's fills; `agent-model.md` "Isolation".

## oxwiki

oxwiki is an extension (`extensions/oxwiki`, on by default, requires
oxplow-bundled), oxplow's own knowledge store: one markdown file per page,
`.oxplow/oxwiki/<slug>.md`, filling `oxplow/knowledge_record` as the store
`oxwiki`.

- **Files.** `files: [{ path: .oxplow/oxwiki, access: write, watch: true }]`:
  the folder is watched and snapshotted though git ignores it
  (`extensions-permissions.md` "Declared folders"), and the fill's `files: {
  folder, suffix: .md }` makes an agent's read of a page's file a read of the
  row (`record.read`).
- **Commands.** `oxwiki.page.write { id?, title?, body }` and
  `oxwiki.page.delete { id }` write and delete `.oxplow/oxwiki/<slug>.md`
  through `oxplow.extension_files.write`/`delete`; the files service resolves
  against the project root, so a write from any worktree lands in the primary
  checkout's folder. Slugs are kebab-case; a title makes one; a title naming an
  existing page is refused. `oxwiki.page.restore` is their undo. A write joins
  `oxplow/knowledge.new`, a delete `oxplow/knowledge.delete`.
- **The collector** `pages` (`on: [start, { files: ".oxplow/oxwiki/*.md" }]`)
  states every page file as a snapshot of the store's rows (`id`, `title` from
  the first heading outside a code fence, `body`, `kind: note`, `updated_at`
  from the file's mtime), so a hand edit, a new file and a deleted one
  converge. Only files directly in the folder are pages; one bigger than 1 MiB
  or not UTF-8 is skipped (named in the run's `problems`).
- **Editing** in the app: the `edit-page` part, mounted on the knowledge
  page's `sections` slot.
- **Skills.** `oxwiki` (writing) and the `/note` command, gated on the model.
- **Off.** Turn it off with its extension; there is no setting of its own.
  Its pages are hidden and its files left alone; on again, the collector
  rescans.
- Fixtures: `extensions/oxwiki/fixtures`.

## A provider or script store

A store may be any extension that fills the model — a script collector reading
a vault, a docs server. It states its rows through a collector's `models:`
output (`models.md`); the shared screens and the datasets above work for it
unchanged.

## Pages

The screens are `oxplow-bundled`'s (`.context/extensions-bundled.md` "The
knowledge pages"):

- **Knowledge** (`knowledge`): the stores, the pages, the stale ones and
  recent reads and writes; New page and Delete (both action points).
- **Knowledge Page** (`for: oxplow/knowledge`): one page by its URI; the body,
  its store, kind, freshness and cited files, Mark verified
  (`oxplow.refs.verify`), the Edit part a store mounts.
- **Knowledge Freshness** (`store`, `id`): the files a page cites.

Never a slug: a page is its `store` and `id`.
