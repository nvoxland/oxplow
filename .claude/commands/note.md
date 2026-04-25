---
description: Capture the current code-exploration into a wiki note
---

# /note — capture exploration to the wiki

Take what you've learned in this turn (or the recent exploration the
user is referring to) and capture it as a wiki note in
`.oxplow/notes/<slug>.md`. Follow the **`oxplow-wiki-capture`** skill.

Concretely:

1. Pick a topic for the capture. If the user passed an argument, treat
   it as a hint at the topic/slug; otherwise infer from the recent
   conversation.
2. **Search before creating.** Run
   `mcp__oxplow__search_notes` (titles), `mcp__oxplow__search_note_bodies`
   (content), and `mcp__oxplow__find_notes_for_file` for any non-trivial
   files you read this turn. If a clearly-relevant existing note covers
   the topic, append to it; otherwise create a new one.
3. Decide append vs new. Append by adding a `## <yyyy-mm-dd> — <focus>`
   section at the end of the existing body.
4. **Write** the file with the Write tool. Slug is kebab-case, ≤50
   chars, topic-shaped (e.g. `stop-hook-pipeline`). Title is `# <Title>`
   on the first line. Reference files inline with backticked
   workspace-relative paths so the watcher picks them up as backlinks.
5. If the turn dispatched query subagents, call
   `mcp__oxplow__get_thread_notes` and fold their findings into the
   note rather than discarding them.
6. Call `mcp__oxplow__resync_note` with the slug to pin freshness.
7. Reply with one line confirming what landed: the slug + whether you
   appended or created.

If the recent context isn't actually exploration worth capturing, say
so plainly and stop — don't fabricate a note.
