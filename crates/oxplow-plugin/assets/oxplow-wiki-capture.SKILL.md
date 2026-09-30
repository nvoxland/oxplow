---
name: oxplow-wiki-capture
description: Capturing non-trivial exploratory Q&A into wiki pages — codebase walkthroughs AND general synthesis (design rationale, comparisons, tradeoffs, recommendations, advice). The wiki is for any durable understanding worth keeping, not just code questions. Loads on mcp__oxplow__list_wiki_pages, search_wiki_pages, search_wiki_page_bodies, list_backlinks, get_wiki_page_metadata, knowledge.write_page, on /note, and when the user asks "how does X work", "where is X", "explain X", "trace X", "describe the architecture", "give me an overview", "summarize the codebase", "walk me through X", "why does/did/should X", "what's the difference between X and Y", "compare X and Y", "what are the tradeoffs", "should I use X or Y", "what's the best way to X", "rationale behind X", "advice on X", or says "save this" / "add a note" / "add to the wiki".
---

# Wiki pages — exploratory capture

The per-project wiki at `.oxplow/wiki/<slug>.md` is where durable
understanding lives: how subsystems work, why a design landed,
tradeoffs, recommendations, comparisons, follow-up analyses. **It is
NOT codebase-only** — any non-trivial exploratory Q&A belongs here,
including general design / process / rationale discussions. Bodies are
markdown; you author with the **Write** tool.

## When to capture

Capture when **all** are true:

- The user asked an exploratory question (how/where something works,
  why a choice was made, tradeoffs, which approach is better) — code or
  general, both qualify.
- The answer involved synthesis (weighed options, surfaced reasoning),
  not a one-line lookup.
- The synthesis is worth keeping.

Skip when: you ran edits/commits (commits capture those); you're still
asking a clarifying question (nothing to capture yet); or it was a
single-file lookup / one-line answer with no reasoning.

If the user types `/note` or says "save this" / "add to the wiki" /
"add a note", capture even if the trigger heuristic above wouldn't
otherwise fire.

## On a read-only thread

`knowledge.write_page` is open to every thread — capture exactly the
same way as on the writer thread. Don't punt the user's
exploration answer just because you can't edit code; the wiki is
where exploration goes regardless of writer status.

## Find before you create

Before writing, search for an existing topic note. Don't fragment.

1. `mcp__oxplow__search_wiki_pages` — title substring (cheap, scan first).
2. `mcp__oxplow__search_wiki_page_bodies` — content substring; catches
   notes that discuss the topic but aren't named after it.
3. `mcp__oxplow__list_backlinks` with `kind: "file"`, `id: <path>` —
   for each non-trivial file you read this turn, check whether an
   existing wiki page (or any other source) already references it.
   Filter the result to `source_kind == "wiki"` if you only want
   wiki backlinks.

If a clearly-relevant note exists, **append a new dated section** to
it. Only create a new note if no existing note fits.

## Slug + title conventions

- Slug: kebab-case, ≤50 chars, topic-shaped. Examples:
  `stop-hook-pipeline`, `wiki-page-storage`, `task-lifecycle`.
- Never include dates or turn ids in the slug — one page per topic.
- Title: `# <Title>` on the first line; human-readable.

## Body shape

```markdown
# <Title>

<one-paragraph overview if the note is new>

## <yyyy-mm-dd> — <focus>

<findings from this turn>

Files referenced: [[src/foo.ts]], [[src/bar/baz.ts]]
```

- Append entries with `## <date> — <focus>` headings.
- Inline file references as **bare wikilinks** with workspace-relative
  paths: `[[src/foo.ts]]`. **Never write `@<version>` literals** —
  freshness is tracked per `page_ref` row in the DB (the pin is
  stamped at write time and preserved across saves that don't touch the
  ref), and `@disk`/`@HEAD`/`@<sha>` are stripped on parse. You manage
  it explicitly with `verified_refs` / `removed_refs` (below).
- Backticks stay reserved for code-ish things (identifiers, types,
  shell commands, config keys). If it's a clickable path, wikilink it.

**Reference other oxplow objects by their id as a wikilink — never as
plain text.** The renderer resolves the id to the object's title and makes
it clickable; the backend records the reference as a backlink. Target
shapes:

- `[[src/foo.ts]]` file (`:42` for a line, `|label` for custom text)
- `[[dir:src/components]]` directory
- `[[abc1234]]` / `[[git:abc1234]]` commit
- `[[tsk42]]` **task** — renders as the task's title, links to the task
  (always the `tsk` prefix; **never** `[[#42]]`/`#42` — that isn't a ref)
- `[[some-other-note]]` wiki page by slug

Example: "The drag handler in [[src/ui/components/Tabs.tsx:88]] calls
`onDrop` after validating the target; this was fixed in [[tsk42]] and
wired up by [[src/ui/index.tsx]]."

## Write mechanics

1. For an existing page, read it first (`get_wiki_page_metadata` gives
   its `path`; Read the file) and merge your addition into its body.
2. Write the whole page with `mcp__oxplow__run_command`:
   `knowledge.write_page { slug, body, verified_refs, removed_refs }`
   (`title` optionally sets the `# ` heading). That one run writes the
   row, the links and `.oxplow/wiki/<slug>.md` — don't Write the file
   yourself. Every `[[link]]` must resolve: a dangling one is refused
   and named, so fix or drop it and run again.
   - `verified_refs`: files you re-read against this body; their
     freshness pins move to now. A file under a `[[dir:…]]` the page
     cites counts (it gets a pin of its own).
   - `removed_refs`: files you took out of the body.
   - Refs left in place without re-checking go in NEITHER list, keeping
     their pin.
   `knowledge.link { page, target }` adds one link under the page's
   Related heading; `knowledge.delete_page { slug }` deletes a page (the
   person confirms).
3. When you close the surrounding task, declare the page in
   `complete_task`'s `impacts`: `{ kind:"wiki", id:"<slug>",
   action:"created"|"updated" }` — this backlinks the task to the page.

## Diagrams — use mermaid

Notes render any ```mermaid fence as an inline SVG. **Reach for a
diagram whenever the relationship is clearer drawn than described** —
ASCII art is wasted effort here. Common picks: hierarchies/dependencies
→ `graph TD`; lifecycles → `stateDiagram-v2`; cross-component flows →
`sequenceDiagram`; phase-by-phase evolution → `timeline`. Wide
state-vs-condition matrices stay markdown tables — don't force a
diagram. Keep them ≤ ~12 nodes (split if crowded) and pair each with a
prose sentence saying what to look at. The [[oxplow-mermaid]] skill
auto-loads when you write a fence and carries the syntax rules.

## Folding in Explore findings

If this turn dispatched query subagents (`oxplow__delegate_query` →
`record_query_finding`), call `mcp__oxplow__list_thread_notes` and
incorporate their findings into the wiki page rather than discarding
them. Subagent notes are otherwise invisible — the wiki is where they
become durable.
