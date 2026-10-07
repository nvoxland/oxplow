-- Efforts are no longer recorded after the fact (oxplow.effort.report records a
-- summary on an effort that exists; .context/work-tracking.md), so their
-- events lose `retroactive`, and the v1 shapes go: every logged
-- effort.opened / closed / finished is a v2 payload without it (a v1
-- payload already read as v2).
UPDATE event_log
   SET payload = json_remove(payload, '$.retroactive'), v = 2
 WHERE type IN ('effort.opened', 'effort.closed', 'effort.finished')
   AND (v = 1 OR json_extract(payload, '$.retroactive') IS NOT NULL);
