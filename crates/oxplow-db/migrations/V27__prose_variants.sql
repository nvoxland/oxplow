-- Audience variants for agent-authored prose (developer / executive /
-- caveman). The canonical developer text stays in its existing column;
-- these nullable JSON blobs hold only the optional non-developer
-- variants, e.g. {"executive":"...","caveman":"..."}. NULL means
-- developer-only — every reader degrades to the developer text, so no
-- backfill is required. See crates/oxplow-domain/src/prose.rs.
--
-- Wired this migration: task.description_variants (phase 1).
-- Stubbed for later phases (columns added now to keep one migration):
--   * task_effort.summary_variants — effort summary variants (phase 3)
--   * comment.section_anchor       — heading-slug anchor so comments
--                                     re-display in the matching section
--                                     of any variant (phase 5)
ALTER TABLE task ADD COLUMN description_variants TEXT;
ALTER TABLE task_effort ADD COLUMN summary_variants TEXT;
ALTER TABLE comment ADD COLUMN section_anchor TEXT;
