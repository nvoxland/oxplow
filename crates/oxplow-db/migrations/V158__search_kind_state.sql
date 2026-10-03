-- tsk864: what each restated search kind last indexed, as a digest of its
-- entries. A restate whose entries hash the same writes nothing, so a
-- kind's index rebuilds only when what it holds changes — at start too.
CREATE TABLE search_kind_state (
    kind TEXT PRIMARY KEY,
    digest TEXT NOT NULL
);
