-- One-shot marks (an advisory or nudge that fires once) are kept per
-- thread, and per effort within it: a hint fires once per thread even
-- when no effort is open (.context/extensions.md "Advisories").
CREATE TABLE once_mark (
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    effort_id INTEGER REFERENCES effort(id) ON DELETE CASCADE,
    mark      TEXT    NOT NULL,
    fired_at  TEXT    NOT NULL
) STRICT;
CREATE UNIQUE INDEX idx_once_mark ON once_mark(thread_id, coalesce(effort_id, 0), mark);

INSERT INTO once_mark (thread_id, effort_id, mark, fired_at)
SELECT e.thread_id, m.effort_id, m.mark, m.fired_at
  FROM effort_once_mark m JOIN effort e ON e.id = m.effort_id;

DROP TABLE effort_once_mark;
