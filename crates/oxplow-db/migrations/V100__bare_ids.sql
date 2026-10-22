-- Fields named for one resource carry its bare id, not a URI
-- (`.context/resources.md` "Bare ids"): `thr3`, `eff12`, a snapshot's
-- number, a commit's sha. Work items and fields that can name anything
-- keep their URIs.

-- Event payloads: each changed type's fixed fields as keys, at its new
-- version.
UPDATE event_log SET payload = uri_keys_in_json(payload, 'job_run', 'job'), v = 2
 WHERE type = 'contribution.job.completed' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'proposal:int', 'supersedes:int'), v = 3
 WHERE type = 'command.proposed' AND v = 2;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'proposal:int'), v = 2
 WHERE type = 'command.approved' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'proposal:int'), v = 2
 WHERE type = 'command.declined' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'event', 'proposal:int'), v = 5
 WHERE type = 'effect.result' AND v = 4;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'stream', 'snapshot:int', 'parent:int'), v = 3
 WHERE type = 'snapshot.taken' AND v = 2;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'stream', 'snapshot:int', 'from', 'to'), v = 2
 WHERE type = 'vcs.head.moved' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'commit', 'stream'), v = 2
 WHERE type = 'vcs.commit.indexed' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'turn', 'thread'), v = 2
 WHERE type = 'agent.turn.started' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'turn', 'thread'), v = 3
 WHERE type = 'agent.turn.ended' AND v = 2;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'thread'), v = 3
 WHERE type = 'agent.session.started' AND v = 2;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'thread'), v = 2
 WHERE type = 'agent.session.ended' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'thread'), v = 2
 WHERE type = 'agent.tokens.reported' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'thread', 'turn'), v = 2
 WHERE type = 'agent.prompt.submitted' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'thread', 'turn'), v = 2
 WHERE type = 'agent.subagent.started' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'thread', 'turn'), v = 2
 WHERE type = 'agent.subagent.finished' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'thread', 'turn'), v = 2
 WHERE type = 'agent.peer_message.received' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'thread', 'turn'), v = 2
 WHERE type = 'agent.context.compacted' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'thread'), v = 2
 WHERE type = 'agent.status.changed' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'thread', 'turn'), v = 2
 WHERE type = 'agent.plan.proposed' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'run:int'), v = 3
 WHERE type = 'test.run.recorded' AND v = 2;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'effort', 'thread', 'start_snapshot:int'), v = 3
 WHERE type = 'effort.opened' AND v = 2;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'claim:int', 'effort'), v = 2
 WHERE type = 'effort.claim_verified' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'claim:int', 'effort'), v = 2
 WHERE type = 'effort.claim_dismissed' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'decision:int', 'effort'), v = 2
 WHERE type = 'effort.decision_reviewed' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'effort'), v = 2
 WHERE type = 'effort.decisions_inferred' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'collector'), v = 2
 WHERE type = 'collector.synced' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'thread', 'turn', 'snapshot:int'), v = 2
 WHERE type = 'thread.checkpoint' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'effort'), v = 2
 WHERE type = 'effort.linked' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'effort', 'commit'), v = 2
 WHERE type = 'effort.landed' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'effort'), v = 2
 WHERE type = 'effort.retitled' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'effort', 'end_snapshot:int'), v = 3
 WHERE type = 'effort.closed' AND v = 2;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'effort', 'end_snapshot:int'), v = 3
 WHERE type = 'effort.finished' AND v = 2;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'stream'), v = 2
 WHERE type = 'code.diagnostics.changed' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'page', 'snapshot:int'), v = 2
 WHERE type = 'knowledge.page.written' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'page'), v = 2
 WHERE type = 'knowledge.page.deleted' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'answer:int', 'thread', 'lens'), v = 2
 WHERE type = 'lens.shown' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'answer:int', 'lens'), v = 3
 WHERE type = 'lens.kept' AND v = 2;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'note', 'thread'), v = 3
 WHERE type = 'knowledge.note.written' AND v = 2;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'note', 'thread'), v = 3
 WHERE type = 'knowledge.note.deleted' AND v = 2;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'handoff', 'from_thread', 'to_thread'), v = 2
 WHERE type = 'handoff.sent' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'handoff', 'from_thread', 'to_thread'), v = 2
 WHERE type = 'handoff.settled' AND v = 1;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'comment'), v = 3
 WHERE type = 'knowledge.comment.written' AND v = 2;
UPDATE event_log SET payload = uri_keys_in_json(payload, 'comment'), v = 3
 WHERE type = 'knowledge.comment.deleted' AND v = 2;

-- What runs later: an undo's inverse, a proposal not yet decided. The
-- fixed input fields; claims, decisions and answers are numbers. (Job
-- intent keys are immutable: an old one keeps its embedded URI.)
UPDATE command_audit
   SET inverse_json = json_set(inverse_json, '$.input',
         json(uri_keys_in_json(json_extract(inverse_json, '$.input'), 'thread', 'stream',
           'session', 'effort', 'comment', 'handoff', 'note', 'event', 'collector', 'job_run',
           'from', 'from_thread', 'to_thread', 'claim:int', 'decision:int', 'answer:int')))
 WHERE inverse_json IS NOT NULL AND json_type(inverse_json, '$.input') = 'object';
-- A pending proposal's decision is 'pending' (V99 looked for NULL and
-- missed them): its old refs become URIs first, then the fixed fields keys.
UPDATE command_proposal
   SET input_json = uri_keys_in_json(
         ref_to_uri_in_json(input_json, 'ref', 'parent_ref', 'work_item', 'target', 'thread',
           'session', 'stream', 'collector', 'effort', 'comment', 'handoff', 'note', 'claim',
           'decision', 'evidence_ref', 'event', 'answer', 'subject', 'action', 'from',
           'to_thread', 'page', 'tab', 'parent'),
         'thread', 'stream', 'session', 'effort', 'comment', 'handoff', 'note', 'event',
         'collector', 'job_run', 'from', 'from_thread', 'to_thread', 'claim:int',
         'decision:int', 'answer:int')
 WHERE decision = 'pending';
