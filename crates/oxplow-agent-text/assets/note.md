---
description: Keep what you just worked out as a knowledge page.
---

Keep what you learned in this conversation — or the exploration the
person points at — as a knowledge page, following the **`oxplow-knowledge`**
skill. `$ARGUMENTS`, when given, is the topic (or the page to add to).

1. Pick the topic: the argument, else what the recent conversation was
   about. A page is one topic, never a day.
2. **Find before you create.** Look across every store: `oxplow_query_sql`
   over `v_knowledge_page` (titles, excerpts), `oxplow_search` with
   `resources: ["oxplow/knowledge"]`, and the pages citing the files you
   read (`v_page_ref`). If one covers the topic, `oxplow_read` it and add a
   `## <yyyy-mm-dd> — <focus>` section to its body.
3. Write it with `run_command oxplow.knowledge.write_page { store?, id?,
   title, body, verified_files }`: name the `store` only when more than
   one is on; give an `id` to add to a page, or only a `title` for a new
   one (the store derives the id). Name files, tasks, commits and other
   pages as `[[…]]` links; list the files you re-read in `verified_files`.
4. If the write is refused, fix what it names (a dangling link, a store to
   name) and run it again. With no store on, say so once instead.
5. Tell the person in one line which page you wrote or added to.
