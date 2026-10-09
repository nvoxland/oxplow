# Knowledge (the wiki)

Pages of durable understanding — agent-written writeups, diagrams and
explanations — kept in the project's chosen **store** (oxplow's wiki,
pages at `.oxplow/wiki/<slug>.md`; a provider's documentation system;
or none), and **recorded by core** the same way whatever keeps them: the
row in `wiki_page`, the links in `page_ref`, their pins, freshness and
the events (P5.C3). The store keeps the page; the record is derived from
what it kept.

## One write path: `oxplow.knowledge.write_page`

`oxplow.knowledge.write_page { slug, title?, body, verified_refs?, removed_refs? }`
(`crates/oxplow-app/src/knowledge.rs`, an `External` command: the store
keeps the page) is how a page is written — by an agent (`run_command`),
by the desktop editor, and by "Mark verified" on the Freshness page. It:

1. validates the slug (kebab-case) and **every `[[link]]` the write
   adds** — a task, page, file, directory, finding or commit that doesn't
   exist is refused, each named. A link the page's recorded body already has isn't
   re-checked: a cited file or task that has since gone (or a hand edit's
   bad link) mustn't block verifying, rewriting or linking the page
   (tsk564) (`link_check::check_links_in`, synchronous: the database,
   the project's files and `Vcs::revision_graph` for commits; a page may
   link to itself);
2. checks the ref declarations (`refs_borne_out`): `removed_refs` must be
   gone from the body; `verified_refs` must be cited, or be a file under a
   `[[dir:…]]` the body cites — both before anything reaches the store;
3. hands the page to the **active store** (`KnowledgeProvider::
   write_page`): the wiki writes the file (if it differs; a temp file
   renamed into place, so a file that can't be written fails the run and
   nothing is recorded), a provider's service keeps it, none keeps
   nothing and the command answers `{ page, tracked: false }`;
4. in its own transaction (`write_page_tx`), from the body the store
   kept, restates the `wiki_page` row (title from the first `# `, excerpt,
   `body_hash`) and merges its `page_ref` edges (`wiki_edges` +
   `merge_source_tx`): edges already stored keep their pins, new file
   refs take the **pin** — the primary stream's latest snapshot, plus
   that snapshot's own revision when it has one — and each verified ref
   is re-pinned (a file verified under a cited directory becomes an edge
   of its own, kept while the directory stays cited);
5. answers `knowledge.page.written@1 { page: "wiki:<slug>", outbound,
   snapshot }` (with the actor's anchors, after any events the store
   reported of its own) for the bus to log after `command.executed`,
   caused by the run. Were the record to fail after the store kept the
   page, the wiki's file is ahead of the row and the watcher converges
   them. The search index reads the body from the row (`wiki_page.body`),
   so it is the recorded one when its asset recomputes;

(the UI re-reads on the `modelsChanged` the row write produces; there is
no wiki event of its own). `@version` literals in links are stripped from the
body: a version is the edge's pin, not the prose's. `title` sets the
body's `# ` heading.

Beside it, dispatched the same way: `oxplow.knowledge.delete_page {
slug }` (Destructive; refused for a page core doesn't record unless the
store is a sink; the store deletes it — the wiki its file, a failure
failing the run — then core drops the row and edges and answers
`knowledge.page.deleted@1`), `oxplow.knowledge.link { page, target }`
(`target` a wikilink's inside; refused when the recorded page already
links it, or when it doesn't resolve; the store adds `- [[target]]` at
the end of the page's `## Related` section — the wiki before any section
after it, or a new one at the end, with `@version` literals stripped;
tsk572 — and core records the body it kept), and
`oxplow.knowledge.resync { slug }` (the wiki's: restate from the file, a
`Tx` command; the repair when a row and its file disagree). All are `Record`: a read-only thread captures
what it explored too. `oxplow.knowledge.write_page` replaced MCP
`record_wiki_page_update`, `resync_wiki_page` and `delete_wiki_page`, and
RPCs `write_wiki_page_body`, `upsert_wiki_page`, `delete_wiki_page`,
`mark_wiki_ref_verified` and `mark_all_wiki_refs_verified`.

**Agents never write the file.** The write guard (hook and
`decide_tool`) refuses an agent's Write/Edit into `.oxplow/wiki/` for
every thread, pointing at the command (`write_guard::wiki_page_reason`).

## Hand edits converge

A person editing the file (or anything else changing it) converges
through the same core: `wiki_pages::sync_page` restates the page from
disk, logged as `system:wiki_watch` — without the link refusal (whatever
is on disk is recorded). The watcher (`wiki_pages_watch`) runs it per
changed file and `scan_and_sync_all` at boot. A body whose hash matches
the row's is a no-op, so a page the command just wrote doesn't sync (or
log) twice; a missing file deletes the page; an unreadable one is an
error, never a delete. **One slug rule** (`knowledge::valid_slug`,
kebab-case): a file whose name isn't a slug is not a page — syncing it
records nothing, and removes a row it left (tsk572) — so every row is
one the commands can write and delete. `updated_at` follows the file's mtime on this
path, so a boot scan doesn't reset recency.

## Attribution

`WikiAttribution` (pump consumer `wiki.attribution`) marks a page touched
by the thread whose command wrote it (`wiki_page_thread_update`, the
rail's "Finished" list) from `knowledge.page.written`'s thread anchor.

## Reads are SQL (P5.C4)

- **`v_knowledge_page`** — one row per page: `ref` (`wiki:<slug>`),
  `provider`, `slug`, `title`, `excerpt`, `body_size`, `outbound_refs`
  (JSON array of every ref it links to), `stale_ref_count`, `updated_at`.
- **`v_knowledge_ref`** — each page's file refs: `page`, `path`,
  `pinned_snapshot_id`, `pinned_vcs_rev`, `pinned_vcs_rev_exact`,
  `latest_snapshot_id`, `stale`.

A ref is **stale** when its file has a **primary-stream** snapshot newer
than the one it was pinned to, or was captured there but never pinned.
The primary is the stream a pin comes from (`pin_tx`), so another
stream's newer snapshot of the file neither makes a page stale nor keeps
a verified one stale (tsk563). `v_knowledge_ref` is the **one
definition**: `v_knowledge_page.stale_ref_count` counts its rows,
`KnowledgeProvider::freshness` reads it, and so do the wiki page's
freshness chip and the Freshness page.
- **`v_knowledge_body`** (P6.E2) — `ref`, `body`: each page's markdown,
  apart from `v_knowledge_page` so listing pages never reads bodies.
- **`v_knowledge_touch`** — which threads wrote which pages (the rail's
  Finished section).

**The body is in the row** (`wiki_page.body`, V125). `.oxplow/.gitignore`
is `*`, so the workspace filter hides `.oxplow/wiki/*.md` — the file
can't be the UI's read path. `write_page_tx` writes the body with the
row, so the command, `link`, `resync` and the watcher's `sync_page_tx`
all keep it; V125 cleared `body_hash` so the first boot's scan restated
every page from its file. The file is still the page (and still written
inside the run); the row now carries its text as well as its hash.

`wiki_ref_drift` (MCP) shows one stale ref's diff; a pin taken by "Track changes only" (no `contents`) holds an identity and no text, so its status is `no_contents`, not an error. Bodies are searched
with the site `search` tool: the `wiki` search kind is an asset over
`v_search_wiki` (tsk864), restated when `wiki_page` commits — a page
written by command or by hand alike — reading the body from the row. The old excerpt-only
`wiki_page_fts` mirror is gone (V125).

**The desktop** (`apps/desktop/src/knowledge.ts`) reads the index, a
page and its freshness from these models and finds pages with the site
search (`kinds: ["wiki"]`, matches marked `«…»`); every read returns what
it read (`reads`) and every wiki surface re-reads it through
`useRerunOnChange` (the title cache through `readsChanged`) when one of
those models changes — a drifting ref re-reads the freshness rows without
reopening the page (`WikiFreshnessPage.rerun.test.tsx`). The RPCs
`list_wiki_pages`, `read_wiki_page_body`, `search_wiki_titles` and
`list_wiki_freshness` are gone.

## The capability

Knowledge is a **choosable, optional** capability: a project keeps its
pages in oxplow's wiki (`oxplow:wiki`, which `oxplow-bundled` declares
under the id `oxplow`), in a documentation system an extension's provider
speaks to (an Obsidian vault, a Confluence or MediaWiki server), or
nowhere. `Services.knowledge` is an `oxplow_domain::knowledge::
KnowledgeRegistry`; `active()` is the project's choice
(`activeProviders.knowledge`, personal over project over the default).
`knowledge::register_built_ins` registers the wiki under each id
declared and core's **none**, `NoneKnowledge`: a sink, as the work
list's none is — a write lands nowhere and answers its page ref, a
delete and a link succeed, freshness is empty — so nothing that writes
pages is refused while no store is active, and the record reads empty.
Bundled disabled, knowledge is none. The wiki pages list and a page's
freshness page say "No knowledge store — choose one in Settings →
Capabilities" when the active row is `none` (`useKnowledgeIsNone`); the
Settings row for the wiki reads "oxplow's wiki — pages as Markdown under
.oxplow/wiki, pinned to snapshots".

`oxplow_domain::knowledge::KnowledgeProvider` is a **store**: `id()`,
`sink()` (none's `true`: it takes any page), and `write_page(call,
draft)` / `link(call, slug, target)` → `Option<Kept { body, events }>`
(the body as it now stands there, and events of its own — a provider's
`knowledge.page.recorded`), `delete_page(call, slug)` →
`Option<events>`; `None` is "kept nothing". A store never sees pins or
snapshots: the record, links and **freshness are core's**
(`knowledge::freshness(db, page)` over `v_knowledge_ref`, the one
definition of staleness), the same for every store. `WikiKnowledge` is
the wiki: it keeps `.oxplow/wiki/<slug>.md`.

`knowledge_conformance::suite(svc, actor)` is what a store must do,
checked through the commands and core's record: a write lands the page,
its recorded body and its event; a pinned ref is fresh, goes stale when
its file drifts, stays stale through an unverified rewrite (which still
moves `updated_at`) and is fresh again once verified; a dangling link is
refused, named; a link lands in the body; a delete takes the page and
logs it. It moves the world by recording a snapshot of a file it
changes. oxplow's wiki passes it (`the_oxplow_wiki_is_a_conforming_store`,
as a person).
