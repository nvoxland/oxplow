-- The event log as an outbox (tsk406; .context/target-architecture.md §5,
-- .context/data-model.md "event_log"). Written in the same transaction as
-- the state change it records, so state and log never disagree. The first
-- STRICT tables in the schema: every later spine table is STRICT too.
--
-- `seq` is delivery order (what consumer checkpoints hold); `id` is the
-- public identity (a UUIDv7, so it sorts by time) that other events and
-- the command audit point at. Anchors are nullable columns rather than a
-- JSON blob so per-anchor timelines are an indexed range scan.
CREATE TABLE event_log (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    id           TEXT    NOT NULL UNIQUE,
    type         TEXT    NOT NULL,
    v            INTEGER NOT NULL,
    at           TEXT    NOT NULL,
    source       TEXT    NOT NULL,
    stream_id    INTEGER,
    thread_id    INTEGER,
    effort_id    INTEGER,
    turn_id      INTEGER,
    snapshot_id  INTEGER,
    -- JSON array of canonical refs the event is about.
    subject      TEXT    NOT NULL,
    -- JSON, validated against `type@v`'s schema on append.
    payload      TEXT    NOT NULL,
    -- Reserved for forgettable payloads: the content hash of a body that
    -- lives in the content store and may be purged while the row stays.
    payload_hash TEXT,
    cause        TEXT,
    -- Emitter-derived key; a second append of the same occurrence fails.
    dedupe_key   TEXT    UNIQUE
) STRICT;

CREATE INDEX event_log_type_seq ON event_log (type, seq);
CREATE INDEX event_log_stream ON event_log (stream_id, seq) WHERE stream_id IS NOT NULL;
CREATE INDEX event_log_thread ON event_log (thread_id, seq) WHERE thread_id IS NOT NULL;
CREATE INDEX event_log_effort ON event_log (effort_id, seq) WHERE effort_id IS NOT NULL;
CREATE INDEX event_log_snapshot ON event_log (snapshot_id, seq) WHERE snapshot_id IS NOT NULL;

-- At-least-once delivery: a consumer's checkpoint commits in the same
-- transaction as its writes.
CREATE TABLE event_consumer_checkpoint (
    consumer   TEXT    PRIMARY KEY,
    last_seq   INTEGER NOT NULL,
    updated_at TEXT    NOT NULL
) STRICT;

-- Poison events park here, visibly, carrying the error; the checkpoint
-- advances past them so one bad event never stalls the pump. A person
-- retries or discards them. Nothing is skipped silently.
CREATE TABLE event_dead_letter (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    consumer        TEXT    NOT NULL,
    event_seq       INTEGER NOT NULL REFERENCES event_log(seq),
    error           TEXT    NOT NULL,
    attempts        INTEGER NOT NULL DEFAULT 1,
    first_failed_at TEXT    NOT NULL,
    last_failed_at  TEXT    NOT NULL,
    state           TEXT    NOT NULL DEFAULT 'pending'
                    CHECK (state IN ('pending', 'retried', 'discarded')),
    UNIQUE (consumer, event_seq)
) STRICT;

-- Every command the bus runs (P1.8), with who ran it and how it went.
-- `inverse_json` is the undo; `undone_by` points at the audit row that
-- applied it.
CREATE TABLE command_audit (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    at           TEXT    NOT NULL,
    command      TEXT    NOT NULL,
    actor_kind   TEXT    NOT NULL CHECK (actor_kind IN ('human', 'agent', 'lens', 'system')),
    actor_id     TEXT,
    thread_id    INTEGER,
    input_json   TEXT    NOT NULL,
    outcome      TEXT    NOT NULL CHECK (outcome IN ('ok', 'denied', 'invalid', 'error')),
    error        TEXT,
    event_id     TEXT,
    inverse_json TEXT,
    undone_by    INTEGER
) STRICT;

-- `task_event` was the old per-task audit table. Nothing has written to it
-- since V1; task transitions log to event_log from P1.6 on.
DROP VIEW v_task_event;
DROP TABLE task_event;

-- Tab ids became canonical refs (tsk405/tsk418); the visit rows carry the
-- old ids and are history only, so they start fresh.
DELETE FROM page_visit;
