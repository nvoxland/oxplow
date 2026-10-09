# Wiki pages

Oxplow's built-in knowledge store keeps per-project markdown pages at
`.oxplow/wiki/`, indexed in SQLite, with first-class backlinks. A project
can choose another knowledge implementation or none in Settings →
Capabilities. With no store active, knowledge writes are not retained.
Wiki pages are how durable understanding lands — codebase
walkthroughs, design rationale, comparisons, recommendations,
the "why we did it this way" stuff that doesn't belong in a
commit message.

## What wiki pages are for

- **Synthesis.** Anything an exploratory Q&A produced — "how
  does X work", "trace this codepath", "compare A and B",
  "should we do this or that".
- **Decisions you'll need later.** "We picked option B because
  option A would have broken the migration."
- **Context the agent should consult.** Wiki pages are
  first-class references — searchable by title and body,
  linkable from tasks, surfaced via backlinks.

They are *not* the right place for:

- Personal todos — that's the work queue.
- Per-task detail — that belongs inline in the task description.

## Editing

Wiki pages render and edit in the same surface — there's no
view/edit toggle. The editor is Tiptap, configured to preserve
everything the read view supports: a `MermaidBlock` node that
shows the rendered SVG when the caret is outside the block and
the raw source when the caret enters; custom link handling that
keeps `file:` / `dir:` / `gitcommit:` schemes through Tiptap's
URL sanitizer; and a wikilink round-trip pass so `[[ ]]` syntax
on disk parses to clickable markdown links in the editor and
re-serializes back to `[[ ]]` on save (with bare vs labeled
forms preserved).

Saves are debounced while you type and flushed on blur.

## Wikilinks

Type `[[page slug]]` in any wiki page. The renderer rewrites
them into clickable links:

| Form | Resolves to |
|---|---|
| `[[some-slug]]` | Another wiki page |
| `[[src/path/to/file.ts]]` | Opens the file in an editor tab |
| `[[src/foo.ts:42]]` | Same, jumps to line 42 |
| `[[abc1234]]` | Bare 7–40 char hex → git commit page |
| `[[git:abc1234]]` | Same, explicit git: prefix |
| `[[some-slug\|display text]]` | Override the link text |

The reference parser picks paths and SHAs out of `[[ ]]` so
backlinks and freshness work without any extra markup.

## Backlinks

Every page (wiki page, task, file, finding) has a
Backlinks panel. It surfaces every other record that points at
it — wiki pages that mention this file, tasks whose
touched-files list includes this file, findings that reference
it, etc.

The index is computed cross-kind from plain data slices (wiki
bodies, task touched files, findings) — no manual upkeep.

## Agent capture

The wiki-capture skill guides agents to retain useful exploratory Q&A:
walkthroughs, rationale, comparisons, and recommendations. The agent
searches existing pages first, then creates or updates one through
`oxplow.knowledge.write_page`. The command maintains the page, links,
and freshness references together. With the built-in store it also
writes the markdown file.

This works on non-writer threads too: recording understanding is
independent of permission to edit the stream's worktree.

## Resyncing

With the built-in store, the file watcher detects external edits and
updates the index. Agents should use the write command so reference
validation and freshness handling occur with the write.

