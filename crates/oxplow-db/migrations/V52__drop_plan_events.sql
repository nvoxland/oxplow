-- Plans an agent put to the person are no longer recorded: nothing showed
-- them. Their events go (`v_agent_plan` read them; it is gone with them).
CREATE TEMP TABLE v52_event (seq INTEGER PRIMARY KEY);
INSERT INTO v52_event SELECT seq FROM event_log WHERE type = 'agent.plan.proposed';
DELETE FROM event_dead_letter WHERE event_seq IN (SELECT seq FROM v52_event);
DELETE FROM effect_run WHERE event_seq IN (SELECT seq FROM v52_event);
DELETE FROM pending_run WHERE event_seq IN (SELECT seq FROM v52_event);
DELETE FROM event_log WHERE seq IN (SELECT seq FROM v52_event);
DROP TABLE v52_event;
