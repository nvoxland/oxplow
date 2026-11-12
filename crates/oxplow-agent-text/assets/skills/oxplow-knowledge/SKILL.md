---
name: oxplow-knowledge
description: Keeping and finding knowledge pages — durable understanding (codebase walkthroughs, design rationale, comparisons, tradeoffs, recommendations) in the project's knowledge stores (oxwiki, oxplow's own, and any an extension adds). Find before you create; read a page with oxplow_read; write with oxplow.knowledge.write_page. Loads on v_knowledge_page, v_knowledge_ref, v_knowledge_read, the oxplow `oxplow_search` tool (resource oxplow/knowledge), v_page_ref, oxplow.knowledge.write_page, on /note, and when the user asks "how does X work", "where is X", "explain X", "trace X", "give me an overview", "walk me through X", "why does/did/should X", "compare X and Y", "what are the tradeoffs", "should I use X or Y", "rationale behind X", or says "save this" / "add a note" / "keep this".
---

# Knowledge pages

A knowledge page is durable understanding: how a subsystem works, why a
design landed, tradeoffs, recommendations. Not codebase-only: any
non-trivial exploratory answer may be one. Bodies are markdown.

Pages live in **stores**, several at once: **oxwiki** (oxplow's own, a
wiki of markdown files) and any an extension adds (a vault, a docs
server). `v_capability_provider WHERE capability = 'knowledge' AND active
= 1` lists the stores on; each declares kinds of page (`note`, …).

## When to keep a page

Keep one when the user asked an exploratory question, the answer took
synthesis, and it's worth keeping. Skip edits and commits (they record
themselves) and one-line lookups. `/note`, "save this" or "keep this"
always keep one.

## Find before you create

Don't fragment. Look across every store first: `oxplow_query_sql` over
`v_knowledge_page` (`uri`, `store`, `id`, `kind`, `title`, `excerpt`,
`stale_ref_count`), then `oxplow_search` with `resources:
["oxplow/knowledge"]`. Read a page with `oxplow_read { uri }`: its row and
its body (in SQL, `v_knowledge_body`: `uri`, `body`). If one fits, add a dated section to it; create a page only when
none does. `references/search-existing.md` has the lookups.

## Writing

`run_command oxplow.knowledge.write_page { store?, id?, title?, body,
verified_files, removed_files }`. One run writes the page through its
store and records it; never write a store's files yourself.

- `store`: needed only when more than one store is on (refused naming
  them otherwise). The person's or your config's choice, never a guess.
- `id`: the store's page id (oxwiki: kebab-case, topic-shaped,
  `stop-hook-pipeline`, never dated). Omit it on a new page and give a
  `title`: the store derives the id. A title naming an existing page is
  refused: pass its `id` to replace it.
- Every `[[link]]` must resolve; a dangling one is refused and named.
- `verified_files`: files you re-read against this body (their pins move
  to now). `removed_files`: files you took out.

`references/write-page.md` has the details: appending, `link`,
`verify_files`, `delete_page`, no store on.

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
