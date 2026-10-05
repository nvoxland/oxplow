-- tsk1049: an effort's observations were stored newest first, so `seq` 0
-- was the latest while every reader took the highest `seq` as the latest.
-- They are stored oldest first now. Forgetting what each closed effort's
-- evidence was computed from, and when the asset last ran, rebuilds all of
-- it on the next start.
DELETE FROM effort_evidence_state;
DELETE FROM asset_state WHERE asset = 'effort_evidence';
