---
name: oxplow-wiki-capture
description: Capturing non-trivial exploratory Q&A into wiki pages — codebase walkthroughs AND general synthesis (design rationale, comparisons, tradeoffs, recommendations, advice). The wiki is for any durable understanding worth keeping, not just code questions. Loads on v_knowledge_page, v_knowledge_ref, the oxplow `oxplow_search` tool (kind wiki), v_page_ref, oxplow.knowledge.write_page, on /note, and when the user asks "how does X work", "where is X", "explain X", "trace X", "describe the architecture", "give me an overview", "summarize the codebase", "walk me through X", "why does/did/should X", "what's the difference between X and Y", "compare X and Y", "what are the tradeoffs", "should I use X or Y", "what's the best way to X", "rationale behind X", "advice on X", or says "save this" / "add a note" / "add to the wiki".
---

# Wiki pages — exploratory capture

The per-project wiki (`.oxplow/wiki/<slug>.md`) holds durable
understanding: how subsystems work, why a design landed, tradeoffs,
comparisons, recommendations. **Not codebase-only**: any non-trivial
exploratory Q&A belongs here. Bodies are markdown.

## When to capture

Capture when **all** hold: the user asked an exploratory question (how or
where something works, why a choice was made, which approach is better);
the answer took synthesis, not a one-line lookup; and it's worth keeping.
Skip when you ran edits or commits (commits capture those), you're still
asking a clarifying question, or it was a single-file lookup. `/note`,
"save this" or "add to the wiki" always capture. Every thread captures
the same way: `oxplow.knowledge.write_page` is open to every thread.

## Find before you create

Don't fragment. Look for an existing topic page first: `oxplow_query_sql` over
`v_knowledge_page` (`slug`, `title`, `excerpt`, `stale_ref_count`), then
`oxplow_search` with `resources: ["oxplow/wiki"]`. If a relevant page exists, **append a
dated section** to it; create a new page only when none fits.
`references/search-existing.md` has the full lookup, including which
pages already cite a file.

## Slug, title, body

- Slug: kebab-case, ≤50 chars, topic-shaped (`stop-hook-pipeline`), never
  dated: one page per topic. Title: `# <Title>` on the first line.
- New page: an overview paragraph. Each addition: a `## <yyyy-mm-dd> —
  <focus>` section, ending with `Files referenced: [[src/foo.ts]], …`.
- Backticks are for identifiers, types, shell commands and config keys;
  a clickable path is a wikilink.

## Wikilinks — the one way to name an oxplow object

Write files, commits, tasks and pages as `[[…]]` links, never as plain
text: the renderer resolves the target to its title and the backend
records a backlink. This is the rule for every skill and every body you
write (task bodies, summaries, wiki pages).

- `[[src/foo.ts]]` file (`:42` for a line, `|label` for custom text)
- `[[dir:src/components]]` directory
- `[[abc1234]]` / `[[git:abc1234]]` commit
- `[[tsk<n>]]` task: always the `tsk` prefix, never `#<n>`
- `[[some-other-note]]` wiki page by slug

Never write `@<version>` on a link; freshness is tracked per reference.

## Writing the page

One command writes the row, the links and the file; don't write the
file yourself: `run_command oxplow.knowledge.write_page { slug, body,
verified_files, removed_files }`. Every `[[link]]` must resolve (a dangling
one is refused and named). `verified_files` are files you re-read against
this body; `removed_files` are files you dropped. Open
`references/write-page.md` for the mechanics, `oxplow.knowledge.link` /
`delete_page`, declaring the page in an effort report, a project with no
knowledge store, and folding in Explore subagent notes.

## Diagrams

Notes render ```mermaid fences. Draw when a relationship is clearer drawn
than described (hierarchy `graph TD`, lifecycle `stateDiagram-v2`, flow
`sequenceDiagram`); keep it to ~12 nodes and say in prose what to look at.
The oxplow-mermaid skill carries the syntax rules.
