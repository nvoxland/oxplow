---
name: oxplow-knowledge
description: Keeping and finding knowledge pages — durable understanding (codebase walkthroughs, design rationale, comparisons, tradeoffs, recommendations) in the project's knowledge stores (oxwiki, oxplow's own, and any an extension adds). Find before you create; read a page with oxplow_read; a store's own command writes it. Loads on v_oxplow_knowledge_page, v_knowledge_pin, the oxplow `oxplow_search` tool (resource oxplow/knowledge), v_page_ref, and when the user asks "how does X work", "where is X", "explain X", "trace X", "give me an overview", "walk me through X", "why does/did/should X", "compare X and Y", "what are the tradeoffs", "should I use X or Y", "rationale behind X", or says "save this" / "add a note" / "keep this".
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
`v_oxplow_knowledge_page` (`uri`, `store`, `id`, `kind`, `title`, `excerpt`,
`body`, `stale_ref_count`, `read_count`), then `oxplow_search` with `resources:
["oxplow/knowledge"]`. Read a page with `oxplow_read { uri }`: its row and
its body (in SQL, the `body` column). `v_model_store` (`WHERE model =
'oxplow/knowledge_record'`) lists the stores that keep pages; `v_knowledge_pin`
lists the files a page cites and whether each changed since the page was
pinned; an agent's read of a page is a `record.read` event in `v_event`. If one fits, add a dated section to it; create a page only when
none does. `references/search-existing.md` has the lookups.

## Writing

A knowledge store writes its pages with its own command, and its skill says
how (oxwiki: the `oxwiki` skill, `oxwiki.page.write`). Never write a store's
files yourself. Every `[[link]]` must resolve; a dangling one is refused and
named. With no store on, say once that the page wasn't kept.

`run_command oxplow.refs.verify { uri, files? }` records that you re-read the
cited files against a page (all of them when `files` is left out), so its pins
move to now.

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
