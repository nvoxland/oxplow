-- P6.C1: an agent's answers in a thread (ephemeral lenses, target §11.4).
-- An answer is either an existing lens shown with some params (`lens`)
-- or the answer's own lens spec (`spec`, JSON); keeping one writes it as
-- a private lens and records which (`kept_lens`).
CREATE TABLE thread_answer (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL,
    effort_id INTEGER REFERENCES effort(id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    lens TEXT,
    spec TEXT,
    params TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    kept_lens TEXT,
    CHECK ((lens IS NULL) <> (spec IS NULL))
);
CREATE INDEX idx_thread_answer_thread ON thread_answer(thread_id, id);
