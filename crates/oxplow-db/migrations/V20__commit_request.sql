-- A commit links to the requests that made it, per path.
--
-- file_snapshot.git_oid: the git blob id of the bytes a row captured, so a
-- snapshot row is comparable with a commit's blob without reading bytes (and
-- survives Local History expiry). A git-backed row's address already is one;
-- an oxplow-stored row (blob_hash is an xxh3) gets it at capture. NULL for
-- deleted and oversize rows and for rows captured before this column.
ALTER TABLE file_snapshot ADD COLUMN git_oid TEXT;
UPDATE file_snapshot SET git_oid = blob_hash WHERE storage = 'git';

--   commit_request  the requests a commit's changes came from, one row per
--                   (commit, path, request).
--   basis           how the path is the request's: named (an edit call of
--                   one of its turns named it), reported (the harness said a
--                   shell call changed it), observed (it changed while the
--                   request ran, nothing else says).
--   confidence      sole (one request changed it since the previous commit
--                   that held it) or shared (several did).
CREATE TABLE commit_request (
    sha TEXT NOT NULL,
    path TEXT NOT NULL,
    request_id INTEGER NOT NULL REFERENCES request(id) ON DELETE CASCADE,
    basis TEXT NOT NULL,
    confidence TEXT NOT NULL,
    turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL,
    at TEXT NOT NULL,
    PRIMARY KEY (sha, path, request_id)
) WITHOUT ROWID;
CREATE INDEX idx_commit_request_request ON commit_request(request_id);
