-- A refused run (input that didn't fit, a caller denied) writes nothing
-- but its audit row, and nothing points at it: the retention sweep deletes
-- those past their window (`event_retention::REFUSED_AUDIT_DAYS`), oldest
-- first, through this index rather than scanning every run.
CREATE INDEX command_audit_refused ON command_audit (at)
    WHERE outcome IN ('invalid', 'denied');
