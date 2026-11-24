-- A change's review check reads the run its analysis event caused
-- (`v_oxplow_review_check`): look it up by the event, not by scanning every
-- run of the collector.
CREATE INDEX contribution_run_by_cause ON contribution_run (cause_event_id, contribution);
