SELECT stream_id, language, path, severity, message, source, code,
       line, col, end_line, end_col, updated_at
FROM source('lsp_diagnostic')
