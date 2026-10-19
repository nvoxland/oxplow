-- Each deployment of an extension: an immutable copy of its source in the
-- store (`.oxplow/runtime/extensions/<name>/<hash>/`), named by the hash of
-- the source it was copied from. `active` marks the one each extension runs
-- now; a bundled extension with no active row runs the copy that ships in
-- oxplow.
CREATE TABLE extension_deployment (
    name TEXT NOT NULL,
    hash TEXT NOT NULL,
    deployed_at TEXT NOT NULL,
    from_stream INTEGER,
    source TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (name, hash)
);

-- Where each listed extension's files come from replaces where it was
-- installed from (installing from git is gone).
ALTER TABLE extension_listing DROP COLUMN source_git;
ALTER TABLE extension_listing DROP COLUMN source_sha;
ALTER TABLE extension_listing ADD COLUMN deployment TEXT NOT NULL DEFAULT 'source';
ALTER TABLE extension_listing ADD COLUMN hash TEXT;
-- Sharing is git's: whether an extension's source is committed.
ALTER TABLE extension_listing DROP COLUMN sharing;
