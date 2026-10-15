# Writing a wiki page

1. For an existing page, read `.oxplow/wiki/<slug>.md` and merge your
   addition into its body. Refs that drifted since they were pinned are
   `v_knowledge_ref` rows with `stale = 1`; `oxplow_wiki_ref_drift` shows what
   changed in one.
2. Write the whole page with `run_command`:
   `oxplow.knowledge.write_page { slug, body, verified_refs, removed_refs }`
   (`title` optionally sets the `# ` heading). That one run writes the row,
   the links and `.oxplow/wiki/<slug>.md`. Every `[[link]]` must resolve: a
   dangling one is refused and named, so fix or drop it and run again.
   - `verified_refs`: files you re-read against this body; their freshness
     pins move to now. A file under a `[[dir:…]]` the page cites counts.
   - `removed_refs`: files you took out of the body.
   - Refs left in place without re-checking go in NEITHER list, keeping
     their pin. Never write `@disk` / `@HEAD` / `@<sha>`: they are stripped
     on parse; the pin is stamped at write time.
   `oxplow.knowledge.link { page, target }` adds one link under the page's
   Related heading; `oxplow.knowledge.delete_page { slug }` deletes a page
   (the person confirms).
3. Optionally declare the page in `oxplow.effort.report`'s `impacts`:
   `{ kind:"wiki", id:"<slug>", action:"created"|"updated" }`, which
   backlinks your effort's item to the page.

## A read-only thread

`oxplow.knowledge.write_page` is open to every thread; capture exactly as
on the writer thread. The wiki is where exploration goes regardless of
writer status.

## A project with no knowledge store

A project can choose no store for knowledge (Settings → Capabilities).
`write_page` then still succeeds but the page lands nowhere: its answer says
`tracked: false` and the wiki reads empty. Don't retry or work around it;
say once that the page wasn't kept and that choosing a knowledge store in
Settings → Capabilities keeps pages.

## Folding in Explore findings

If this turn dispatched Explore subagents that wrote findings into thread
notes (`oxplow.knowledge.update_note`), read `v_thread_note` with
`oxplow_query_sql` and fold their findings into the wiki page rather than
discarding them. Subagent notes are otherwise invisible; the wiki is where
they become durable.

## Diagrams

Common picks: hierarchies and dependencies → `graph TD`; lifecycles →
`stateDiagram-v2`; cross-component flows → `sequenceDiagram`; evolution by
phase → `timeline`. Wide state-vs-condition matrices stay markdown tables.
Pair each diagram with a sentence saying what to look at.
