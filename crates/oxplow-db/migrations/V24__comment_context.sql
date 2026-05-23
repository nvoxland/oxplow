-- Generalize comment anchoring to the W3C Web Annotation model and
-- capture the typed, hierarchical context a selection sits in. See
-- .context/data-model.md.

-- `anchor_json` held an opaque per-surface position hint. It now holds a
-- W3C selectors array (TextQuoteSelector + TextPositionSelector + an
-- optional per-surface coordinate selector); rename it to say so. The
-- column stays opaque to the store — only the renderer parses selectors.
ALTER TABLE comment RENAME COLUMN anchor_json TO selectors_json;

-- The typed context the agent reads alongside the quote. Both are JSON
-- arrays of {"kind","id"} using the canonical page_ref vocabulary
-- (file / directory / wiki / task / git-commit / finding).
--
--  context_chain_json   — ancestor refs from the page's nested
--    context-node hierarchy, innermost→outermost, EXCLUDING the primary
--    target (e.g. for a file row under a commit: [{git-commit,sha}]).
--  referenced_refs_json — canonical refs found INSIDE the selection
--    (rendered links + inline mentions), so highlighting a filename
--    tells the agent it is a file.
ALTER TABLE comment ADD COLUMN context_chain_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE comment ADD COLUMN referenced_refs_json TEXT NOT NULL DEFAULT '[]';
