-- A bookmark's `scope` says where it shows: `visibility_scope`, the name
-- the panel layout uses too. Bare "scope" is what a contribution may call
-- on the host. SQLite rewrites the CHECKs and the `bookmark_owner` index.
ALTER TABLE bookmark RENAME COLUMN scope TO visibility_scope;
