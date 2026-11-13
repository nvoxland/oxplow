# Knowledge

Pages of durable understanding — notes, walkthroughs, diagrams, what an
agent keeps for itself, what it is told to do — kept in one or more
**stores** at once, and **recorded by core** the same way whatever keeps
them: the row in `knowledge_page`, the links in `page_ref`, their pins,
freshness and the events. The store keeps the page; the record is derived
from what it kept. oxplow observes every store the same way, so the UI,
metrics and tracking are common to all of them.

## The oxwiki extension (phase 4, built)

oxwiki is an extension (`extensions/oxwiki`, on by default, requires
oxplow-bundled) that fills bundled's `oxplow/knowledge_record` model as the
store `oxwiki`; the sections below describe core's built-in oxwiki, which
stands down while the extension is enabled (`knowledge::prepare_built_ins`
declares no store then) and goes in step 5 of the removal. The extension:

- **Files.** `files: [{ path: .oxplow/oxwiki, access: write, watch: true }]`:
  the folder is watched and snapshotted though git ignores it
  (`extensions-permissions.md` "Declared folders"), and the fill's `files: {
  folder, suffix: .md }` makes an agent's read of a page's file a read of the
  row (`record.read`, `models.md`).
- **Commands.** `oxwiki.page.write { id?, title?, body }` and
  `oxwiki.page.delete { id }` write and delete `.oxplow/oxwiki/<slug>.md`
  through `oxplow.extension_files.write`/`delete`. The files service resolves
  against the project root, so a write from any worktree stream lands in the
  primary checkout's folder (never refused, never written into a worktree).
  The slug rules are the ones below (kebab-case; a title makes one; a title
  naming an existing page is refused). A body whose `[[links]]` don't resolve
  is refused through the `links.check` service. `oxwiki.page.restore` is their
  undo (the previous file, or none). A write joins bundled's
  `oxplow/knowledge.new` point.
- **The collector** `pages` (`on: [start, { files: ".oxplow/oxwiki/*.md" }]`)
  states every page file as a snapshot of the store's rows (`id`, `title`
  from the first heading, `body`, `kind: note`, `updated_at` from the file's
  mtime), so a hand edit, a new file and a deleted one converge; a file whose
  name isn't a slug is no page.
- **Skills.** `oxwiki` (writing) and the `/note` command, gated on the model;
  the `oxplow-knowledge` skill keeps finding and reading.
- **Reading.** The `oxplow/knowledge` resource is bundled's (as the task
  resource is): `resolve: knowledge_page`, `searchable: search_knowledge`
  over **`v_oxplow_knowledge_page`** — one row per page of the shared
  knowledge model, every store: the old `v_knowledge_page` columns (`uri`,
  `store`, `id`, `kind`, `bucket`, `title`, `url`, `author`, `excerpt`,
  `body_size`, `outbound_uris`, `stale_ref_count`, `read_count`,
  `agent_read_count`, `last_read_at`, `updated_at`) and the page's `body`.
  An agent's reads are `record.read` events, the cited files' staleness
  `v_knowledge_pin`. `oxplow_read` answers the row (its `body` column is the
  content), the ref resolver reads the same dataset by `resolve_view`, and
  the Knowledge pages, the metrics and the desktop's page list read it.
  Core's `v_knowledge_page` / `v_knowledge_body` stay for the built-in until
  step 5. With bundled off the resource is undeclared and a `[[link]]` to a
  page is unrecognized, as a task's is.
- **Reads and touches** are bundled's own models, written by its own
  collectors (the observed-systems way: core keeps no table for them):
  `oxplow/knowledge_read_record` (collector `oxplow.knowledge_read`, on
  `record.read`: an agent's read of a page through `oxplow_read` or its file;
  key = page, time, via, turn) and `oxplow/knowledge_touch_record` (collector
  `oxplow.knowledge_touch`, on `oxplow.knowledge_record.changed`: the thread
  whose run made the change, from the event's thread anchor, so a person's
  edit of a file touches nothing; key = thread and page, `last_seen`). Their
  datasets: `v_oxplow_knowledge_read` (`uri`, `actor`, `thread_id`,
  `turn_id`, `via`, `at`; a person's opens are `page_visit` rows, core's),
  `v_oxplow_knowledge_touch` (`page`, `thread_id`, `last_seen_at`) and
  `v_oxplow_knowledge_ref` (`v_ref_pin` of the pages: `page`, `path`, `kind`,
  `pinned_*`, `stale`). `v_oxplow_knowledge_page`'s read and stale columns
  come from them, so no bundled reader reads core's `v_knowledge_*`; the Work
  panel's Finished list reads the touches. A dataset that reads a view that
  reads views, in a compound select, trips the lineage check (`reads … without
  ref()`): read a model view directly or use CTEs.
- **Kinds.** The metrics slice by `oxplow.knowledge_kind` (the page's kind,
  oxwiki: `note`), not by core's bucket.
- **Editing.** The knowledge page has a `sections` slot
  (`oxplow/knowledge-page.sections`); oxwiki mounts its `edit-page` part there
  (a markdown viz with an `edit: true` Save on `oxwiki.page.write`, empty on
  another store's page), because bundled can't name `oxwiki.*` commands.
- **Skill.** Bundled's `knowledge` skill (gated on the model) is the read and
  find half of what core's `oxplow-knowledge` skill was, with the link rules;
  oxwiki's `oxwiki` skill is the writing half.
- **Points.** Bundled declares `oxplow/knowledge.new` (title?, body?: the
  desktop's Save as knowledge page offers it) and `oxplow/knowledge.delete`
  (the index's row Delete); the store's own commands join them (oxwiki:
  `page.write`, `page.delete`). Inline editing of a page's body on the
  knowledge page is not offered until a store's own page replaces it.
- **The write guard** is generic: a tool's write of a file a fill's `files:`
  names (`<folder>/<id><suffix>`) is refused, naming the store and its
  extension (`write_guard::record_file_reason`).
- **The collector** takes only files directly in the folder, skips a file
  that is bigger than 1 MiB or not UTF-8 (it says so in `problems` of its
  run) rather than failing the snapshot, and takes a heading outside a code
  fence for the title.
- Fixtures: `extensions/oxwiki/fixtures`.

## Stores, pages, kinds

- **One capability, one unit: a page.** A page has a markdown body, a
  title, a store-qualified URI, the URIs it links, and when and by whom
  it last changed.
- **Several stores at once, no priority.** `knowledge` is a *many*
  capability (`oxplow_domain::capability`, like `agent_harness`): every
  declared store is enabled and in use (`v_capability_provider` rows
  `active = 1`, `chosen_by = 'declared'`); there is no choice, no `none`
  and no `capability.switched`. Where a page goes is the writer's choice
  (the agent's config, or what the person asked), never oxplow's.
  `Services.knowledge` is an `oxplow_domain::knowledge::KnowledgeStores`:
  `get(id)`, `enabled()`, `only()` and `resolve(store?)`.
- **oxwiki** is oxplow's own store: a wiki of markdown files, one per page,
  `.oxplow/oxwiki/<slug>.md` (`oxplow_domain::knowledge::OXWIKI_DIR`).
  Its code is the crate `crates/oxplow-oxwiki`: `OxwikiStore` (writes and
  deletes the file), `pages` (restates a page from its file) and `watch`
  (follows the folder). `oxplow-foundation` declares it
  (`implementations:`, id `oxwiki`, built-in entry `oxplow:oxwiki`, which
  core accepts only under that id), so it runs with `oxplow-bundled` off.
- **Another store** is an extension's: a provider process or a script
  (`implementations:` with `capability: knowledge`), registered under its
  instance's id (`providers::knowledge::ExternalKnowledge`,
  `.context/providers.md` "A knowledge provider").
- **URIs carry the store**: `oxplow:oxplow/knowledge?id=<id>&store=<store>`
  (`uri::build::knowledge_uri`, `knowledge_of_uri`; resource `knowledge`,
  label "Knowledge page", declared by foundation). `id` is the store's own
  page id: oxwiki's slug, a vault path, a uuid.
- **Wikilinks.** A bare `[[slug]]` is an oxwiki page. `[[<store>:<id>]]`
  names another enabled store's page (`canonical_wikilink`; the vocabulary
  knows the enabled stores through `ResourceRegistry::with_knowledge_stores`,
  set from `CapabilityRegistry::recognizers`).
- **Kinds and buckets.** A store declares the kinds of page it keeps
  (`KindDecl { id, title, bucket }`), each in one of three core
  **buckets**: `note` (understanding to read later), `memory` (what an
  agent keeps for itself), `instruction` (what an agent is told to do) —
  as a task list's stages map to buckets. A provider or script declares
  them as its capability's `data: { kinds: [...] }`
  (`capability::KNOWLEDGE_DATA`); a store that declares none keeps `note`.
  oxwiki keeps one kind, `note`. Published as `v_capability_provider.kinds`.
- **Feature `edit`**: whether oxplow writes the store's pages. A feature a
  store doesn't declare is off, so a read-only store (instruction files, a
  harness's memory) refuses writes; oxwiki declares it on.

## One write path: `oxplow.knowledge.write_page`

`oxplow.knowledge.write_page { store?, id?, title?, body, verified_files?, removed_files? }`
(`crates/oxplow-app/src/knowledge.rs`, an `External` command: the store
keeps the page) is how a page is written — by an agent (`run_command`),
by the desktop editor, by "Save as knowledge page…", and by "Mark
verified" (`oxplow.knowledge.verify_files`, which writes through it). It:

1. **resolves the store** (`KnowledgeStores::resolve`): the one `store`
   names, which must be enabled; with none named, the only enabled one.
   Refused naming the fix: "no knowledge store is enabled: turn on an
   extension that keeps pages, such as oxwiki"; with
   several, "name the store: oxwiki, …". A store whose `edit` is off
   refuses;
2. validates **every `[[link]]` the write adds** — a task, page, file,
   directory, finding or commit that doesn't exist is refused, each named.
   A link the page's recorded body already has isn't re-checked: a cited
   file or task that has since gone (or a hand edit's bad link) mustn't
   block verifying, rewriting or linking the page
   (`link_check::check_links_in`; a page may link to itself);
3. checks the file declarations (`CitedRefs::bear_out`): `removed_files`
   must be gone from the body; `verified_files` must be cited, or be a
   file under a `[[dir:…]]` the body cites;
4. hands the page to the store (`KnowledgeProvider::write_page`) as a
   `PageDraft`: with no `id` the store **derives one from the title**
   (oxwiki slugifies it: "Auth flow" → `auth-flow`). oxwiki writes the
   file (if it differs; a temp file renamed into place, so a file that
   can't be written fails the run and nothing is recorded); a provider's
   service keeps it. The store answers `Kept { id, body, title?, kind?,
   url?, author?, refs?, events }`;
5. in its own transaction records what the store kept
   (`oxplow_db::knowledge_record::write_page_tx`): the `knowledge_page`
   row (title from the store, else the first `# `, else the id; excerpt;
   `body_hash`) and its `page_ref` edges (`knowledge_edges`, or
   `stated_edges` when the store states its links): edges already stored
   keep their pins, new file refs take the **pin** — the primary stream's
   latest snapshot, plus that snapshot's own revision when it has one —
   and each verified file is re-pinned (a file verified under a cited
   directory becomes an edge of its own, kept while the directory stays
   cited);
6. answers `knowledge.page.written@3 { page: <uri>, store, outbound,
   snapshot }` (with the actor's anchors, after any events the store
   reported) for the bus to log after `command.executed`. Were the record
   to fail after the store kept the page, oxwiki's file is ahead of the
   row and the watch converges them.

`@version` literals in links are stripped from the body: a version is the
edge's pin, not the prose's. `title` sets the body's `# ` heading.

Beside it:

- `oxplow.knowledge.delete_page { store?, id }` (Destructive): refused for a
  page core doesn't record; the store deletes it (oxwiki its file, a
  failure failing the run), then core drops the row and edges and answers
  `knowledge.page.deleted@3`.
- `oxplow.knowledge.link { store?, id, target }` (`target` a wikilink's
  inside): **core's**, the same for every store. Refused when the recorded
  page already has `- [[target]]` or when the target doesn't resolve;
  otherwise core adds `- [[target]]` at the end of the page's
  `## Related` section (before any section after it, or a new one at the
  end) and writes the body through `write_page`.
- `oxplow.knowledge.verify_files { store?, id, files? }`: re-pin the named
  files, or every file the page cites, to now, by rewriting the recorded
  body through `write_page` with them as `verified_files`.
- `oxplow.knowledge.resync { id }`: oxwiki's — restate a page from its file
  (a `Tx` command; the repair when a row and its file disagree; a missing
  file deletes the page). Refused while oxwiki is off.

All are `Record`: any thread captures what it explored.

**Agents never write oxwiki's files.** The write guard (hook and
`decide_tool`) refuses an agent's Write/Edit into `.oxplow/oxwiki/` for
every thread, pointing at the command (`write_guard::oxwiki_page_reason`).

## oxwiki's files converge

A person editing a file (or anything else changing it) converges through
the same record: `oxplow_oxwiki::pages::sync_page` restates the page from
disk, logged as `system:oxwiki_watch` — without the link refusal (whatever
is on disk is recorded). A body whose hash matches the row's is a no-op,
so a page the command just wrote doesn't sync (or log) twice; a missing
file deletes the page; an unreadable one is an error, never a delete.
**One slug rule** (`oxplow_oxwiki::valid_slug`, kebab-case): a file whose
name isn't a slug is not a page — syncing it records nothing, and removes
a row it left. `updated_at` follows the file's mtime on this path, so a
scan doesn't reset recency.

`oxplow_oxwiki::watch::follow` follows the primary stream's watch of the
folder while oxwiki is present, and **scans every file each time oxwiki
appears** (`OxwikiPresence`, set by the publication that declares it:
boot's, or turning it back on). While oxwiki is off its files are left
alone.

## Turning oxwiki off

oxwiki is an extension: turn it off with its extension (Settings → Extensions,
or `disableExtensions: [oxwiki]` in a settings file). There is no setting of
its own and no `oxplow.knowledge.set_oxwiki`. Off, its store leaves the
knowledge model's stores (`v_model_store`), its pages are hidden and its
files left alone; on again, its collector rescans the folder. Settings →
Integrations → Knowledge stores lists the stores of the model with the
extension each is from.

## Hidden, not deleted

The record keeps every store's pages. `v_knowledge_page` joins the enabled
stores (`capability_provider` rows of `knowledge` with `active = 1`), so a
store that is turned off, or a provider instance that stopped, has its
pages hidden; they show again when it comes back (oxwiki rescans; a
provider restates on its next sync). Nothing deletes a store's pages but
its own deletes.

## A provider's pages

A provider's store answers each write with the page's
`knowledge.page.recorded@3 { page: { uri, kind?, url?, author?, title,
body, uris, updated_at, deleted } }`; its `uris` (oxplow's grammar) are
what core links and pins (`PageWrite.stated`), never a body parsed in the
store's own grammar. Its collector streams the pages edited outside
oxplow as the same records; `providers::knowledge::KnowledgeRecords`
restates each one whose source is the provider (`provider:<id>`), of its
own store, while that store is enabled. A record whose body the row
already holds restates nothing.

## Attribution

`KnowledgeAttribution` (pump consumer `knowledge.attribution`) marks a page
touched by the thread whose command wrote it (`knowledge_page_touch`,
`v_knowledge_touch`) from `knowledge.page.written`'s thread anchor.

## Reads are SQL

- **`v_knowledge_page`** — one row per page of an enabled store: `uri`,
  `store`, `id`, `kind`, `bucket`, `title`, `url` (open it in the store's
  own app), `author`, `excerpt`, `body_size`, `outbound_uris` (JSON array
  of every URI it links to), `stale_ref_count`, `read_count` (every read,
  a person's opens too), `agent_read_count` (agents' only),
  `last_read_at`, `updated_at`.
- **`v_knowledge_ref`** — each page's file refs: `page`, `path`,
  `pinned_snapshot_id`, `pinned_vcs_rev`, `pinned_vcs_rev_exact`,
  `latest_snapshot_id`, `stale` (over `v_knowledge_pin`, every recorded
  page's).
- **`v_knowledge_body`** — `uri`, `body`: each page's markdown, apart from
  `v_knowledge_page` so listing pages never reads bodies.
- **`v_knowledge_touch`** — which threads wrote which pages.
- **`v_knowledge_read`** — every read of a page (see "Reads"); the page's
  `read_count` and `last_read_at` count them.
- **`v_search_knowledge`** — what site search indexes (`oxplow_search`,
  resource `oxplow/knowledge`); its asset restates the index when
  `knowledge_page` commits.

A ref is **stale** when its file has a **primary-stream** snapshot newer
than the one it was pinned to, or was captured there but never pinned.
The primary is the stream a pin comes from (`pin_tx`), so another stream's
newer snapshot of the file neither makes a page stale nor keeps a verified
one stale. `v_knowledge_ref` is the **one definition**:
`v_knowledge_page.stale_ref_count` counts its rows, `knowledge::freshness`
reads it, and so do the UI's freshness chip and page.
`oxplow_knowledge_drift { store?, id, path }` (MCP) shows one stale ref's
diff; a pin taken by "Track changes only" (no `contents`) holds an
identity and no text, so its status is `no_contents`, not an error.

**Drift is an event.** A primary-stream take that changed files a
recorded page cites appends `knowledge.page.drifted@1 { page, snapshot,
paths }` per page, caused by the `snapshot.taken`
(`knowledge::KnowledgeDrift`, consumer `knowledge.drift`), whether or not
the page was stale already. Another stream's take, or one that changed
no cited file, says nothing. What re-counts stale pages follows it
instead of every snapshot (bundled's `oxplow.knowledge_pages`): most takes
touch no cited file, and each count was a capture every metric reader
heard.

**The body is in the row** (`knowledge_page.body`). `.oxplow/.gitignore`
is `*`, so the workspace filter hides oxwiki's files — the file can't be
the UI's read path. Every write keeps the body with the row.

**The desktop** (`apps/desktop/src/knowledge.ts`) reads pages, a page and
its freshness from these datasets by URI; `useKnowledgeStores` (and
`knowledgeStores.ts`, started at boot) reads the enabled stores, and
`knowledgeStoreIds.ts` is what markdown reads `[[<store>:<id>]]` with.
Every read returns what it read (`reads`) and re-reads when one of those
datasets changes.

## Reads

Reading pages is what knowledge is for, so oxplow observes every read,
for every store, in `v_knowledge_read` (`uri, actor, thread_id, turn_id,
via, at`):

- **An agent's `oxplow_read`** (`oxplow.records.read`) of a page returns
  its row and, as `content`, its body (`v_knowledge_body`). The command
  logs `knowledge.page.read@1 { page, via: tool }` itself, as the agent
  (`knowledge::log_read`): a read runs unrecorded, so the events a handler
  returns would go nowhere (`.context/commands.md`). A person's read
  through the same command is no agent's read.
- **An agent reading a page's file with its own tool** (an
  `agent.tool.finished` of kind `Read`) reads the page, `via: file`: each
  path — the worktree's when relative, else absolute — that an enabled
  store says is a page's file (`KnowledgeProvider::page_at`; oxwiki:
  `.oxplow/oxwiki/<slug>.md`). A path that is no page's is nothing.
- One mechanism for both: a file read becomes a `knowledge.page.read {
  via: file }` too (`knowledge::KnowledgeFileReads`, consumer
  `knowledge.file_reads`, the tool call its cause), and
  `knowledge::KnowledgeReads` (`knowledge.reads`) turns every
  `knowledge.page.read` into a `knowledge_page_read` row, only for recorded
  pages, with the event's thread and turn. A tool read carries the agent's
  open turn (`activity_anchors_tx`), so its request counts as one that read
  a page. A deleted page's reads go with it.
- **A person opening a page** is their page visit (`page_visit` rows of
  `page_kind = 'oxplow/knowledge'`, `via: open`): no event of its own.

## The store interface

`oxplow_domain::knowledge::KnowledgeProvider` is a **store**: `id()`,
`kinds()`, `features()` (`edit`), `page_at(path)` (the page a file is,
when its pages are files: oxwiki's `.oxplow/oxwiki/<slug>.md`),
`write_page(call, draft)` →
`Kept`, `delete_page(call, id)` → its events. A store never sees pins or
snapshots: the record, links and **freshness are core's**, the same for
every store (`oxplow_db::knowledge_record`). Adding a link is core's too.

`knowledge_conformance::suite(svc, store, actor)` is what a store must do,
checked through the commands (each naming the store, so it runs beside
others) and core's record: a write lands the page, its recorded body and
its event; a pinned ref is fresh, goes stale when its file drifts, stays
stale through an unverified rewrite (which still moves `updated_at`) and
is fresh again once verified; a dangling link is refused, named; a link to
another page of the store (`[[<store>:<id>]]`) lands in the body; a delete
takes the page and logs it. oxwiki passes it
(`oxwiki_is_a_conforming_store`), and so do the script and provider
stores in the tests and `oxplow extension test`.

## Agents

Every harness gets the same: the `oxplow-knowledge` skill (find before you
create, across stores; read with `oxplow_read`; write with `store?`, `id?`
and a title; links as `[[…]]`, `[[<store>:<id>]]` for another store), a
bullet of the working-in-oxplow preamble saying knowledge exists and where,
and core's slash command `/oxplow:note` (keep what was just worked out as a
page, following the skill). An agent never writes a store's files: the
write guard refuses oxwiki's (`oxwiki_page_reason`).

## Removed for good

The wiki was one chosen store (`oxplow:wiki` under the id `oxplow`, or
`none`, a sink) and its pages were `oxplow:oxplow/wiki?slug=…`. V28 dropped
`wiki_page` and `wiki_page_thread_update` and deleted the old pages'
events, links, visits, bookmarks, comments, search entries and context
items; nothing was carried over. Thread notes are gone too: durable
understanding is a knowledge page; what an agent did is its turns,
requests and handoffs.

## Pages

The screens are `oxplow-bundled`'s (`.context/extensions-bundled.md`
"The knowledge pages"); this extension holds only the data they read.

- **Knowledge** (`knowledge`): the stores, the pages, the stale ones and
  recent reads and writes; New page and Delete.
- **Knowledge Page** (`for: oxplow/knowledge`): one page by its URI. The
  body is editable only when its store's `edit` is on. A store with
  `url` set gets Open in its store.
- **Knowledge Freshness** (`store`, `id`): the files a page cites.

They read `v_knowledge_page`, `v_knowledge_body`, `v_knowledge_ref`,
`v_knowledge_read`, `v_page_ref` and `v_capability_provider`, and write
through `oxplow.knowledge.write_page`, `delete_page` and `verify_files`.
Never a slug: a page is its `store` and `id`.
