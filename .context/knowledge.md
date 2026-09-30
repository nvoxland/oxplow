# Knowledge (the wiki)

The per-project wiki: pages at `.oxplow/wiki/<slug>.md`, their row in
`wiki_page` and their links in `page_ref` (P5.C3, `target-architecture.md`
§6). Pages are agent-written writeups, diagrams and explanations; the
file is the page, the row and edges are derived from it.

## One write path: `knowledge.write_page`

`knowledge.write_page { slug, title?, body, verified_refs?, removed_refs? }`
(`crates/oxplow-app/src/knowledge.rs`, a `Tx` command) is how a page is
written — by an agent (`run_command`), by the desktop editor, and by
"Mark verified" on the Freshness page. In the bus's transaction it:

1. validates the slug (kebab-case) and **every `[[link]]`** — a task,
   page, file, directory, finding or commit that doesn't exist is refused,
   each named (`link_check::check_links_in`, synchronous: the database,
   the project's files and `Vcs::revision_graph` for commits; a page may
   link to itself);
2. checks the ref declarations: `removed_refs` must be gone from the
   body; `verified_refs` must be cited, or be a file under a
   `[[dir:…]]` the body cites;
3. restates the `wiki_page` row (title from the first `# `, excerpt,
   `body_hash`) and merges its `page_ref` edges (`wiki_edges` +
   `merge_source_tx`): edges already stored keep their pins, new file
   refs take the **pin** — the primary stream's latest snapshot, plus
   that snapshot's own revision when it has one — and each verified ref
   is re-pinned (a file verified under a cited directory becomes an edge
   of its own, kept while the directory stays cited);
4. logs `knowledge.page.written@1 { page: "wiki:<slug>", outbound,
   snapshot }` with the actor's anchors;

and once committed writes the file (if it differs) and announces
`WikiPagesChanged`. `@version` literals in links are stripped from the
body: a version is the edge's pin, not the prose's. `title` sets the
body's `# ` heading.

Beside it: `knowledge.delete_page { slug }` (Destructive; row, edges and
file; `knowledge.page.deleted@1`), `knowledge.link { page, target }`
(adds `- [[target]]` under the page's `## Related`, validated), and
`knowledge.resync { slug }` (restate from the file; the repair when a row
and its file disagree). All are `Record`: a read-only thread captures
what it explored too. `knowledge.write_page` replaced MCP
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
error, never a delete. `updated_at` follows the file's mtime on this
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
  `pinned_snapshot_id`, `latest_snapshot_id`, `stale`.

A ref is **stale** when its file has a snapshot newer than the one it was
pinned to, or was captured but never pinned — the rule the Freshness page
(`list_wiki_freshness`) and `KnowledgeProvider::freshness` use too.
`wiki_ref_drift` (MCP) shows one stale ref's diff. Bodies are searched
with the site `search` tool: the `search.index` pump consumer indexes a
page from `knowledge.page.written` / `deleted` (written by command or by
hand alike). The MCP read tools went; the desktop still reads through
its `list_wiki_pages` / `read_wiki_page_body` RPCs until the knowledge
pages move onto the models (P6).

## The capability

`oxplow_domain::knowledge::KnowledgeProvider`: `provider()`,
`write_page(actor, PageDraft)`, `delete_page`, `link`, and
`freshness(page) -> Vec<RefFreshness>` — freshness is the provider's to
say (a provider that can't pin to snapshots reports what it can).
`OxplowKnowledge` (`Services.knowledge`) runs the `knowledge.*` commands
as the actor (a destructive one confirmed: the provider call is the
caller's decision — an agent's is still left for a person, by the bus)
and reads freshness from the pins.

`knowledge_conformance::suite(provider, probe, actor)` is what a provider
must do: a write lands the page, its body and its event; a pinned ref is
fresh, goes stale when its file drifts, stays stale through an unverified
rewrite (which still moves `updated_at`) and is fresh again once
verified; a dangling link is refused, named; a delete takes the page and
logs it. The `KnowledgeProbe` reads the host and moves the world (a file
changes and is captured). oxplow's wiki passes it
(`the_oxplow_wiki_is_a_conforming_provider`, as a person).
