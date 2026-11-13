---
name: knowledge
description: Finding and reading knowledge pages — durable understanding (codebase walkthroughs, design rationale, comparisons, tradeoffs, recommendations) in the project's knowledge stores (oxwiki, and any an extension adds), and how to write a link in any text. Find before you create; read a page with oxplow_read; a store's own command writes it. Loads on v_oxplow_knowledge_page, v_oxplow_knowledge_read, v_oxplow_knowledge_touch, v_oxplow_knowledge_ref, the oxplow `oxplow_search` tool (resource oxplow/knowledge), v_page_ref, and when the user asks "how does X work", "where is X", "explain X", "trace X", "give me an overview", "walk me through X", "why does/did/should X", "compare X and Y", "what are the tradeoffs", "should I use X or Y", "rationale behind X", or says "save this" / "add a note" / "keep this".
---

# Knowledge pages

A knowledge page is durable understanding: how a subsystem works, why a
design landed, tradeoffs, recommendations. Not codebase-only: any
non-trivial exploratory answer may be one. Bodies are markdown.

Pages are rows of one shared model (`oxplow/knowledge_record`), kept by a
**store** (several at once: oxwiki, and any an extension adds). The store's
own skill teaches its commands (oxwiki: the `oxwiki` skill); this one teaches
how to read the model. `v_model_store WHERE model =
'oxplow/knowledge_record'` lists the stores.

## When to keep a page

Keep one when the user asked an exploratory question, the answer took
synthesis, and it's worth keeping. Skip edits and commits (they record
themselves) and one-line lookups. `/note`, "save this" or "keep this"
always keep one.

## Find before you create

Don't fragment. Look across every store first: `oxplow_query_sql` over
`v_oxplow_knowledge_page` (`uri`, `store`, `id`, `kind`, `title`, `excerpt`,
`body`, `outbound_uris`, `stale_ref_count`, `read_count`, `updated_at`) —
`SELECT uri, store, title FROM v_oxplow_knowledge_page WHERE title LIKE
'%hook%'` — then `oxplow_search` with `resources: ["oxplow/knowledge"]`
(full text over bodies; it catches pages that discuss a topic without being
named for it). Read a page with `oxplow_read { uri }`: its row, the `body`
column the page. If one fits, add a dated section to it; create a page only
when none does.

- **What a page cites.** `v_oxplow_knowledge_ref` (`page`, `path`, `kind`,
  `pinned_snapshot_id`, `stale`): the files and directories a page pins and
  whether each changed since. The pages that cite a file: `WHERE path =
  '<path>'`; the stale ones: `WHERE stale = 1`. `run_command
  oxplow.refs.verify { uri, files? }` records that you re-read the cited
  files against a page (all of them when `files` is left out), so its pins
  move to now.
- **Backlinks.** `v_page_ref` (`source_uri`, `target_uri`, `ref_type`):
  `WHERE target_uri = '<the page's uri>'` is what links to it.
- **Reads and writers.** `v_oxplow_knowledge_read` (`uri`, `actor`, `via`,
  `at`: an agent's reads through `oxplow_read` or the page's file, and a
  person's opens) and `v_oxplow_knowledge_touch` (`page`, `thread_id`,
  `last_seen_at`: which thread last wrote a page).

## Writing

A knowledge store writes its pages with its own command, and its skill says
how. Never write a store's files yourself. Every `[[link]]` must resolve; a
dangling one is refused and named. With no store on, say once that the page
wasn't kept.

## Links — the one way to name an oxplow object

Write files, commits, tasks and pages as `[[…]]` links, never as plain
text, in every body you write:

- `[[src/foo.ts]]` file (`:42` a line, `|label` custom text)
- `[[dir:src/components]]` directory
- `[[abc1234]]` / `[[git:abc1234]]` commit
- `[[tsk<n>]]` task: its own id, never `#<n>`
- `[[some-page]]` an oxwiki page; `[[<store>:<id>]]` another store's page

Never write `@<version>` on a file link; freshness is tracked per ref.

## Diagrams

Bodies render ```mermaid fences. Draw when a relationship is clearer
drawn than described; ~12 nodes; say in prose what to look at. The
oxplow-mermaid skill has the syntax rules.
