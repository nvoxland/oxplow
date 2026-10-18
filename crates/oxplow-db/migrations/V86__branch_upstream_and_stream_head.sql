-- How far a local branch has parted from its upstream, and where each
-- stream's workspace is (its HEAD, detached or not): both restated by
-- the refs scan, read as `v_branch` and `v_stream.head_sha`.
ALTER TABLE git_branch ADD COLUMN upstream TEXT;
ALTER TABLE git_branch ADD COLUMN ahead INTEGER NOT NULL DEFAULT 0;
ALTER TABLE git_branch ADD COLUMN behind INTEGER NOT NULL DEFAULT 0;

CREATE TABLE stream_head (
    stream_id INTEGER PRIMARY KEY,
    head_sha TEXT NOT NULL
);
