-- The person's bookmarks (tsk1099): a page they starred, at one scope —
-- a thread's, a stream's or the whole project's. From a thread a ref is
-- bookmarked at most once across the scopes it sees (that thread, its
-- stream, the project): bookmarking it again moves it. `ref` is the
-- page's canonical ref (its tab id); `page_kind` its page kind, for the
-- icon. Every write is a `bookmark.*` command.
CREATE TABLE bookmark (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ref TEXT NOT NULL,
    page_kind TEXT NOT NULL,
    label TEXT,
    scope TEXT NOT NULL CHECK (scope IN ('thread', 'stream', 'project')),
    thread_id INTEGER REFERENCES threads(id) ON DELETE CASCADE,
    stream_id INTEGER REFERENCES streams(id) ON DELETE CASCADE,
    added_at TEXT NOT NULL,
    CHECK ((scope = 'thread') = (thread_id IS NOT NULL)),
    CHECK ((scope = 'stream') = (stream_id IS NOT NULL))
) STRICT;
CREATE UNIQUE INDEX bookmark_owner
    ON bookmark (ref, scope, coalesce(thread_id, 0), coalesce(stream_id, 0));
