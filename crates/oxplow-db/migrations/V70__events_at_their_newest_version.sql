-- Every event is stored at its type's newest version, so reading the log
-- needs no upcast and the old versions' schemas go.

-- An old tool event named the tool by Claude Code's name and one `path`:
-- its kind comes from the name, and the path becomes `paths`. v2 is v3
-- without the optional `subagent`.
UPDATE event_log
   SET v = 3,
       payload = json_set(
           CASE WHEN json_type(payload, '$.path') IS NOT NULL
                THEN json_set(json_remove(payload, '$.path'),
                              '$.paths', json_array(json_extract(payload, '$.path')))
                ELSE payload END,
           '$.kind',
           CASE
               WHEN json_extract(payload, '$.tool') IN ('Write', 'Edit', 'MultiEdit', 'NotebookEdit') THEN 'edit'
               WHEN json_extract(payload, '$.tool') = 'Bash' THEN 'shell'
               WHEN json_extract(payload, '$.tool') IN ('Task', 'Agent') THEN 'subagent'
               WHEN json_extract(payload, '$.tool') = 'AskUserQuestion' THEN 'ask'
               WHEN json_extract(payload, '$.tool') = 'ExitPlanMode' THEN 'plan'
               WHEN json_extract(payload, '$.tool') = 'Read' THEN 'read'
               WHEN json_extract(payload, '$.tool') IN ('Grep', 'Glob', 'List') THEN 'search'
               WHEN json_extract(payload, '$.tool') IN ('WebFetch', 'WebSearch') THEN 'fetch'
               WHEN substr(json_extract(payload, '$.tool'), 1, 5) = 'mcp__' THEN 'mcp'
               ELSE 'other'
           END)
 WHERE type IN ('agent.tool.requested', 'agent.tool.finished') AND v = 1;
UPDATE event_log SET v = 3
 WHERE type IN ('agent.tool.requested', 'agent.tool.finished') AND v = 2;

-- An effect result before v3 was a live first attempt; v3's origins are
-- v4's.
UPDATE event_log
   SET v = 4, payload = json_set(payload, '$.attempt', 1, '$.origin', 'live')
 WHERE type = 'effect.result' AND v = 2;
UPDATE event_log SET v = 4 WHERE type = 'effect.result' AND v = 3;

-- The rest are their newest version as they are: an older version's
-- values are a subset of the newer one's.
UPDATE event_log SET v = 3 WHERE type = 'command.executed' AND v IN (1, 2);
UPDATE event_log SET v = 2 WHERE type = 'command.proposed' AND v = 1;
UPDATE event_log SET v = 2 WHERE type = 'agent.session.started' AND v = 1;
UPDATE event_log SET v = 2 WHERE type = 'agent.turn.ended' AND v = 1;
UPDATE event_log SET v = 2 WHERE type = 'snapshot.taken' AND v = 1;
UPDATE event_log SET v = 2 WHERE type = 'lens.kept' AND v = 1;
