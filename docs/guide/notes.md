# Notes

The notes pane is a wiki-style scratchpad attached to a stream.
Notes are markdown, support `[[wiki-link]]` syntax for cross-
linking, and live in the SQLite database alongside work items
and snapshots.

## What notes are for

- **Decisions you'll need later.** "We picked option B because
  option A would have broken the migration."
- **Context the agent should consult.** Notes can be referenced
  by name from the agent via wiki-note MCP tools.
- **Working memory across turns.** Anything you'd otherwise put
  in a scratch file, but want to keep tied to the project.

They are *not* the right place for:

- Acceptance criteria — those go on the work item.
- Long-term project documentation — that's `.context/` or
  whatever your repo uses for durable docs.
- Personal todos — that's the work queue.

## Wiki links

Type `[[note name]]` in any note. If a note with that name
exists, the link resolves. If not, clicking it creates the note.
This is the same pattern as Obsidian, Roam, and similar tools —
it works for cross-referencing decisions, design sketches, and
reusable context.

## Backlinks

Each note shows a list of every other note that links *to* it.
Useful for finding related material without manual indexing.

## Agent access

The agent can call `wiki-note` MCP tools to:

- List notes.
- Read a note by name.
- Search notes by content.
- Append to a note.

Use this when you want the agent to have a stable reference for
recurring context. Example: a `coding-style` note that captures
your preferences once, then gets cited every time the agent
generates code.

## Resyncing

If you edit notes externally (e.g. by hand in another editor on
the SQLite-backed file), call **Resync** from the notes pane
context menu. This re-reads the underlying storage and refreshes
backlinks.

## Scope

Notes live per-project (not per-stream) by design — they're
useful exactly because they survive stream churn and persist
across the work that's actually shipping. The notes pane shows
the project's notes regardless of which stream you're viewing.
