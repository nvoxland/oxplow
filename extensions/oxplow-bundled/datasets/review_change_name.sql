-- What a review's tabs call its change: a commit by its short sha and
-- subject, a request or a turn by the first line of its prompt, cut at a
-- word near 40 characters so the tab stays short.
WITH said AS (
  SELECT c.id AS change_id, c.kind, c.target,
         trim(CASE c.kind
           WHEN 'request' THEN (SELECT r.prompt FROM ref('request') r WHERE r.id = CAST(c.target AS INTEGER))
           WHEN 'turn' THEN (SELECT t.prompt FROM ref('agent_turn') t WHERE t.id = CAST(c.target AS INTEGER))
           WHEN 'commit' THEN (SELECT m.subject FROM ref('commit') m WHERE m.sha = c.target)
         END) AS text
  FROM ref('change') c
),
line AS (
  SELECT change_id, kind, target,
         trim(CASE WHEN instr(text, char(10)) > 0 THEN substr(text, 1, instr(text, char(10)) - 1) ELSE text END) AS text
  FROM said
),
short AS (
  SELECT change_id, kind, target,
         CASE WHEN length(text) <= 40 THEN text
              -- Back to the last space in the first 40 characters; a first
              -- word longer than that is cut where it reaches 40.
              ELSE coalesce(nullif(rtrim(rtrim(substr(text, 1, 40), replace(substr(text, 1, 40), ' ', '')), ' '), ''),
                            substr(text, 1, 40)) || '…' END AS text
  FROM line
)
SELECT change_id,
       CAST(CASE kind
         WHEN 'commit' THEN substr(target, 1, 7) || coalesce(' ' || nullif(text, ''), '')
         WHEN 'working' THEN 'uncommitted changes'
         ELSE coalesce(nullif(text, ''), 'change ' || change_id) END AS TEXT) AS name
FROM short
