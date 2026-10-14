-- The symbol index comes from oxplow's own tree-sitter extraction, not
-- the language servers (.context/language-support.md "Symbols"): a ref
-- names the current symbol (`symbol:<path>/<id>`, no snapshot), so it is
-- unique per stream; a callable in an overloading language carries its
-- signature. The rows are rebuilt by the boot backfill.
DELETE FROM symbol;
DELETE FROM symbol_capture;
DROP INDEX idx_symbol_ref;
CREATE UNIQUE INDEX idx_symbol_ref ON symbol(stream_id, ref);
ALTER TABLE symbol ADD COLUMN signature TEXT;
ALTER TABLE symbol_capture DROP COLUMN files_over_budget;
ALTER TABLE symbol_capture DROP COLUMN files_without_server;
