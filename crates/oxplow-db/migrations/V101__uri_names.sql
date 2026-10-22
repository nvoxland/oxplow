-- A URI is called a URI (`.context/resources.md`): the columns, payload
-- fields and command inputs that said `ref` for one say `uri`. Wiki
-- citations (file paths) say `files`; a claim's evidence, which can be
-- the literal `reviewer`, says `evidence`.

-- Tables. SQLite carries each column's indexes along.
ALTER TABLE work_item RENAME COLUMN ref TO uri;
ALTER TABLE work_item RENAME COLUMN parent_ref TO parent_uri;
ALTER TABLE work_item_comment RENAME COLUMN ref TO uri;
ALTER TABLE work_item_link RENAME COLUMN from_ref TO from_uri;
ALTER TABLE work_item_link RENAME COLUMN to_ref TO to_uri;
ALTER TABLE task_link RENAME COLUMN to_ref TO to_uri;
ALTER TABLE fact_subject RENAME COLUMN ref TO uri;
ALTER TABLE search_entry RENAME COLUMN ref_id TO uri;
ALTER TABLE claim RENAME COLUMN evidence_ref TO evidence;
ALTER TABLE bookmark RENAME COLUMN ref TO uri;
ALTER TABLE symbol RENAME COLUMN ref TO uri;
ALTER TABLE change_function RENAME COLUMN ref TO uri;
ALTER TABLE wiki_page RENAME COLUMN file_refs_json TO files_json;
ALTER TABLE wiki_page RENAME COLUMN dir_refs_json TO dirs_json;

-- Indexes named for the old columns, and the one over a payload path.
DROP INDEX work_item_comment_ref;
CREATE INDEX work_item_comment_uri ON work_item_comment (uri);
DROP INDEX idx_symbol_ref;
CREATE UNIQUE INDEX idx_symbol_uri ON symbol (stream_id, uri);
DROP INDEX event_log_work_item_ref;

-- Stored events at their new versions: `work_item.recorded@3` names the
-- item `uri` and its parent `parent_uri`; `knowledge.page.recorded@2`
-- names the page `uri` and what it links `uris`.
UPDATE event_log
   SET payload = json_remove(json_set(payload, '$.item.uri', json(payload -> '$.item.ref')),
                             '$.item.ref'),
       v = 3
 WHERE type = 'work_item.recorded' AND v = 2;
UPDATE event_log
   SET payload = json_remove(json_set(payload, '$.item.parent_uri',
                                      json(payload -> '$.item.parent_ref')),
                             '$.item.parent_ref')
 WHERE type = 'work_item.recorded' AND json_type(payload, '$.item.parent_ref') IS NOT NULL;
UPDATE event_log
   SET payload = json_remove(json_set(payload, '$.page.uri', json(payload -> '$.page.ref'),
                                      '$.page.uris', json(payload -> '$.page.refs')),
                             '$.page.ref', '$.page.refs'),
       v = 2
 WHERE type = 'knowledge.page.recorded' AND v = 1;
CREATE INDEX event_log_work_item_uri
    ON event_log (json_extract(payload, '$.item.uri'), seq);

-- What runs later: an undo's inverse, a proposal not yet decided. Each
-- renamed input key, where it's there.
UPDATE command_audit
   SET inverse_json = json_remove(json_set(inverse_json, '$.input.uri',
                                           json(inverse_json -> '$.input.ref')),
                                  '$.input.ref')
 WHERE json_type(inverse_json, '$.input.ref') IS NOT NULL;
UPDATE command_proposal
   SET input_json = json_remove(json_set(input_json, '$.uri', json(input_json -> '$.ref')),
                                '$.ref')
 WHERE decision = 'pending' AND json_type(input_json, '$.ref') IS NOT NULL;
UPDATE command_audit
   SET inverse_json = json_remove(json_set(inverse_json, '$.input.parent_uri',
                                           json(inverse_json -> '$.input.parent_ref')),
                                  '$.input.parent_ref')
 WHERE json_type(inverse_json, '$.input.parent_ref') IS NOT NULL;
UPDATE command_proposal
   SET input_json = json_remove(json_set(input_json, '$.parent_uri', json(input_json -> '$.parent_ref')),
                                '$.parent_ref')
 WHERE decision = 'pending' AND json_type(input_json, '$.parent_ref') IS NOT NULL;
UPDATE command_audit
   SET inverse_json = json_remove(json_set(inverse_json, '$.input.evidence',
                                           json(inverse_json -> '$.input.evidence_ref')),
                                  '$.input.evidence_ref')
 WHERE json_type(inverse_json, '$.input.evidence_ref') IS NOT NULL;
UPDATE command_proposal
   SET input_json = json_remove(json_set(input_json, '$.evidence', json(input_json -> '$.evidence_ref')),
                                '$.evidence_ref')
 WHERE decision = 'pending' AND json_type(input_json, '$.evidence_ref') IS NOT NULL;
UPDATE command_audit
   SET inverse_json = json_remove(json_set(inverse_json, '$.input.files',
                                           json(inverse_json -> '$.input.refs')),
                                  '$.input.refs')
 WHERE json_type(inverse_json, '$.input.refs') IS NOT NULL;
UPDATE command_proposal
   SET input_json = json_remove(json_set(input_json, '$.files', json(input_json -> '$.refs')),
                                '$.refs')
 WHERE decision = 'pending' AND json_type(input_json, '$.refs') IS NOT NULL;
UPDATE command_audit
   SET inverse_json = json_remove(json_set(inverse_json, '$.input.verified_files',
                                           json(inverse_json -> '$.input.verified_refs')),
                                  '$.input.verified_refs')
 WHERE json_type(inverse_json, '$.input.verified_refs') IS NOT NULL;
UPDATE command_proposal
   SET input_json = json_remove(json_set(input_json, '$.verified_files', json(input_json -> '$.verified_refs')),
                                '$.verified_refs')
 WHERE decision = 'pending' AND json_type(input_json, '$.verified_refs') IS NOT NULL;
UPDATE command_audit
   SET inverse_json = json_remove(json_set(inverse_json, '$.input.removed_files',
                                           json(inverse_json -> '$.input.removed_refs')),
                                  '$.input.removed_refs')
 WHERE json_type(inverse_json, '$.input.removed_refs') IS NOT NULL;
UPDATE command_proposal
   SET input_json = json_remove(json_set(input_json, '$.removed_files', json(input_json -> '$.removed_refs')),
                                '$.removed_refs')
 WHERE decision = 'pending' AND json_type(input_json, '$.removed_refs') IS NOT NULL;
UPDATE command_proposal SET command = 'oxplow.knowledge.verify_files'
 WHERE decision = 'pending' AND command = 'oxplow.knowledge.verify_refs';
UPDATE command_audit
   SET inverse_json = json_set(inverse_json, '$.name', 'oxplow.knowledge.verify_files')
 WHERE json_extract(inverse_json, '$.name') = 'oxplow.knowledge.verify_refs';
