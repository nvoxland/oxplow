-- Knowledge pages are kept in several stores at once (oxwiki, an
-- extension's), each page named by its store and the store's own id:
-- `oxplow:oxplow/knowledge?id=<id>&store=<store>`. The record keeps every
-- store's pages; `v_knowledge_page` shows the enabled stores'.
--
-- Nothing of the wiki's record is carried over: its pages, their links,
-- visits, comments, search entries and events go (the files under
-- `.oxplow/wiki/` are removed by hand). Migrations run with foreign keys
-- off, so nothing cascades: every child goes by name.

CREATE TABLE knowledge_page (
    uri             TEXT PRIMARY KEY,
    store           TEXT NOT NULL,
    id              TEXT NOT NULL,
    -- One of the store's kinds; NULL: its first.
    kind            TEXT,
    title           TEXT NOT NULL,
    -- Where the person opens it in the store's own app.
    url             TEXT,
    author          TEXT,
    body            TEXT NOT NULL DEFAULT '',
    body_hash       TEXT NOT NULL DEFAULT '',
    excerpt         TEXT NOT NULL DEFAULT '',
    body_size_bytes INTEGER NOT NULL DEFAULT 0,
    files_json      TEXT NOT NULL DEFAULT '[]',
    dirs_json       TEXT NOT NULL DEFAULT '[]',
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    UNIQUE (store, id)
) STRICT;
CREATE INDEX idx_knowledge_page_updated ON knowledge_page(updated_at DESC);

-- When a thread last saw a page (an agent wrote or read it): what makes a
-- later change by someone else news to it.
CREATE TABLE knowledge_page_touch (
    thread_id    INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    uri          TEXT NOT NULL REFERENCES knowledge_page(uri) ON DELETE CASCADE,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY (thread_id, uri)
) STRICT;

-- An agent's reads of a page (`knowledge.page.read`): through a tool, or a
-- file of a store's folder. A person's opens are their page visits.
CREATE TABLE knowledge_page_read (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    uri       TEXT NOT NULL,
    thread_id INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    turn_id   INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL,
    via       TEXT NOT NULL CHECK (via IN ('tool', 'file')),
    at        TEXT NOT NULL
) STRICT;
CREATE INDEX idx_knowledge_page_read_uri ON knowledge_page_read(uri, at DESC);

DROP TABLE wiki_page_thread_update;
DROP INDEX IF EXISTS idx_wiki_page_updated;
DROP TABLE wiki_page;

-- The wiki's events, and what was waiting on them.
CREATE TEMP TABLE v28_event (seq INTEGER PRIMARY KEY);
INSERT INTO v28_event
  SELECT seq FROM event_log
   WHERE type LIKE 'knowledge.page.%' OR type LIKE 'knowledge.note.%';
DELETE FROM event_dead_letter WHERE event_seq IN (SELECT seq FROM v28_event);
DELETE FROM event_log WHERE seq IN (SELECT seq FROM v28_event);
DROP TABLE v28_event;
-- Other events stay; they stop naming a wiki page.
UPDATE event_log
   SET subject = (SELECT json_group_array(j.value) FROM json_each(event_log.subject) j
                   WHERE j.value NOT LIKE 'oxplow:oxplow/wiki?%')
 WHERE subject LIKE '%oxplow:oxplow/wiki?%';

-- Links from and to wiki pages.
DELETE FROM page_ref
 WHERE source_resource = 'oxplow/wiki' OR target_resource = 'oxplow/wiki';
-- The files and directories a task's body names were recorded under the
-- wiki's types; they are a body's mentions.
UPDATE page_ref SET ref_type = 'file_mention' WHERE ref_type = 'wiki_file_ref';
UPDATE page_ref SET ref_type = 'dir_mention' WHERE ref_type = 'wiki_dir_ref';

-- Visits, bookmarks and usage of the wiki's pages.
DELETE FROM page_visit
 WHERE page_kind IN ('oxplow/wiki', 'wiki') OR page_id LIKE 'oxplow:oxplow/wiki?%';
DELETE FROM bookmark
 WHERE page_kind IN ('oxplow/wiki', 'wiki') OR uri LIKE 'oxplow:oxplow/wiki?%';
DELETE FROM usage_event WHERE kind = 'wiki';

-- The search index's wiki entries.
DELETE FROM search_fts WHERE rowid IN (SELECT rowid FROM search_entry WHERE kind = 'oxplow/wiki');
DELETE FROM search_entry WHERE kind = 'oxplow/wiki';

-- Comments on wiki pages, their messages first.
DELETE FROM comment_message WHERE comment_id IN
  (SELECT id FROM comment WHERE target_resource = 'oxplow/wiki');
DELETE FROM comment WHERE target_resource = 'oxplow/wiki';

-- Wiki pages added to a thread's agent context.
DELETE FROM thread_context_item WHERE source_uri LIKE 'oxplow:oxplow/wiki%';

-- The consumers that followed the one wiki are gone.
DELETE FROM event_dead_letter WHERE consumer IN ('knowledge.switch', 'wiki.attribution');
DELETE FROM event_consumer_checkpoint WHERE consumer IN ('knowledge.switch', 'wiki.attribution');

-- Knowledge was one chosen store (or none); every store is in use now, and
-- the next publication states them.
DELETE FROM capability_provider WHERE capability = 'knowledge';

-- A knowledge store's kinds of page (`[{ id, title, bucket }]`), as a task
-- list's stages are `stages_json`.
ALTER TABLE capability_provider ADD COLUMN kinds_json TEXT NOT NULL DEFAULT '[]';
