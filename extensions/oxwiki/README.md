# oxwiki

oxplow's own knowledge store, as an extension in its own namespace
(`oxwiki`). It requires `oxplow-bundled` (which declares the knowledge
model it fills) and is on by default.

- **A page is a markdown file**, `.oxplow/oxwiki/<slug>.md`, named by its
  kebab-case slug. The folder is a watched folder: oxplow hears it and
  snapshots it though git ignores it.
- **Commands** `oxwiki.page.write { id?, title?, body }` and
  `oxwiki.page.delete { id }` write and delete the files, always in the
  primary checkout's folder (from any worktree), through
  `oxplow.extension_files.write`. A write whose `[[links]]` don't resolve is
  refused (`links.check`). `oxwiki.page.restore` is their undo. A write joins
  bundled's `oxplow/knowledge.new` point.
- **A collector** states every page file as a row of `oxplow/knowledge_record`
  (store `oxwiki`) on start and whenever a file changes, so a page edited by
  hand converges. Its title is the first heading.
- **Skills** `oxwiki` (writing a page) and the `/note` command.

Behavior is tested by the fixtures in `fixtures/`: `bun run test:ext oxwiki`.
