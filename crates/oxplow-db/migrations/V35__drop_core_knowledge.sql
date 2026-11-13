-- Core keeps no knowledge: a page is a row of the shared model
-- `oxplow/knowledge_record`, kept by a store (oxwiki is an extension with its
-- own folder), and what reads and touches a page is bundled's own models.
-- These are the tables core kept before: the record of every store's pages,
-- when a thread last saw one, and an agent's reads of one.
DROP TABLE knowledge_page_touch;
DROP TABLE knowledge_page_read;
DROP TABLE knowledge_page;
DELETE FROM table_generation
 WHERE name IN ('knowledge_page', 'knowledge_page_touch', 'knowledge_page_read');

-- The commands that wrote and read them, and those of the comments they were
-- part of, are gone (comments are `oxplow.comment.*`): their audit rows, what
-- ran from them and what proposed them go, an undo naming one with them.
CREATE TEMP TABLE v35_audit (id INTEGER PRIMARY KEY);
INSERT INTO v35_audit
  SELECT id FROM command_audit
   WHERE command LIKE 'oxplow.knowledge.%'
      OR inverse_json LIKE '%"oxplow.knowledge.%';
DELETE FROM effect_run
 WHERE audit_id IN (SELECT id FROM v35_audit)
    OR proposal_id IN (SELECT id FROM command_proposal
                        WHERE command LIKE 'oxplow.knowledge.%'
                           OR audit_id IN (SELECT id FROM v35_audit));
DELETE FROM command_proposal
 WHERE command LIKE 'oxplow.knowledge.%'
    OR audit_id IN (SELECT id FROM v35_audit);
UPDATE command_audit SET undone_by = NULL
 WHERE undone_by IN (SELECT id FROM v35_audit);
DELETE FROM command_audit WHERE id IN (SELECT id FROM v35_audit);
DROP TABLE v35_audit;

-- The events of the record and of comments under their old names, and what
-- was waiting on them.
CREATE TEMP TABLE v35_event (seq INTEGER PRIMARY KEY);
INSERT INTO v35_event
  SELECT seq FROM event_log
   WHERE type LIKE 'knowledge.page.%' OR type LIKE 'knowledge.comment.%';
DELETE FROM event_dead_letter WHERE event_seq IN (SELECT seq FROM v35_event);
DELETE FROM event_log WHERE seq IN (SELECT seq FROM v35_event);
DROP TABLE v35_event;
-- The consumers that followed them are gone.
DELETE FROM event_dead_letter
 WHERE consumer IN ('knowledge.attribution', 'knowledge.drift', 'knowledge.file_reads',
                    'knowledge.reads', 'knowledge.records', 'records.models');
DELETE FROM event_consumer_checkpoint
 WHERE consumer IN ('knowledge.attribution', 'knowledge.drift', 'knowledge.file_reads',
                    'knowledge.reads', 'knowledge.records', 'records.models');
-- A provider instance's reconcile markers.
DELETE FROM asset_state WHERE asset LIKE 'model_reconcile:%';

-- The edges core wrote for a page of its own record (a model row's are
-- `file_pin` / `dir_pin`).
DELETE FROM page_ref WHERE ref_type IN ('knowledge_file_ref', 'knowledge_dir_ref');

-- No knowledge capability, and no capability declares its own fields, task
-- stages, kinds of page or id pattern: those columns were written empty.
DELETE FROM capability_provider WHERE capability = 'knowledge';
ALTER TABLE capability_provider DROP COLUMN kinds_json;
ALTER TABLE capability_provider DROP COLUMN stages_json;
ALTER TABLE capability_provider DROP COLUMN fields_json;
ALTER TABLE capability_provider DROP COLUMN id_pattern;

-- A store is an extension's fill, never a provider instance's.
ALTER TABLE model_store_decl DROP COLUMN provider;
