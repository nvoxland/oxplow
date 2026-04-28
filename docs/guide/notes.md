# Notes (the wiki)

Oxplow's wiki is a per-project folder of markdown files at
`.oxplow/notes/`, indexed in SQLite, with first-class
backlinks. Notes are how durable understanding lands — codebase
walkthroughs, design rationale, comparisons, recommendations,
the "why we did it this way" stuff that doesn't belong in a
commit message.

## What notes are for

- **Synthesis.** Anything an exploratory Q&A produced — "how
  does X work", "trace this codepath", "compare A and B",
  "should we do this or that".
- **Decisions you'll need later.** "We picked option B because
  option A would have broken the migration."
- **Context the agent should consult.** Notes are first-class
  references — searchable by title and body, linkable from work
  items, surfaced via backlinks.

They are *not* the right place for:

- Acceptance criteria — those go on the work item.
- Long-term subsystem documentation — that's `.context/` (see
  [the project's `.context/` convention](#context-vs-notes)).
- Personal todos — that's the work queue.

## Wikilinks

Type `[[note slug]]` in any note. The wiki renderer rewrites
them into clickable links:

| Form | Resolves to |
|---|---|
| `[[some-slug]]` | Another wiki note |
| `[[src/path/to/file.ts]]` | Opens the file in an editor tab |
| `[[src/foo.ts:42]]` | Same, jumps to line 42 |
| `[[abc1234]]` | Bare 7–40 char hex → git commit page |
| `[[git:abc1234]]` | Same, explicit git: prefix |
| `[[some-slug\|display text]]` | Override the link text |

The reference parser picks paths and SHAs out of `[[ ]]` so
backlinks and freshness work without any extra markup.

## Backlinks

Every page (note, work item, file, finding) has a Backlinks
panel. It surfaces every other record that points at it —
notes that mention this file, work items whose touched-files
list includes this file, findings that reference it, etc.

The index is computed cross-kind from plain data slices (note
bodies, work-item touched files, findings) — no manual upkeep.

## Agent capture

Non-trivial exploratory Q&A is captured into a wiki note
automatically. The runtime injects a `<wiki-capture-hint>` into
the agent's prompt when it sees patterns like "how does X
work", "explain X", "trace X", "why does X", "what's the
difference", "compare", "tradeoffs", "recommend". The agent
searches existing notes first, then writes / appends to
`.oxplow/notes/<slug>.md`, then calls `resync_note` to
re-baseline the index.

This works on read-only threads too — the write guard exempts
the notes directory because the wiki is research output, not
authored project change.

## What's exposed via MCP

The agent's note tools are metadata-only:

- `list_notes`, `search_notes`, `search_note_bodies`
- `find_notes_for_file` (backlinks for a file path)
- `get_note_metadata`, `resync_note`, `delete_note`

There is intentionally **no** create-note or update-note MCP
call. The agent writes bodies directly with its `Write` /
`Edit` tools on `.oxplow/notes/<slug>.md` (much cheaper than
round-tripping full bodies through tool args). The notes
watcher re-syncs metadata + body on every file event.

## Resyncing

If you edit notes externally (your own editor, a script), the
file watcher catches it and re-baselines. If you want to force
a sync immediately, call `resync_note` from the note's kebab
menu (or the agent does it after writes).

## .context/ vs notes

Many projects also keep durable subsystem docs in `.context/`
— short, opinionated guides for how each subsystem works.
These are committed to git and live in the repo, separate from
the per-project wiki. The agent reads them via
`get_subsystem_doc({ name })` and the **Subsystem docs** page.

Rule of thumb:

- **`.context/`** — durable, committed-to-git documentation of
  how a subsystem works.
- **`.oxplow/notes/`** — exploratory wiki capture, working
  memory, decisions, comparisons. Not committed by default.

Both are markdown; both are searchable; both surface as pages
in the rail's Pages directory.
