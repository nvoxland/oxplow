-- Comments are gone: a person's remark about a place is a note on a
-- context-basket item now (V44), delivered with it to the agent. The two
-- tables go, with what named a comment: its refs, its events and the audit of
-- the `oxplow.comment.*` commands (an undo naming one with it). A model's
-- child comments (a task's) are not these: their `comment_*` mentions stay.
DROP TABLE comment_message;
DROP TABLE comment;
DELETE FROM table_generation WHERE name IN ('comment', 'comment_message');

DELETE FROM page_ref
 WHERE source_resource = 'oxplow/comment' OR target_resource = 'oxplow/comment';

CREATE TEMP TABLE v45_event (seq INTEGER PRIMARY KEY);
INSERT INTO v45_event
  SELECT seq FROM event_log WHERE type IN ('comment.written', 'comment.deleted');
DELETE FROM event_dead_letter WHERE event_seq IN (SELECT seq FROM v45_event);
DELETE FROM effect_run WHERE event_seq IN (SELECT seq FROM v45_event);
DELETE FROM pending_run WHERE event_seq IN (SELECT seq FROM v45_event);
DELETE FROM event_log WHERE seq IN (SELECT seq FROM v45_event);
DROP TABLE v45_event;

CREATE TEMP TABLE v45_audit (id INTEGER PRIMARY KEY);
INSERT INTO v45_audit
  SELECT id FROM command_audit
   WHERE command LIKE 'oxplow.comment.%'
      OR inverse_json LIKE '%"oxplow.comment.%';
DELETE FROM effect_run
 WHERE audit_id IN (SELECT id FROM v45_audit)
    OR proposal_id IN (SELECT id FROM command_proposal
                        WHERE command LIKE 'oxplow.comment.%'
                           OR audit_id IN (SELECT id FROM v45_audit));
DELETE FROM command_proposal
 WHERE command LIKE 'oxplow.comment.%'
    OR audit_id IN (SELECT id FROM v45_audit);
UPDATE command_audit SET undone_by = NULL
 WHERE undone_by IN (SELECT id FROM v45_audit);
DELETE FROM command_audit WHERE id IN (SELECT id FROM v45_audit);
DROP TABLE v45_audit;
