-- Every stored ref becomes an oxplow URI (`.context/resources.md`):
-- `work_item:oxplow:tsk42` is `oxplow:oxplow/work_item?id=tsk42&provider=oxplow`.
-- `ref_to_uri` (the frozen table in `uri_migration`) reads each old ref; a
-- value it can't read is left as it was. Kinds give way to resources: the
-- `ref_kind` table goes (every resource is declared, `resource`), and
-- `page_ref` and `comment` keep one URI per end instead of a kind and an id.
--
-- An old ref's id escaped as the old grammar spelled it, for the tables
-- that kept a kind and a bare id apart:
--   replace(replace(replace(id, '%', '%25'), '@', '%40'), '#', '%23')

-- A resource of an extension's opens on one of its pages (interim, until
-- a renderer opens a URI as itself).
ALTER TABLE resource ADD COLUMN page TEXT;

-- page_ref: one URI per end, and the resource of each for its indexes.
CREATE TABLE page_ref_uri (
  source_uri      TEXT NOT NULL,
  target_uri      TEXT NOT NULL,
  source_resource TEXT NOT NULL,
  target_resource TEXT NOT NULL,
  ref_type        TEXT NOT NULL,
  source_extra    TEXT,
  local_snapshot_id INTEGER,
  closest_vcs_rev TEXT,
  vcs_rev_exact   INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (source_uri, target_uri, ref_type)
);
INSERT OR IGNORE INTO page_ref_uri
  (source_uri, target_uri, source_resource, target_resource, ref_type, source_extra,
   local_snapshot_id, closest_vcs_rev, vcs_rev_exact)
SELECT s, t, uri_resource(s), uri_resource(t), ref_type, source_extra, local_snapshot_id,
       closest_vcs_rev, vcs_rev_exact
FROM (
  SELECT ref_to_uri(p.source_kind || ':'
           || replace(replace(replace(p.source_id, '%', '%25'), '@', '%40'), '#', '%23'),
           (SELECT k.extension FROM ref_kind k WHERE k.kind = p.source_kind)) AS s,
         ref_to_uri(p.target_kind || ':'
           || replace(replace(replace(p.target_id, '%', '%25'), '@', '%40'), '#', '%23'),
           (SELECT k.extension FROM ref_kind k WHERE k.kind = p.target_kind)) AS t,
         p.*
  FROM page_ref p
)
WHERE uri_resource(s) IS NOT NULL AND uri_resource(t) IS NOT NULL;
DROP TABLE page_ref;
ALTER TABLE page_ref_uri RENAME TO page_ref;
CREATE INDEX idx_page_ref_target ON page_ref(target_uri);
CREATE INDEX idx_page_ref_source ON page_ref(source_uri);
CREATE INDEX idx_page_ref_target_resource ON page_ref(target_resource);
CREATE INDEX idx_page_ref_source_resource ON page_ref(source_resource);
CREATE INDEX idx_page_ref_snapshot ON page_ref(local_snapshot_id);

-- comment: its target one URI, its context chain and the URIs it links to
-- JSON arrays of URIs (they were `{kind, id}` objects).
CREATE TABLE comment_uri (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    thread_id INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    target_resource TEXT NOT NULL,
    target_uri TEXT NOT NULL,
    -- The selected text: the durable anchor + the context handed to
    -- the agent.
    quote TEXT NOT NULL,
    -- Opaque per-surface position hint (re-validated on load).
    selectors_json TEXT NOT NULL,
    -- 'note' (note-to-self) | 'followup' (wants the agent to act).
    intent TEXT NOT NULL DEFAULT 'note',
    -- 'open' | 'resolved'.
    status TEXT NOT NULL DEFAULT 'open',
    -- 1 when the quote could no longer be located in current content;
    -- still listed in the inbox, just without an inline highlight.
    orphaned INTEGER NOT NULL DEFAULT 0,
    author TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    -- Bumped on every new message; drives sorting + GC.
    last_activity_at TEXT NOT NULL,
    resolved_at TEXT,
    context_chain_json TEXT NOT NULL DEFAULT '[]',
    referenced_json TEXT NOT NULL DEFAULT '[]'
);
INSERT INTO comment_uri
  (id, stream_id, thread_id, target_resource, target_uri, quote, selectors_json, intent, status,
   orphaned, author, created_at, updated_at, last_activity_at, resolved_at, context_chain_json,
   referenced_json)
SELECT c.id, c.stream_id, c.thread_id, coalesce(uri_resource(t.uri), ''), t.uri, c.quote,
       c.selectors_json, c.intent, c.status, c.orphaned, c.author, c.created_at, c.updated_at,
       c.last_activity_at, c.resolved_at,
       (SELECT json_group_array(ref_to_uri(json_extract(e.value, '$.kind') || ':'
                || replace(replace(replace(json_extract(e.value, '$.id'),
                     '%', '%25'), '@', '%40'), '#', '%23')))
          FROM json_each(c.context_chain_json) e),
       (SELECT json_group_array(ref_to_uri(json_extract(e.value, '$.kind') || ':'
                || replace(replace(replace(json_extract(e.value, '$.id'),
                     '%', '%25'), '@', '%40'), '#', '%23')))
          FROM json_each(c.referenced_refs_json) e)
FROM comment c
JOIN (SELECT id, ref_to_uri(target_kind || ':'
             || replace(replace(replace(target_id, '%', '%25'), '@', '%40'), '#', '%23')) AS uri
        FROM comment) t ON t.id = c.id;
-- `comment_message` references `comment(id)`: rebuilt against the new table.
CREATE TABLE comment_message_uri (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    comment_id INTEGER NOT NULL REFERENCES comment_uri(id) ON DELETE CASCADE,
    -- Free-form, e.g. 'user' or 'agent'.
    author TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TEXT NOT NULL
);
INSERT INTO comment_message_uri SELECT * FROM comment_message;
DROP TABLE comment_message;
DROP TABLE comment;
ALTER TABLE comment_uri RENAME TO comment;
ALTER TABLE comment_message_uri RENAME TO comment_message;
CREATE INDEX idx_comment_stream ON comment(stream_id, status, last_activity_at DESC);
CREATE INDEX idx_comment_thread ON comment(thread_id, last_activity_at DESC);
CREATE INDEX idx_comment_target ON comment(target_uri);

-- The event log: subjects, and the payload fields that carry refs.
UPDATE event_log SET subject = ref_to_uri_in_json(subject)
 WHERE subject IS NOT NULL AND subject LIKE '%:%';
UPDATE event_log
   SET payload = ref_to_uri_in_json(payload, 'thread', 'turn', 'snapshot', 'parent', 'stream',
         'run', 'collector', 'work_item', 'target', 'job_run', 'job', 'contribution', 'page',
         'note', 'refs', 'effort', 'end_snapshot', 'start_snapshot', 'commit', 'comment',
         'outbound', 'answer', 'lens', 'proposal', 'to', 'session', 'handoff', 'claim',
         'decision', 'subject', 'from_thread', 'to_thread')
 WHERE type NOT IN ('command.executed', 'knowledge.comment.written', 'knowledge.comment.deleted');
-- `work_item.recorded` carries the item whole.
UPDATE event_log
   SET payload = json_set(payload,
         '$.item.ref', ref_to_uri(json_extract(payload, '$.item.ref')),
         '$.item.parent_ref', ref_to_uri(json_extract(payload, '$.item.parent_ref')))
 WHERE type = 'work_item.recorded' AND json_type(payload, '$.item.parent_ref') = 'text';
UPDATE event_log
   SET payload = json_set(payload, '$.item.ref', ref_to_uri(json_extract(payload, '$.item.ref')))
 WHERE type = 'work_item.recorded' AND json_type(payload, '$.item.ref') = 'text';
UPDATE event_log
   SET payload = json_set(payload, '$.item.links',
         json((SELECT json_group_array(json(json_set(l.value, '$.target',
                   ref_to_uri(json_extract(l.value, '$.target')))))
                 FROM json_each(payload, '$.item.links') l)))
 WHERE type = 'work_item.recorded' AND json_type(payload, '$.item.links') = 'array';
-- A comment's events name its target by one URI (v2).
UPDATE event_log
   SET payload = json_object(
         'comment', ref_to_uri(json_extract(payload, '$.comment')),
         'target', ref_to_uri(json_extract(payload, '$.target_kind') || ':'
           || replace(replace(replace(json_extract(payload, '$.target_id'),
                '%', '%25'), '@', '%40'), '#', '%23'))),
       v = 2
 WHERE type IN ('knowledge.comment.written', 'knowledge.comment.deleted') AND v = 1;

-- What runs later: an undo's inverse, a proposal not yet decided.
UPDATE command_audit
   SET inverse_json = json_set(inverse_json, '$.input',
         json(ref_to_uri_in_json(json_extract(inverse_json, '$.input'), 'ref', 'parent_ref',
           'work_item', 'target', 'thread', 'session', 'stream', 'collector', 'effort',
           'comment', 'handoff', 'note', 'claim', 'decision', 'evidence_ref', 'event', 'answer',
           'subject', 'action', 'from', 'to_thread', 'page', 'tab', 'parent')))
 WHERE inverse_json IS NOT NULL AND json_type(inverse_json, '$.input') = 'object';
UPDATE command_proposal
   SET input_json = ref_to_uri_in_json(input_json, 'ref', 'parent_ref', 'work_item', 'target',
         'thread', 'session', 'stream', 'collector', 'effort', 'comment', 'handoff', 'note',
         'claim', 'decision', 'evidence_ref', 'event', 'answer', 'subject', 'action', 'from',
         'to_thread', 'page', 'tab', 'parent')
 WHERE decision IS NULL;

-- Facts' subjects, and the resource each is of.
UPDATE OR IGNORE fact_subject SET ref = ref_to_uri(ref) WHERE ref NOT LIKE 'oxplow:%';
UPDATE fact_subject SET kind = uri_resource(ref) WHERE uri_resource(ref) IS NOT NULL;

-- Work items, and what names them.
UPDATE work_item SET ref = ref_to_uri(ref), parent_ref = ref_to_uri(parent_ref);
UPDATE work_item_comment SET ref = ref_to_uri(ref);
UPDATE OR IGNORE work_item_link
   SET from_ref = ref_to_uri(from_ref), to_ref = ref_to_uri(to_ref);
UPDATE work_item_link SET to_kind = coalesce(uri_resource(to_ref), to_kind);
UPDATE OR IGNORE task_link SET to_ref = ref_to_uri(to_ref);
UPDATE effort SET work_item = ref_to_uri(work_item) WHERE work_item IS NOT NULL;
UPDATE effort
   SET impacts_json = (SELECT json_group_array(json(CASE json_extract(i.value, '$.kind')
                         WHEN 'work_item' THEN json_set(i.value, '$.id',
                           ref_to_uri(json_extract(i.value, '$.id')))
                         ELSE i.value END))
                         FROM json_each(effort.impacts_json) i)
 WHERE impacts_json IS NOT NULL AND json_valid(impacts_json)
   AND json_type(impacts_json) = 'array';
UPDATE claim SET work_item = ref_to_uri(work_item), evidence_ref = ref_to_uri(evidence_ref);
UPDATE decision SET work_item = ref_to_uri(work_item);
UPDATE handoff SET work_item = ref_to_uri(work_item);

-- What a person keeps and visits.
UPDATE OR IGNORE bookmark SET ref = ref_to_uri(ref);
UPDATE page_visit SET page_id = ref_to_uri(page_id);
UPDATE OR IGNORE ui_preference
   SET scope = CASE
         WHEN scope LIKE 'kind:%' THEN 'resource:oxplow/' || substr(scope, 6)
         ELSE ref_to_uri(scope) END;
UPDATE catalog_prompt SET about = 'oxplow/' || about WHERE about IS NOT NULL AND about <> '';

-- Runs and symbols.
UPDATE extension_run SET contribution = ref_to_uri(contribution);
UPDATE OR IGNORE symbol SET ref = ref_to_uri(ref);
UPDATE OR IGNORE change_function SET ref = ref_to_uri(ref);

-- Restated from the extensions at every catalog load, under their new
-- URIs.
DELETE FROM contribution;

-- The search index: an entry is its resource and URI (it was a kind and an
-- id). An extension kind's entries go, with their index's state: its
-- resource's index rebuilds them from its view.
UPDATE search_entry
   SET ref_id = ref_to_uri(kind || ':'
         || replace(replace(replace(ref_id, '%', '%25'), '@', '%40'), '#', '%23'));
DELETE FROM search_fts WHERE rowid IN
  (SELECT rowid FROM search_entry WHERE uri_resource(ref_id) IS NULL);
DELETE FROM search_entry WHERE uri_resource(ref_id) IS NULL;
UPDATE search_entry SET kind = uri_resource(ref_id);
-- (An extension kind's state under its new name names no resource: the
-- next sync clears it.)
UPDATE asset_state SET asset = 'search:oxplow/' || substr(asset, 8) WHERE asset LIKE 'search:%';
DELETE FROM asset_failure WHERE asset LIKE 'search:%';

DROP TABLE ref_kind;
