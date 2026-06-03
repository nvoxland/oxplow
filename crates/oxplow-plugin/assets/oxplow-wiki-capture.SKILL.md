---
name: oxplow-wiki-capture
description: Capturing non-trivial exploratory Q&A into wiki pages — codebase walkthroughs AND general synthesis (design rationale, comparisons, tradeoffs, recommendations, advice). The wiki is for any durable understanding worth keeping, not just code questions. Loads on mcp__oxplow__list_wiki_pages, search_wiki_pages, search_wiki_page_bodies, list_backlinks, get_wiki_page_metadata, resync_wiki_page, record_wiki_page_update, on /note, and when the user asks "how does X work", "where is X", "explain X", "trace X", "describe the architecture", "give me an overview", "summarize the codebase", "walk me through X", "why does/did/should X", "what's the difference between X and Y", "compare X and Y", "what are the tradeoffs", "should I use X or Y", "what's the best way to X", "rationale behind X", "advice on X", or says "save this" / "add a note" / "add to the wiki".
---

# Wiki pages — exploratory capture

The per-project wiki at `.oxplow/wiki/<slug>.md` is where durable
understanding lives: how subsystems work, why a design landed, the
tradeoffs of an approach, recommendations, comparisons, follow-up
analyses. **It is NOT codebase-only** — any non-trivial exploratory
Q&A belongs here, including general design / process / rationale
discussions. The agent writes it as the user asks questions. Bodies
are markdown files; metadata is synced by a watcher, so you author
with the **Write** tool and call `mcp__oxplow__resync_wiki_page` to pin
freshness.

## When to capture

Capture when **all** are true:

- The user asked an exploratory question — how something works, where
  something lives, why a design choice was made, what the tradeoffs
  are, which approach is better, what the rationale behind X is, etc.
  Both code-flavored and general questions qualify; the wiki is for
  any durable understanding, not just code walkthroughs.
- The answer involved synthesis — pulled together facts, weighed
  options, surfaced reasoning — not just a one-line lookup.
- The synthesis is worth keeping. Trivial restatements aren't.

Skip when:

- You ran edits or commits — those are change events; commits already
  capture them.
- You're asking the user a clarifying question (no answer to capture
  yet — wait for the next turn).
- The exploration was a single-file lookup with no synthesis, or a
  one-line factual answer with no reasoning attached.

If the user types `/note` or says "save this" / "add to the wiki" /
"add a note", capture even if the trigger heuristic above wouldn't
otherwise fire.

## On a read-only thread

The write guard exempts `.oxplow/wiki/<slug>.md` — capture exactly
the same way as on the writer thread. Don't punt the user's
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

Files referenced: `src/foo.ts`, `src/bar/baz.ts`
```

- Append entries with `## <date> — <focus>` headings.
- Inline file references as **bare wikilinks** with workspace-relative
  paths: `[[src/foo.ts]]`. **No `@<version>` literals in body.**
  Version tracking lives in the database (`page_ref` row carries
  `local_snapshot_id` / `closest_git_version` / `git_version_exact`,
  stamped at write time and preserved on subsequent saves that
  don't touch the ref). Body stays prose; `@disk`, `@HEAD`,
  `@<sha>` are all stripped on parse.
- Backticks stay reserved for code-ish things (identifiers, types,
  shell commands, config keys) — `EditorPane`, `bun test`,
  `NODE_ENV`. If it's a path the reader should be able to click,
  use a wikilink, not backticks.

### Freshness is tracked in the row, not the body

Each `(wiki page → file)` edge in `page_ref` records the snapshot
the ref was captured against. The wiki sync preserves that pin
across saves that don't touch the ref — so editing one paragraph
doesn't pretend you re-verified every other link. You manage
freshness explicitly via `record_wiki_page_update` (see Write
mechanics below) with `verified_refs` and `removed_refs`.

Wikilink target shapes:

- `[[src/foo.ts]]` — file (bare, no version literal)
- `[[src/foo.ts:42]]` — file + line anchor
- `[[src/foo.ts|the foo helper]]` — custom display text
- `[[dir:src/components]]` — directory
- `[[abc1234]]` or `[[git:abc1234]]` — git commit reference
- `[[some-other-note]]` — link to another wiki page by slug

Example: "The drag handler in [[src/ui/components/Tabs.tsx:88]]
calls `onDrop` after validating the target. The entry point
[[src/ui/index.tsx]] wires it up. See
[[dir:src/ui/components]] for the rest of that surface."

## Write mechanics

1. Resolve the path: call `mcp__oxplow__get_wiki_page_metadata` (existing
   note) or `mcp__oxplow__list_wiki_pages` and use the returned `path`.
   For a brand-new slug, the path is
   `<projectDir>/.oxplow/wiki/<slug>.md`.
2. Use the **Write** tool to write/replace the file. (For appends to
   an existing note, Read first, then Write the merged body.)
3. Call `mcp__oxplow__record_wiki_page_update` with the slug,
   `verified_refs` (every file path you re-read against the new
   body during this edit), and `removed_refs` (file paths you
   intentionally removed from the page). Both lists are REQUIRED.
   Empty `[]` is allowed but deliberate: it means "I didn't
   re-check anything" / "I didn't remove anything." Refs left in
   place without re-checking go in NEITHER list and keep their
   existing freshness pin so the staleness signal stays honest.
   A `verified_ref` may be a file the body cites directly OR a file
   under a directory the body cites (`[[dir:…]]`) — so if you
   verified a fact against `crates/x/src/lib.rs` but the page only
   links `[[dir:crates/x]]`, list the specific file; it gets its own
   precise freshness pin (and survives future syncs while the dir
   ref stays).
4. When you close the surrounding task via `complete_task`,
   declare the wiki page in `impacts`:
   `{ kind: "wiki", id: "<slug>", action: "created" | "updated" }`.
   This produces a `page_ref` backlink from the task to the page,
   so the page's "referenced by" list points at the task that
   spawned it without anyone parsing the summary body.

## Diagrams — use mermaid

Notes render through `MarkdownView` with mermaid post-processing
enabled, so any ```mermaid fenced block becomes an inline SVG in
NoteTab. **Reach for a diagram whenever the relationship would be
clearer drawn than described.** ASCII art is wasted effort here —
write mermaid instead.

Strong signals that a diagram earns its keep:

- Entity hierarchies, table relationships, module dependencies
  → `graph TD` or `flowchart TD`
- State machines (statuses, lifecycles, transitions)
  → `stateDiagram-v2`
- Time-ordered request/response or event flows between components
  → `sequenceDiagram`
- Phase-by-phase evolution of a system over time
  → `timeline`
- Tabular state-vs-condition matrices that would be wide and ugly
  inline → leave as a markdown table; don't force a diagram

Keep diagrams small (≤ ~12 nodes); split into multiple diagrams under
sub-headings if a single one gets crowded. Always pair the diagram
with a prose sentence that says what to look at — readers skim
captions, not boxes.

### graph TD — hierarchy / relationship

```mermaid
graph TD
  Stream[streams] --> Thread[threads]
  Thread --> Task[task]
  Task --> Note[work_note]
  Task --> Effort[task_effort]
  Effort --> EffortFile[task_effort_file]
```

### stateDiagram-v2 — lifecycle

```mermaid
stateDiagram-v2
  [*] --> ready
  ready --> in_progress
  in_progress --> done
  in_progress --> blocked
  blocked --> in_progress
  done --> in_progress: reopen (redo)
  done --> archived
  in_progress --> canceled
  canceled --> archived
```

### sequenceDiagram — cross-component flow

```mermaid
sequenceDiagram
  participant CC as Claude Code
  participant Hook as PreToolUse hook
  participant RT as runtime
  CC->>Hook: tool_input { tool: "Edit", ... }
  Hook->>RT: POST /hook/PreToolUse
  RT-->>Hook: { permissionDecision: "deny", reason }
  Hook-->>CC: deny response
```

### timeline — phase-by-phase evolution

```mermaid
timeline
  title Hook transport evolution
  Phase 1 : --settings + shell forwarder : daemon mode
  Phase 2 : Claude Code plugin : http hooks : drop daemon
  Phase 3 : SessionStart workaround : session-id from any hook
  Phase 4 : handler logic accretes : Stop slimmed
```

Use mermaid's own syntax docs if you need a less common diagram type
(class, ER, gantt, pie, journey). The ```mermaid fence is the only
gating requirement; everything inside is forwarded to mermaid as-is.

## Folding in Explore findings

If this turn dispatched query subagents (`oxplow__delegate_query` →
`record_query_finding`), call `mcp__oxplow__get_thread_notes` and
incorporate their findings into the wiki page rather than discarding
them. Subagent notes are otherwise invisible — the wiki is where they
become durable.
