-- The tasks a request worked on, each link with the evidence it rests on.
-- Observed, never stated: nothing here opens, closes or moves a task.
--   basis  `prompt`    a person's prompt in the request named the task
--          `command`   the agent ran a task command on it in a turn
--          `assigned`  it was in progress and assigned to the thread when a
--                      turn of the request ended (the weakest)
--   turn_id  the turn the evidence was recorded in.
-- task_uri is not a foreign key: a task's tracker may drop it, and a link
-- may dangle (`.context/resources.md`).
CREATE TABLE request_task (
    request_id INTEGER NOT NULL REFERENCES request(id) ON DELETE CASCADE,
    task_uri TEXT NOT NULL,
    basis TEXT NOT NULL,
    turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL,
    at TEXT NOT NULL,
    PRIMARY KEY (request_id, task_uri, basis)
) WITHOUT ROWID;
CREATE INDEX idx_request_task_task ON request_task(task_uri);
