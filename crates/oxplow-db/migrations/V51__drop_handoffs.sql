-- Handoffs are gone: work moves between threads as a context basket sent to
-- a thread's next prompt, or as a task assigned there. The table goes, with
-- what named a handoff: its refs (a turn's mention of one it delivered, its
-- body's links), its search entries, its events and the audit of the
-- `oxplow.handoff.*` commands (an undo naming one with it).
DROP TABLE handoff;
DELETE FROM table_generation WHERE name = 'handoff';

DELETE FROM page_ref
 WHERE source_resource = 'oxplow/handoff' OR target_resource = 'oxplow/handoff';

DELETE FROM search_fts
 WHERE rowid IN (SELECT rowid FROM search_entry WHERE kind = 'oxplow/handoff');
DELETE FROM search_entry WHERE kind = 'oxplow/handoff';

CREATE TEMP TABLE v51_event (seq INTEGER PRIMARY KEY);
INSERT INTO v51_event
  SELECT seq FROM event_log WHERE type IN ('handoff.sent', 'handoff.settled');
DELETE FROM event_dead_letter WHERE event_seq IN (SELECT seq FROM v51_event);
DELETE FROM effect_run WHERE event_seq IN (SELECT seq FROM v51_event);
DELETE FROM pending_run WHERE event_seq IN (SELECT seq FROM v51_event);
DELETE FROM event_log WHERE seq IN (SELECT seq FROM v51_event);
DROP TABLE v51_event;

CREATE TEMP TABLE v51_audit (id INTEGER PRIMARY KEY);
INSERT INTO v51_audit
  SELECT id FROM command_audit
   WHERE command LIKE 'oxplow.handoff.%'
      OR inverse_json LIKE '%"oxplow.handoff.%';
DELETE FROM effect_run
 WHERE audit_id IN (SELECT id FROM v51_audit)
    OR proposal_id IN (SELECT id FROM command_proposal
                        WHERE command LIKE 'oxplow.handoff.%'
                           OR audit_id IN (SELECT id FROM v51_audit));
DELETE FROM command_proposal
 WHERE command LIKE 'oxplow.handoff.%'
    OR audit_id IN (SELECT id FROM v51_audit);
UPDATE command_audit SET undone_by = NULL
 WHERE undone_by IN (SELECT id FROM v51_audit);
DELETE FROM command_audit WHERE id IN (SELECT id FROM v51_audit);
DROP TABLE v51_audit;
