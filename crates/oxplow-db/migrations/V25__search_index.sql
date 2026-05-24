-- Unified site-wide search index (FTS5 / BM25). See .context/data-model.md.
--
-- `search_entry` is the identity → rowid map; `search_fts` is the FTS5
-- index, rowid-aligned with `search_entry`, so a logical entity
-- (kind, ref_id, stream_id) can be updated or removed by a stable rowid.
-- Standalone FTS5 (stores its own copy of the text) because the sources
-- are heterogeneous — some rows come from other tables, some from disk.
--
--   kind:      task | comment | note | wiki | file
--   ref_id:    task id, comment id, note id, wiki slug, repo-relative path
--   stream_id: NULL = project-global (wiki); else the owning stream

CREATE TABLE search_entry (
    rowid     INTEGER PRIMARY KEY,
    kind      TEXT NOT NULL,
    ref_id    TEXT NOT NULL,
    stream_id TEXT
);

-- NULLs are distinct under a plain UNIQUE constraint, which would let
-- duplicate global rows through; collapse NULL → '' so identity is
-- enforced for stream-global entities too.
CREATE UNIQUE INDEX search_entry_identity
    ON search_entry (kind, ref_id, COALESCE(stream_id, ''));

CREATE INDEX search_entry_stream ON search_entry (stream_id);

CREATE VIRTUAL TABLE search_fts USING fts5(
    title,
    body,
    tokenize = 'porter unicode61',
    prefix = '2 3'
);
