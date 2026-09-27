-- LSP diagnostics the language servers have published (tsk319). Live
-- state, like the servers themselves: cleared at boot and when a server
-- (re)starts, restated per file on each publishDiagnostics.
CREATE TABLE lsp_diagnostic (
    stream_id INTEGER NOT NULL,
    language TEXT NOT NULL,
    path TEXT NOT NULL,
    severity TEXT NOT NULL,
    message TEXT NOT NULL,
    source TEXT,
    code TEXT,
    line INTEGER NOT NULL,
    col INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    end_col INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_lsp_diagnostic_file ON lsp_diagnostic(stream_id, language, path);

CREATE VIEW v_diagnostic AS
SELECT stream_id, language, path, severity, message, source, code,
       line, col, end_line, end_col, updated_at
FROM lsp_diagnostic;
