-- tsk799: a provider's read records an item only when it changed — a
-- record equal to the item's last `work_item.recorded` restates nothing
-- (and would echo a write back to whatever reacted to it). The read looks
-- up that last record by the item's ref.
CREATE INDEX event_log_work_item_ref
    ON event_log (json_extract(payload, '$.item.ref'), seq)
    WHERE type = 'work_item.recorded';
