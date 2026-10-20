-- command.executed@4 adds a custom component among its actor kinds; every
-- v3 event is a v4 one as it is (`.context/data-model.md`, V70's rule:
-- every event is stored at its type's newest version).
UPDATE event_log SET v = 4 WHERE type = 'command.executed' AND v = 3;
