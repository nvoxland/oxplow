---
name: oxwiki
description: Writing a knowledge page in oxwiki, the project's own wiki of markdown files — oxwiki.page.write and oxwiki.page.delete, page ids, [[links]] that must resolve, dated sections, and what to do when a write is refused. Loads on the oxplow `run_command` tool with oxwiki.page.*, on /note, and when the user says "save this" / "add a note" / "keep this".
---

# Writing oxwiki pages

oxwiki keeps a page as a markdown file, `.oxplow/oxwiki/<id>.md`, whichever
checkout you work in (the command always writes the primary's folder). Find
before you create (the `knowledge` skill says how), then write:

`run_command oxwiki.page.write { id?, title?, body }` — the whole page.

- `id`: kebab-case and topic-shaped (`stop-hook-pipeline`), never dated.
  Give it to replace a page or add to one. Omit it on a new page and give a
  `title`: the id is made from it, and a title naming an existing page is
  refused (pass its `id`).
- `body`: markdown; start it with a `# ` heading (the page's title is its
  first heading). To add to a page, `oxplow_read` it first and merge a
  `## <yyyy-mm-dd> — <focus>` section into the body, ending with `Files
  referenced: [[src/foo.ts]], …`.
- Every `[[link]]` must resolve: a dangling one is refused and named; fix or
  drop it and run again. A bare `[[some-page]]` is an oxwiki page.
- The answer names the page (`id`, `page`, its uri). The page shows up as a
  row a moment later (the file is watched); a hand edit of the file does
  too, so never write the folder's files yourself.

`oxwiki.page.delete { id }` deletes a page (the person confirms); both undo.

## Links

Write files, commits, tasks and pages as `[[…]]` links, never as plain text:
`[[src/foo.ts]]` (`:42` a line, `|label` text), `[[dir:src/components]]`,
`[[abc1234]]` or `[[git:abc1234]]`, `[[tsk<n>]]`, `[[some-page]]`. Never write
`@<version>` on a file link: freshness is tracked per ref.

## Diagrams

Bodies render ```mermaid fences. Draw when a relationship is clearer drawn
than described; about 12 nodes; say in prose what to look at. The
oxplow-mermaid skill has the syntax.
