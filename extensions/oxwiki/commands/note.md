---
description: Keep what you just worked out as a knowledge page.
---

Keep what you learned in this conversation — or the exploration the
person points at — as a knowledge page, following the **`oxplow-knowledge`**
(finding) and **`oxwiki`** (writing) skills. `$ARGUMENTS`, when given, is
the topic (or the page to add to).

1. Pick the topic: the argument, else what the recent conversation was
   about. A page is one topic, never a day.
2. **Find before you create.** Look across every store: `oxplow_query_sql`
   over `v_oxplow_knowledge_record` (titles, bodies), `oxplow_search` with
   `resources: ["oxplow/knowledge"]`, and the pages citing the files you
   read (`v_page_ref`). If one covers the topic, `oxplow_read` it and add a
   `## <yyyy-mm-dd> — <focus>` section to its body.
3. Write it with `run_command oxwiki.page.write { id?, title?, body }`:
   give an `id` to add to a page, or only a `title` for a new one (the id is
   made from it). Name files, tasks, commits and other pages as `[[…]]`
   links.
4. If the write is refused, fix what it names (a dangling link, a title
   that names an existing page) and run it again.
5. Tell the person in one line which page you wrote or added to.
