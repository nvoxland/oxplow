-- oxtask's own file. Every write to a row (from a command, from
-- `oxplow-dev task`, from a person's sqlite3) gets the next change number
-- from the triggers below, so the collector, which follows `seq`, sees it.
CREATE TABLE change_seq (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  v  INTEGER NOT NULL
);
INSERT INTO change_seq (id, v) VALUES (1, 0);

CREATE TABLE task (
  n          INTEGER PRIMARY KEY,
  -- Minted in the insert from the row number, so two writers can't collide
  -- and `INSERT … RETURNING id` names it.
  id         TEXT GENERATED ALWAYS AS ('tsk' || n) STORED UNIQUE,
  title      TEXT NOT NULL,
  body       TEXT,
  stage      TEXT NOT NULL DEFAULT 'backlog'
             CHECK (stage IN ('backlog', 'ready', 'in_progress', 'blocked', 'done', 'canceled')),
  parent     TEXT,
  priority   TEXT CHECK (priority IS NULL OR priority IN ('urgent', 'high', 'medium', 'low')),
  author     TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  closed_at  TEXT,
  seq        INTEGER NOT NULL DEFAULT 0,
  deleted    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX task_seq ON task (seq);

CREATE TABLE task_link (
  task_id TEXT NOT NULL,
  to_uri  TEXT NOT NULL,
  type    TEXT NOT NULL
          CHECK (type IN ('blocks', 'discovered_from', 'relates_to', 'duplicates', 'supersedes', 'replies_to', 'about')),
  seq     INTEGER NOT NULL DEFAULT 0,
  deleted INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (task_id, to_uri, type)
);

CREATE TABLE task_comment (
  n       INTEGER PRIMARY KEY,
  id      TEXT GENERATED ALWAYS AS ('cmt' || n) STORED UNIQUE,
  task_id TEXT NOT NULL,
  author  TEXT,
  at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
  body    TEXT NOT NULL,
  seq     INTEGER NOT NULL DEFAULT 0,
  deleted INTEGER NOT NULL DEFAULT 0
);

-- A task row: insert, update and (soft) delete all take the next number.
-- The guard `NEW.seq IS OLD.seq` keeps the trigger's own update (which
-- changes seq) from firing it again.
CREATE TRIGGER task_seq_insert AFTER INSERT ON task
BEGIN
  UPDATE change_seq SET v = v + 1;
  UPDATE task SET seq = (SELECT v FROM change_seq) WHERE n = NEW.n;
END;
CREATE TRIGGER task_seq_update AFTER UPDATE ON task WHEN NEW.seq IS OLD.seq
BEGIN
  UPDATE change_seq SET v = v + 1;
  UPDATE task SET seq = (SELECT v FROM change_seq) WHERE n = NEW.n;
END;

-- A link or a comment is part of its task's row (the collector restates the
-- whole set), so a change to one also takes the task's number.
CREATE TRIGGER task_link_seq_insert AFTER INSERT ON task_link
BEGIN
  UPDATE change_seq SET v = v + 1;
  UPDATE task_link SET seq = (SELECT v FROM change_seq)
    WHERE task_id = NEW.task_id AND to_uri = NEW.to_uri AND type = NEW.type;
  UPDATE task SET seq = (SELECT v FROM change_seq) WHERE id = NEW.task_id;
END;
CREATE TRIGGER task_link_seq_update AFTER UPDATE ON task_link WHEN NEW.seq IS OLD.seq
BEGIN
  UPDATE change_seq SET v = v + 1;
  UPDATE task_link SET seq = (SELECT v FROM change_seq)
    WHERE task_id = NEW.task_id AND to_uri = NEW.to_uri AND type = NEW.type;
  UPDATE task SET seq = (SELECT v FROM change_seq) WHERE id = NEW.task_id;
END;
CREATE TRIGGER task_comment_seq_insert AFTER INSERT ON task_comment
BEGIN
  UPDATE change_seq SET v = v + 1;
  UPDATE task_comment SET seq = (SELECT v FROM change_seq) WHERE n = NEW.n;
  UPDATE task SET seq = (SELECT v FROM change_seq) WHERE id = NEW.task_id;
END;
CREATE TRIGGER task_comment_seq_update AFTER UPDATE ON task_comment WHEN NEW.seq IS OLD.seq
BEGIN
  UPDATE change_seq SET v = v + 1;
  UPDATE task_comment SET seq = (SELECT v FROM change_seq) WHERE n = NEW.n;
  UPDATE task SET seq = (SELECT v FROM change_seq) WHERE id = NEW.task_id;
END;
