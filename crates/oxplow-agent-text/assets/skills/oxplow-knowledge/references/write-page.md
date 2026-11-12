# Writing a knowledge page

1. For an existing page, `oxplow_read { uri }` it and merge your addition
   into its body: a `## <yyyy-mm-dd> — <focus>` section, ending with
   `Files referenced: [[src/foo.ts]], …`. Refs that drifted since they were
   pinned are `v_knowledge_ref` rows with `stale = 1`;
   `oxplow_knowledge_drift { store?, id, path }` shows what changed in one.
2. Write the whole page with `run_command`:
   `oxplow.knowledge.write_page { store?, id?, title?, body, verified_files,
   removed_files }` (`title` sets the `# ` heading). The result names the
   page (`page`, its URI; `id`). Every `[[link]]` must resolve: fix or drop
   a dangling one and run again.
   - `verified_files`: files you re-read against this body; a file under a
     `[[dir:…]]` the page cites counts.
   - `removed_files`: files you took out of the body.
   - Refs left in place without re-checking go in NEITHER list, keeping
     their pin.
3. Beside it: `oxplow.knowledge.verify_files { store?, id, files? }`
   re-pins files you re-read without rewriting the page (any store, even
   one oxplow can't write); `oxplow.knowledge.link { store?, id, target }`
   adds one link under the page's Related heading;
   `oxplow.knowledge.delete_page { store?, id }` deletes one (the person
   confirms).

## A store that can't be written

A store whose `features.edit` is off (read-only: instruction files, a
harness's memory) refuses writes; read and verify its pages instead.

## No store on

With no knowledge store on, a write is refused: "no knowledge store is
enabled". Don't retry or work around it; say once that the page wasn't
kept, and that turning oxwiki on (Settings → Integrations → Knowledge
stores) or enabling an extension's store keeps pages.

## Diagrams

Hierarchies and dependencies → `graph TD`; lifecycles →
`stateDiagram-v2`; cross-component flows → `sequenceDiagram`. Pair each
diagram with a sentence saying what to look at.
