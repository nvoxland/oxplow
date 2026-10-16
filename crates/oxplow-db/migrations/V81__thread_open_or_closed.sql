-- A thread is open or closed (`closed_at`), nothing more: there is no
-- writer thread, so the stream-wide "one active thread" index and the
-- status column go (.context/agent-model.md "Why there is no writer
-- thread"). A thread may carry the person's mark instead, a colour on its
-- tab that changes nothing else; none is set here.

DROP INDEX idx_threads_one_active_per_stream;
ALTER TABLE threads DROP COLUMN status;
ALTER TABLE threads ADD COLUMN mark TEXT
    CHECK (mark IN ('amber', 'red', 'green', 'blue', 'violet'));
