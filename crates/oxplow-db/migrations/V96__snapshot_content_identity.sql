-- P2.1 (tsk423) — one content-hash identity space for snapshot files
-- (.context/target-architecture.md §6.1; .context/data-model.md
-- "snapshot + file_snapshot").
--
-- Until now `file_snapshot.blob_hash` was two things at once: the
-- storage ADDRESS (where the bytes live) and the content IDENTITY used to
-- diff. For `oxplow` rows those coincide (the xxh3-128 of the bytes names
-- the blob-store object), but a `git` row's address is a git blob OID —
-- a different hash space — so the same bytes captured once as `git` and
-- once as `oxplow` compared as different files.
--
-- `content_hash` is the identity: the xxh3-128 of the bytes, for every
-- storage class that has bytes. `blob_hash` stays the address. `oxplow`
-- rows copy it over; `git` rows are filled lazily (hashed from the git
-- odb the first time a comparison needs them — never eagerly, the whole
-- point of git-backed rows is not reading clean files at boot);
-- `oversize` / `deleted` rows have none.
--
-- `snapshot.tree_hash` is whole-tree identity: the xxh3-128 of the sorted
-- manifest of the reconstructed tree (see `oxplow_db::snapshot_tree`).
-- NULL on snapshots taken before this version.

ALTER TABLE file_snapshot ADD COLUMN content_hash TEXT;
UPDATE file_snapshot SET content_hash = blob_hash WHERE storage = 'oxplow';

ALTER TABLE snapshot ADD COLUMN tree_hash TEXT;

DROP VIEW v_snapshot;
CREATE VIEW v_snapshot AS
SELECT id, stream_id, created_at, git_commit, git_branch, tree_hash
  FROM snapshot;
