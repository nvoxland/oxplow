-- Every language the loaded extensions declare, on or off: what
-- `v_language` lists. Restated whole with each published generation.
CREATE TABLE language_decl (
    id TEXT NOT NULL,
    title TEXT NOT NULL,
    extension TEXT NOT NULL,
    -- The declaring extension is on.
    enabled INTEGER NOT NULL,
    -- It has a grammar: code in it parses.
    analyzable INTEGER NOT NULL,
    -- JSON arrays: the file extensions, the aliases, the LSP language ids.
    extensions TEXT NOT NULL,
    aliases TEXT NOT NULL,
    lsp_ids TEXT NOT NULL,
    lsp_package TEXT,
    -- JSON object: `{ imports, merge, duplication, starter }`, each 0 or 1.
    facets TEXT NOT NULL,
    PRIMARY KEY (id, extension)
) WITHOUT ROWID;
