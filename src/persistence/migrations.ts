import type { Logger } from "../core/logger.js";
import type { SqlDriver } from "./driver.js";

export interface Migration {
  version: number;
  name: string;
  up: (driver: SqlDriver) => void;
}

export const MIGRATIONS: Migration[] = [
  {
    version: 1,
    name: "initial schema",
    up: (db) => {
      db.exec(`
        CREATE TABLE streams (
          id TEXT PRIMARY KEY,
          title TEXT NOT NULL,
          summary TEXT NOT NULL,
          branch TEXT NOT NULL,
          branch_ref TEXT NOT NULL,
          branch_source TEXT NOT NULL,
          worktree_path TEXT NOT NULL,
          working_pane TEXT NOT NULL,
          talking_pane TEXT NOT NULL,
          working_session_id TEXT NOT NULL DEFAULT '',
          talking_session_id TEXT NOT NULL DEFAULT '',
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE runtime_state (
          id INTEGER PRIMARY KEY CHECK (id = 1),
          current_stream_id TEXT
        );

        CREATE TABLE batches (
          id TEXT PRIMARY KEY,
          stream_id TEXT NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
          title TEXT NOT NULL,
          status TEXT NOT NULL,
          sort_index INTEGER NOT NULL,
          pane_target TEXT NOT NULL,
          resume_session_id TEXT NOT NULL DEFAULT '',
          summary TEXT NOT NULL DEFAULT '',
          summary_updated_at TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE batch_selection (
          stream_id TEXT PRIMARY KEY REFERENCES streams(id) ON DELETE CASCADE,
          selected_batch_id TEXT
        );

        CREATE TABLE work_items (
          id TEXT PRIMARY KEY,
          batch_id TEXT NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
          parent_id TEXT REFERENCES work_items(id) ON DELETE CASCADE,
          kind TEXT NOT NULL,
          title TEXT NOT NULL,
          description TEXT NOT NULL DEFAULT '',
          status TEXT NOT NULL,
          priority TEXT NOT NULL,
          sort_index INTEGER NOT NULL DEFAULT 0,
          created_by TEXT NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          completed_at TEXT,
          deleted_at TEXT
        );

        CREATE TABLE work_item_links (
          id TEXT PRIMARY KEY,
          batch_id TEXT NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
          from_item_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
          to_item_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
          link_type TEXT NOT NULL,
          created_at TEXT NOT NULL,
          CHECK (from_item_id <> to_item_id)
        );

        CREATE TABLE work_item_events (
          id TEXT PRIMARY KEY,
          batch_id TEXT NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
          item_id TEXT REFERENCES work_items(id) ON DELETE CASCADE,
          event_type TEXT NOT NULL,
          actor_kind TEXT NOT NULL,
          actor_id TEXT NOT NULL,
          payload_json TEXT NOT NULL,
          created_at TEXT NOT NULL
        );

        CREATE INDEX idx_streams_branch ON streams(branch);
        CREATE INDEX idx_batches_stream_sort ON batches(stream_id, sort_index);
        CREATE INDEX idx_work_items_batch_parent ON work_items(batch_id, parent_id, sort_index);
        CREATE INDEX idx_work_items_batch_status ON work_items(batch_id, status, sort_index);
        CREATE INDEX idx_work_items_batch_deleted ON work_items(batch_id, deleted_at, sort_index);
        CREATE INDEX idx_work_links_batch_from ON work_item_links(batch_id, from_item_id);
        CREATE INDEX idx_work_links_batch_to ON work_item_links(batch_id, to_item_id);
        CREATE INDEX idx_work_events_batch_item ON work_item_events(batch_id, item_id, created_at);

        INSERT INTO runtime_state (id, current_stream_id) VALUES (1, NULL);
      `);
    },
  },
  {
    version: 2,
    name: "work_items.acceptance_criteria",
    up: (db) => {
      db.exec(`ALTER TABLE work_items ADD COLUMN acceptance_criteria TEXT;`);
    },
  },
  {
    version: 3,
    name: "agent_turn",
    up: (db) => {
      db.exec(`
        CREATE TABLE agent_turn (
          id TEXT PRIMARY KEY,
          batch_id TEXT NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
          work_item_id TEXT REFERENCES work_items(id) ON DELETE SET NULL,
          prompt TEXT NOT NULL,
          answer TEXT,
          session_id TEXT,
          started_at TEXT NOT NULL,
          ended_at TEXT
        );

        CREATE INDEX idx_agent_turn_batch ON agent_turn(batch_id, started_at DESC);
        CREATE INDEX idx_agent_turn_item ON agent_turn(work_item_id, started_at DESC);
      `);
    },
  },
  {
    version: 4,
    name: "batch_file_change",
    up: (db) => {
      db.exec(`
        CREATE TABLE batch_file_change (
          id TEXT PRIMARY KEY,
          batch_id TEXT NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
          turn_id TEXT REFERENCES agent_turn(id) ON DELETE SET NULL,
          work_item_id TEXT REFERENCES work_items(id) ON DELETE SET NULL,
          path TEXT NOT NULL,
          change_kind TEXT NOT NULL,
          source TEXT NOT NULL,
          tool_name TEXT,
          created_at TEXT NOT NULL
        );

        CREATE INDEX idx_batch_file_change_batch ON batch_file_change(batch_id, created_at DESC);
        CREATE INDEX idx_batch_file_change_turn ON batch_file_change(turn_id, created_at DESC);
      `);
    },
  },
  {
    version: 5,
    name: "work_items.batch_id nullable (backlog)",
    up: (db) => {
      // SQLite can't drop NOT NULL in place; rebuild the table and event table.
      // defer_foreign_keys lets us drop/recreate referenced tables inside the
      // transaction without tripping FK enforcement until commit.
      db.exec(`
        PRAGMA defer_foreign_keys = 1;

        CREATE TABLE work_items_new (
          id TEXT PRIMARY KEY,
          batch_id TEXT REFERENCES batches(id) ON DELETE CASCADE,
          parent_id TEXT REFERENCES work_items(id) ON DELETE CASCADE,
          kind TEXT NOT NULL,
          title TEXT NOT NULL,
          description TEXT NOT NULL DEFAULT '',
          acceptance_criteria TEXT,
          status TEXT NOT NULL,
          priority TEXT NOT NULL,
          sort_index INTEGER NOT NULL DEFAULT 0,
          created_by TEXT NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          completed_at TEXT,
          deleted_at TEXT
        );

        INSERT INTO work_items_new (
          id, batch_id, parent_id, kind, title, description, acceptance_criteria,
          status, priority, sort_index, created_by, created_at, updated_at, completed_at, deleted_at
        )
        SELECT
          id, batch_id, parent_id, kind, title, description, acceptance_criteria,
          status, priority, sort_index, created_by, created_at, updated_at, completed_at, deleted_at
        FROM work_items;

        DROP TABLE work_items;
        ALTER TABLE work_items_new RENAME TO work_items;

        CREATE INDEX idx_work_items_batch_parent ON work_items(batch_id, parent_id, sort_index);
        CREATE INDEX idx_work_items_batch_status ON work_items(batch_id, status, sort_index);
        CREATE INDEX idx_work_items_batch_deleted ON work_items(batch_id, deleted_at, sort_index);
        CREATE INDEX idx_work_items_backlog ON work_items(deleted_at, sort_index) WHERE batch_id IS NULL;

        CREATE TABLE work_item_events_new (
          id TEXT PRIMARY KEY,
          batch_id TEXT REFERENCES batches(id) ON DELETE CASCADE,
          item_id TEXT REFERENCES work_items(id) ON DELETE CASCADE,
          event_type TEXT NOT NULL,
          actor_kind TEXT NOT NULL,
          actor_id TEXT NOT NULL,
          payload_json TEXT NOT NULL,
          created_at TEXT NOT NULL
        );

        INSERT INTO work_item_events_new
          SELECT id, batch_id, item_id, event_type, actor_kind, actor_id, payload_json, created_at
          FROM work_item_events;

        DROP TABLE work_item_events;
        ALTER TABLE work_item_events_new RENAME TO work_item_events;

        CREATE INDEX idx_work_events_batch_item ON work_item_events(batch_id, item_id, created_at);
        CREATE INDEX idx_work_events_item ON work_item_events(item_id, created_at);
      `);
    },
  },
  {
    version: 6,
    name: "commit_point",
    up: (db) => {
      // Commit points sit in the batch's work queue (ordered by sort_index in
      // the same space as work_items). When the agent reaches one, the Stop
      // hook directs it to propose a commit message; the runtime performs the
      // commit either immediately (auto) or after user approval (approval).
      db.exec(`
        CREATE TABLE commit_point (
          id TEXT PRIMARY KEY,
          batch_id TEXT NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
          sort_index INTEGER NOT NULL,
          mode TEXT NOT NULL,
          status TEXT NOT NULL,
          proposed_message TEXT,
          approved_message TEXT,
          commit_sha TEXT,
          rejection_note TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          completed_at TEXT
        );

        CREATE INDEX idx_commit_point_batch_sort ON commit_point(batch_id, sort_index);
        CREATE INDEX idx_commit_point_batch_status ON commit_point(batch_id, status);
      `);
    },
  },
  {
    version: 7,
    name: "wait_point",
    up: (db) => {
      // Wait points interrupt the Stop-hook auto-progression pipeline. When
      // the agent reaches a pending wait point the runtime flips it to
      // `triggered` and lets the agent stop; the user clicks Continue (which
      // marks it `done`) before the next prompt resumes auto-progression.
      db.exec(`
        CREATE TABLE wait_point (
          id TEXT PRIMARY KEY,
          batch_id TEXT NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
          sort_index INTEGER NOT NULL,
          status TEXT NOT NULL,
          note TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          completed_at TEXT
        );

        CREATE INDEX idx_wait_point_batch_sort ON wait_point(batch_id, sort_index);
        CREATE INDEX idx_wait_point_batch_status ON wait_point(batch_id, status);
      `);
    },
  },
  {
    version: 8,
    name: "file_snapshot",
    up: (db) => {
      // Content-addressed snapshot tracking. `file_snapshot` is the metadata
      // row for one flushed manifest on disk; the manifest itself lives at
      // `.oxplow/snapshots/manifests/<id>.json` and holds the dirty-path
      // entries (hash + mtime + size). Walking the parent chain
      // reconstructs the full file set at any point in time.
      db.exec(`
        CREATE TABLE file_snapshot (
          id TEXT PRIMARY KEY,
          stream_id TEXT NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
          worktree_path TEXT NOT NULL,
          kind TEXT NOT NULL,
          turn_id TEXT REFERENCES agent_turn(id) ON DELETE SET NULL,
          batch_id TEXT REFERENCES batches(id) ON DELETE SET NULL,
          parent_snapshot_id TEXT REFERENCES file_snapshot(id) ON DELETE SET NULL,
          manifest_path TEXT NOT NULL,
          created_at TEXT NOT NULL
        );

        CREATE INDEX idx_file_snapshot_stream ON file_snapshot(stream_id, created_at DESC);
        CREATE INDEX idx_file_snapshot_turn ON file_snapshot(turn_id);

        ALTER TABLE batch_file_change ADD COLUMN snapshot_id TEXT REFERENCES file_snapshot(id) ON DELETE SET NULL;
        ALTER TABLE streams ADD COLUMN current_snapshot_id TEXT REFERENCES file_snapshot(id) ON DELETE SET NULL;
      `);
    },
  },
  {
    version: 9,
    name: "snapshot_entry",
    up: (db) => {
      // Move manifest storage out of .oxplow/snapshots/manifests/*.json into
      // SQLite. Also drops the now-unused manifest_path column on
      // file_snapshot. Existing snapshot rows are wiped since their
      // manifests are on-disk JSON we no longer read; cascades clear
      // streams.current_snapshot_id and batch_file_change.snapshot_id for
      // us. Blobs in .oxplow/snapshots/objects/ are orphaned by this and
      // get GC'd on the next cleanup cycle.
      db.exec(`
        PRAGMA defer_foreign_keys = 1;

        DELETE FROM file_snapshot;

        CREATE TABLE file_snapshot_new (
          id TEXT PRIMARY KEY,
          stream_id TEXT NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
          worktree_path TEXT NOT NULL,
          kind TEXT NOT NULL,
          turn_id TEXT REFERENCES agent_turn(id) ON DELETE SET NULL,
          batch_id TEXT REFERENCES batches(id) ON DELETE SET NULL,
          parent_snapshot_id TEXT REFERENCES file_snapshot_new(id) ON DELETE SET NULL,
          created_at TEXT NOT NULL
        );

        DROP TABLE file_snapshot;
        ALTER TABLE file_snapshot_new RENAME TO file_snapshot;

        CREATE INDEX idx_file_snapshot_stream ON file_snapshot(stream_id, created_at DESC);
        CREATE INDEX idx_file_snapshot_turn ON file_snapshot(turn_id);

        CREATE TABLE snapshot_entry (
          snapshot_id TEXT NOT NULL REFERENCES file_snapshot(id) ON DELETE CASCADE,
          path TEXT NOT NULL,
          hash TEXT NOT NULL,
          mtime_ms INTEGER NOT NULL,
          size INTEGER NOT NULL,
          state TEXT NOT NULL,
          PRIMARY KEY (snapshot_id, path)
        );

        CREATE INDEX idx_snapshot_entry_hash ON snapshot_entry(hash);
      `);
    },
  },
  {
    version: 10,
    name: "agent_turn_token_usage",
    up: (db) => {
      // Token usage is extracted from Claude Code's transcript jsonl on Stop
      // and shown in the Agent history panel. Nullable so turns that predate
      // migration v10 (or fail to parse) render as "—".
      db.exec(`
        ALTER TABLE agent_turn ADD COLUMN input_tokens INTEGER;
        ALTER TABLE agent_turn ADD COLUMN output_tokens INTEGER;
        ALTER TABLE agent_turn ADD COLUMN cache_read_input_tokens INTEGER;
      `);
    },
  },
  {
    version: 11,
    name: "rename_to_check_to_human_check",
    up: (db) => {
      // The status enum renamed "to_check" → "human_check" so it reads as
      // "waiting on a human to verify" instead of a vague to-do phrase. Rewrite
      // every existing work_items row so old DBs keep working.
      db.exec(`UPDATE work_items SET status = 'human_check' WHERE status = 'to_check';`);
    },
  },
  {
    version: 12,
    name: "rename_current_batch_default_title",
    up: (db) => {
      // The seeded batch title changed from "Current Batch" to "Default".
      // Rewrite the first-ever batch in every stream so existing DBs pick up
      // the new name. Guard on sort_index = 0 so users who manually named a
      // batch "Current Batch" further down the queue are untouched.
      db.exec(`
        UPDATE batches
           SET title = 'Default'
         WHERE title = 'Current Batch'
           AND sort_index = 0;
      `);
    },
  },
  {
    version: 13,
    name: "drop_batch_summary_columns",
    up: (db) => {
      // The "batch summary" feature (record_batch_summary MCP tool + Stop-
      // hook rolling summary) was removed. The `summary` and
      // `summary_updated_at` columns are no longer read or written; drop
      // them so the schema matches the TypeScript model.
      db.exec(`
        ALTER TABLE batches DROP COLUMN summary_updated_at;
        ALTER TABLE batches DROP COLUMN summary;
      `);
    },
  },
  {
    version: 14,
    name: "streams.sort_index",
    up: (db) => {
      db.exec(`
        ALTER TABLE streams ADD COLUMN sort_index INTEGER NOT NULL DEFAULT 0;
        UPDATE streams SET sort_index = rowid;
      `);
    },
  },
  {
    version: 15,
    name: "batches.auto_commit",
    up: (db) => {
      db.exec(`ALTER TABLE batches ADD COLUMN auto_commit INTEGER NOT NULL DEFAULT 0;`);
    },
  },
  {
    version: 16,
    name: "commit_point.mode surfaced",
    up: (db) => {
      // The `mode` column has existed since v6 but was written as a fixed
      // 'auto' placeholder and not exposed in the TypeScript type. Now that
      // auto vs approve mode is a real UI concept, reset every existing row
      // to 'approve' (the new default intent) so the stop-hook pipeline
      // interprets them correctly. New rows also default to 'approve' via the
      // store's create() method.
      db.exec(`UPDATE commit_point SET mode = 'approve' WHERE mode = 'auto';`);
    },
  },
  {
    version: 17,
    name: "work_note",
    up: (db) => {
      // Dedicated note rows for work items. Notes are structured (body, author,
      // created_at) and queried directly, unlike the schemaless work_item_events
      // log entries. The UI shows a note count badge on list rows and a read-only
      // notes section in the edit modal. Author is a free-form string so both
      // "user" and "agent" sources can be written without a separate actor table.
      db.exec(`
        CREATE TABLE work_note (
          id TEXT PRIMARY KEY,
          work_item_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
          body TEXT NOT NULL,
          author TEXT NOT NULL,
          created_at TEXT NOT NULL
        );

        CREATE INDEX idx_work_note_item ON work_note(work_item_id, created_at);
      `);
    },
  },
  {
    version: 18,
    name: "custom_prompt_on_streams_and_batches",
    up: (db) => {
      // Per-stream and per-batch custom prompts. When set, these are appended
      // to the agent's system prompt so teams can give stream- or batch-level
      // standing instructions without touching the global agentPromptAppend.
      db.exec(`
        ALTER TABLE streams ADD COLUMN custom_prompt TEXT;
        ALTER TABLE batches ADD COLUMN custom_prompt TEXT;
      `);
    },
  },
  {
    version: 19,
    name: "drop_commit_point_proposed_message_and_status",
    up: (db) => {
      // The "proposed" status + `proposed_message` column were part of a
      // two-step draft-then-commit flow that's been replaced by the agent
      // drafting a message directly in chat and calling `oxplow__commit` once
      // the user approves. Collapse any lingering `proposed` rows back to
      // `pending` and drop the now-unused column.
      db.exec(`
        UPDATE commit_point SET status = 'pending' WHERE status = 'proposed';
        ALTER TABLE commit_point DROP COLUMN proposed_message;
      `);
    },
  },
  {
    version: 20,
    name: "rethink_history_tracking",
    up: (db) => {
      // Rebuilds the history-tracking subsystem. Drops batch_file_change
      // entirely (per-file logs now derived from snapshot diffs), flattens
      // file_snapshot (no more parent chain; time-ordered with a version hash
      // for dedup), drops agent_turn.work_item_id in favour of a many-to-many
      // join through work_item_effort, and introduces work_item_effort +
      // work_item_effort_turn. All prior snapshot/turn/file-change rows are
      // wiped — history is not preserved across this migration.
      db.exec(`
        PRAGMA defer_foreign_keys = 1;

        DROP TABLE IF EXISTS batch_file_change;

        DELETE FROM file_snapshot;
        -- snapshot_entry rows cascade from file_snapshot via the v9 FK.
        UPDATE streams SET current_snapshot_id = NULL;

        CREATE TABLE file_snapshot_new (
          id TEXT PRIMARY KEY,
          stream_id TEXT NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
          worktree_path TEXT NOT NULL,
          version_hash TEXT NOT NULL,
          source TEXT NOT NULL,
          created_at TEXT NOT NULL
        );

        DROP TABLE file_snapshot;
        ALTER TABLE file_snapshot_new RENAME TO file_snapshot;

        CREATE INDEX idx_file_snapshot_stream ON file_snapshot(stream_id, created_at DESC);
        CREATE INDEX idx_file_snapshot_stream_hash ON file_snapshot(stream_id, version_hash);

        CREATE TABLE agent_turn_new (
          id TEXT PRIMARY KEY,
          batch_id TEXT NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
          prompt TEXT NOT NULL,
          answer TEXT,
          session_id TEXT,
          started_at TEXT NOT NULL,
          ended_at TEXT,
          input_tokens INTEGER,
          output_tokens INTEGER,
          cache_read_input_tokens INTEGER,
          start_snapshot_id TEXT REFERENCES file_snapshot(id) ON DELETE SET NULL,
          end_snapshot_id TEXT REFERENCES file_snapshot(id) ON DELETE SET NULL
        );

        DROP TABLE agent_turn;
        ALTER TABLE agent_turn_new RENAME TO agent_turn;

        CREATE INDEX idx_agent_turn_batch ON agent_turn(batch_id, started_at DESC);

        CREATE TABLE work_item_effort (
          id TEXT PRIMARY KEY,
          work_item_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
          started_at TEXT NOT NULL,
          ended_at TEXT,
          start_snapshot_id TEXT REFERENCES file_snapshot(id) ON DELETE SET NULL,
          end_snapshot_id TEXT REFERENCES file_snapshot(id) ON DELETE SET NULL
        );

        CREATE INDEX idx_work_item_effort_item ON work_item_effort(work_item_id, started_at DESC);
        CREATE INDEX idx_work_item_effort_open ON work_item_effort(work_item_id) WHERE ended_at IS NULL;

        CREATE TABLE work_item_effort_turn (
          effort_id TEXT NOT NULL REFERENCES work_item_effort(id) ON DELETE CASCADE,
          turn_id TEXT NOT NULL REFERENCES agent_turn(id) ON DELETE CASCADE,
          PRIMARY KEY (effort_id, turn_id)
        );

        CREATE INDEX idx_work_item_effort_turn_turn ON work_item_effort_turn(turn_id);
      `);
    },
  },
  {
    version: 21,
    name: "work_item_effort_unique_open",
    up: (db) => {
      // Enforce the "at most one open effort per work_item at a time"
      // invariant at the DB layer. `openEffort`'s in-code guard is still
      // the normal path; this makes a direct insert or a race impossible
      // to leave behind two open rows silently. The v20 partial index was
      // not UNIQUE.
      db.exec(`
        DROP INDEX IF EXISTS idx_work_item_effort_open;
        CREATE UNIQUE INDEX idx_work_item_effort_open
          ON work_item_effort(work_item_id) WHERE ended_at IS NULL;
      `);
    },
  },
  {
    version: 22,
    name: "work_item_effort_file",
    up: (db) => {
      // Per-effort write log. Populated by the PostToolUse hook when
      // exactly one effort is in_progress for the batch (the active-
      // effort heuristic) so parallel subagents within one batch get
      // distinct file lists instead of the union via the snapshot pair
      // diff. See .context/agent-model.md "per-effort write log".
      db.exec(`
        CREATE TABLE work_item_effort_file (
          effort_id TEXT NOT NULL REFERENCES work_item_effort(id) ON DELETE CASCADE,
          path TEXT NOT NULL,
          first_seen_at TEXT NOT NULL,
          PRIMARY KEY (effort_id, path)
        );

        CREATE INDEX idx_work_item_effort_file_effort ON work_item_effort_file(effort_id);
      `);
    },
  },
  {
    version: 23,
    name: "rename_batch_to_thread",
    up: (db) => {
      // Global concept rename: "batch" → "thread". SQLite >= 3.25 supports
      // ALTER TABLE RENAME TO and ALTER TABLE RENAME COLUMN, which preserves
      // row ids and foreign-key relationships automatically. Indexes must be
      // dropped and recreated under their new names — SQLite has no RENAME
      // INDEX.
      db.exec(`
        PRAGMA defer_foreign_keys = 1;

        -- Tables
        ALTER TABLE batches RENAME TO threads;
        ALTER TABLE batch_selection RENAME TO thread_selection;

        -- Columns on thread_selection
        ALTER TABLE thread_selection RENAME COLUMN selected_batch_id TO selected_thread_id;

        -- Columns on child tables (batch_id → thread_id)
        ALTER TABLE work_items RENAME COLUMN batch_id TO thread_id;
        ALTER TABLE work_item_links RENAME COLUMN batch_id TO thread_id;
        ALTER TABLE work_item_events RENAME COLUMN batch_id TO thread_id;
        ALTER TABLE agent_turn RENAME COLUMN batch_id TO thread_id;
        ALTER TABLE commit_point RENAME COLUMN batch_id TO thread_id;
        ALTER TABLE wait_point RENAME COLUMN batch_id TO thread_id;

        -- Drop old indexes (named after the old concept).
        DROP INDEX IF EXISTS idx_batches_stream_sort;
        DROP INDEX IF EXISTS idx_work_items_batch_parent;
        DROP INDEX IF EXISTS idx_work_items_batch_status;
        DROP INDEX IF EXISTS idx_work_items_batch_deleted;
        DROP INDEX IF EXISTS idx_work_items_backlog;
        DROP INDEX IF EXISTS idx_work_links_batch_from;
        DROP INDEX IF EXISTS idx_work_links_batch_to;
        DROP INDEX IF EXISTS idx_work_events_batch_item;
        DROP INDEX IF EXISTS idx_work_events_item;
        DROP INDEX IF EXISTS idx_agent_turn_batch;
        DROP INDEX IF EXISTS idx_commit_point_batch_sort;
        DROP INDEX IF EXISTS idx_commit_point_batch_status;
        DROP INDEX IF EXISTS idx_wait_point_batch_sort;
        DROP INDEX IF EXISTS idx_wait_point_batch_status;

        -- Recreate with the new names.
        CREATE INDEX idx_threads_stream_sort ON threads(stream_id, sort_index);
        CREATE INDEX idx_work_items_thread_parent ON work_items(thread_id, parent_id, sort_index);
        CREATE INDEX idx_work_items_thread_status ON work_items(thread_id, status, sort_index);
        CREATE INDEX idx_work_items_thread_deleted ON work_items(thread_id, deleted_at, sort_index);
        CREATE INDEX idx_work_items_backlog ON work_items(deleted_at, sort_index) WHERE thread_id IS NULL;
        CREATE INDEX idx_work_links_thread_from ON work_item_links(thread_id, from_item_id);
        CREATE INDEX idx_work_links_thread_to ON work_item_links(thread_id, to_item_id);
        CREATE INDEX idx_work_events_thread_item ON work_item_events(thread_id, item_id, created_at);
        CREATE INDEX idx_work_events_item ON work_item_events(item_id, created_at);
        CREATE INDEX idx_agent_turn_thread ON agent_turn(thread_id, started_at DESC);
        CREATE INDEX idx_commit_point_thread_sort ON commit_point(thread_id, sort_index);
        CREATE INDEX idx_commit_point_thread_status ON commit_point(thread_id, status);
        CREATE INDEX idx_wait_point_thread_sort ON wait_point(thread_id, sort_index);
        CREATE INDEX idx_wait_point_thread_status ON wait_point(thread_id, status);
      `);
    },
  },
  {
    version: 24,
    name: "drop_snapshot_entry_deleted_tombstones",
    up: (db) => {
      // Tombstones (state='deleted' rows) are no longer persisted. Readers
      // already treat "entry missing" and "state='deleted'" identically;
      // collapsing to a single case removes the computeVersionHash carve-out
      // that excluded tombstones. No schema change — the state column has
      // always been plain TEXT with no CHECK constraint.
      db.exec(`DELETE FROM snapshot_entry WHERE state = 'deleted';`);
    },
  },
  {
    version: 25,
    name: "work_note_thread_scoped",
    up: (db) => {
      // Broaden `work_note` so it can hold thread-scoped rows (not attached
      // to any individual work item). This is the durable landing spot for
      // `oxplow__delegate_query` Explore-subagent findings: the orchestrator
      // can fetch them via `get_thread_notes` only when it actually needs
      // the content, keeping its own cached context small.
      //
      // Schema changes:
      //   - `work_item_id` relaxed to NULLABLE (was NOT NULL).
      //   - New nullable `thread_id TEXT REFERENCES threads(id) ON DELETE CASCADE`.
      //   - CHECK: exactly one of `work_item_id`, `thread_id` is non-NULL.
      //
      // Existing rows (if any) are item-scoped so they satisfy the CHECK
      // with thread_id = NULL unchanged.
      db.exec(`
        PRAGMA defer_foreign_keys = 1;

        CREATE TABLE work_note_new (
          id TEXT PRIMARY KEY,
          work_item_id TEXT REFERENCES work_items(id) ON DELETE CASCADE,
          thread_id TEXT REFERENCES threads(id) ON DELETE CASCADE,
          body TEXT NOT NULL,
          author TEXT NOT NULL,
          created_at TEXT NOT NULL,
          CHECK (
            (work_item_id IS NOT NULL AND thread_id IS NULL)
            OR (work_item_id IS NULL AND thread_id IS NOT NULL)
          )
        );

        INSERT INTO work_note_new (id, work_item_id, thread_id, body, author, created_at)
          SELECT id, work_item_id, NULL, body, author, created_at FROM work_note;

        DROP TABLE work_note;
        ALTER TABLE work_note_new RENAME TO work_note;

        CREATE INDEX idx_work_note_item ON work_note(work_item_id, created_at);
        CREATE INDEX idx_work_note_thread ON work_note(thread_id, created_at DESC);
      `);
    },
  },
  {
    version: 26,
    name: "work_items.author",
    up: (db) => {
      // Add a nullable `author` column to `work_items` that distinguishes
      // the semantic origin of the row (as opposed to `created_by` which
      // classifies the writer: user/agent/system). Values:
      //   'user'       — explicit user-initiated create
      //   'agent'      — explicit agent-initiated create (via MCP tool)
      //   'agent-auto' — runtime-synthesized on first write-intent tool call
      //   NULL         — legacy rows / not classified
      // The auto-file → explicit adoption flow flips 'agent-auto' → 'agent'
      // when the agent calls create_work_item during the same turn.
      db.exec(`
        ALTER TABLE work_items ADD COLUMN author TEXT;
        CREATE INDEX idx_work_items_thread_author_status
          ON work_items(thread_id, author, status);
      `);
    },
  },
  {
    version: 27,
    name: "work_item_commit junction",
    up: (db) => {
      // Junction linking work items to the git commits that landed them.
      // Populated by the runtime's auto-commit path after a successful
      // `gitCommitAll`, using the same "tasks since last commit" cutoff
      // the MCP tool uses. Enables a local-history / blame view that
      // attributes a commit sha back to the contributing work items.
      db.exec(`
        CREATE TABLE work_item_commit (
          work_item_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
          sha TEXT NOT NULL,
          committed_at TEXT NOT NULL,
          PRIMARY KEY (work_item_id, sha)
        );
        CREATE INDEX idx_work_item_commit_sha ON work_item_commit(sha);
      `);
    },
  },
  {
    version: 28,
    name: "drop work_item_commit junction",
    up: (db) => {
      // Commit↔item attribution can't be made reliable — users commit
      // outside oxplow (IDE buttons, CLI, CI rebases, merges, squashes)
      // and oxplow has no authoritative hook at those sites. The
      // heuristic "settled_at > latest_done_commit_point" misattributes
      // silently. A blame overlay that sometimes lies is worse than no
      // overlay, so the feature is removed before any live consumer
      // shipped.
      db.exec(`DROP TABLE IF EXISTS work_item_commit;`);
    },
  },
  {
    version: 29,
    name: "cancel legacy agent-auto in_progress rows",
    up: (db) => {
      // Auto-file / auto-complete were removed in favor of passively
      // rendering `agent_turn` rows in the in_progress bucket. Any
      // author='agent-auto' row still sitting in_progress on upgrade is an
      // orphan (the auto-complete path used to close them, but the listener
      // went away). Flip them to `canceled` and drop a note so the user
      // can see what happened if they hunt down the row. Non-in_progress
      // agent-auto rows (already human_check / done / canceled) are left
      // alone — they've already run the adoption/completion path.
      const now = new Date().toISOString();
      db.exec(`
        UPDATE work_items
        SET status = 'canceled', updated_at = '${now}'
        WHERE author = 'agent-auto' AND status = 'in_progress';
      `);
      const rows = db.all<{ id: string; thread_id: string | null }>(
        `SELECT id, thread_id FROM work_items
         WHERE author = 'agent-auto' AND status = 'canceled' AND updated_at = '${now}'`,
      );
      for (const row of rows) {
        const noteId = `note-v29-${row.id}`;
        db.run(
          `INSERT INTO work_note (id, work_item_id, thread_id, body, author, created_at)
           VALUES (?, ?, NULL, ?, 'system', ?)`,
          noteId,
          row.id,
          "legacy auto-item (pre-v29); auto-file removed",
          now,
        );
      }
    },
  },
  {
    version: 30,
    name: "agent_turn.task_list_json",
    up: (db) => {
      // Add per-turn TaskCreate/TaskUpdate breakdown storage. Written
      // incrementally by the PostToolUse TaskCreate/TaskUpdate bridge so
      // the Work panel's open-turn row can render the live sub-list;
      // persists on the row after the turn closes for History.
      db.exec(`ALTER TABLE agent_turn ADD COLUMN task_list_json TEXT;`);
    },
  },
  {
    version: 31,
    name: "agent_turn.produced_activity",
    up: (db) => {
      // Per-turn record of whether any mutation / filing / dispatch tool
      // call fired during the turn (the same flag the Stop-hook's
      // ready-work suppression rule already computes in-memory). Captured
      // at Stop so the Work panel can surface closed turns that were pure
      // Q&A as "Recent answers" — re-readable without cluttering the
      // Done section or the in_progress bucket. Pre-migration rows stay
      // NULL (unknown); the UI treats NULL as "not shown" to avoid
      // back-filling every legacy turn with a synthetic value.
      db.exec(`ALTER TABLE agent_turn ADD COLUMN produced_activity INTEGER;`);
    },
  },
  {
    version: 32,
    name: "agent_turn.archived_at",
    up: (db) => {
      // "Recent answers" rows can be archived by the user to clear them
      // out of the Work panel without marking them as having produced
      // activity (which would misrepresent the turn). Nullable ISO
      // timestamp: NULL means visible, non-NULL means hidden from the
      // Recent-answers query.
      db.exec(`ALTER TABLE agent_turn ADD COLUMN archived_at TEXT;`);
    },
  },
  {
    version: 33,
    name: "wiki_note",
    up: (db) => {
      // User-curated personal knowledgebase. Bodies are plain markdown
      // files under `.oxplow/notes/<slug>.md` — filesystem is the source
      // of truth for content. This table only holds metadata (title,
      // references, freshness baseline) and is re-synced by the notes
      // watcher whenever a file changes on disk. Named `wiki_note` to
      // distinguish from the existing `work_note` table (which is
      // attached to work items).
      //
      // captured_head_sha / captured_refs_json are the freshness
      // baseline: the HEAD and the set of referenced {path, blob_sha}
      // tuples as of the most recent write/edit. A note is "stale" when
      // HEAD has advanced or any referenced file's blob hash has
      // changed since the baseline was captured.
      db.exec(`
        CREATE TABLE wiki_note (
          id TEXT PRIMARY KEY,
          slug TEXT NOT NULL UNIQUE,
          title TEXT NOT NULL,
          captured_head_sha TEXT,
          captured_refs_json TEXT NOT NULL DEFAULT '[]',
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE INDEX idx_wiki_note_updated ON wiki_note(updated_at DESC);
      `);
    },
  },
  {
    version: 34,
    name: "streams.kind",
    up: (db) => {
      // Distinguish the leftmost "primary" stream — whose worktree IS the
      // project directory — from user-created worktree streams under
      // .oxplow/worktrees/. Primary's branch may be changed from the UI;
      // the primary row is never deleted. All existing rows default to
      // 'worktree' so the database boots safely; initialize() then
      // promotes or creates a primary row at runtime.
      db.exec(`
        ALTER TABLE streams ADD COLUMN kind TEXT NOT NULL DEFAULT 'worktree'
          CHECK (kind IN ('primary','worktree'));
      `);
    },
  },
  {
    version: 35,
    name: "file_snapshot.effort_id",
    up: (db) => {
      // Snapshots re-anchor from turn-scoped to effort-scoped. The
      // existing `turn_id` column is left intact for now (sibling task
      // wi-db150f2b312c will drop it together with agent_turn). New
      // `effort_id` is nullable: startup snapshots and any pre-migration
      // rows have no owning effort.
      db.exec(`
        ALTER TABLE file_snapshot ADD COLUMN effort_id TEXT
          REFERENCES work_item_effort(id) ON DELETE SET NULL;
        CREATE INDEX idx_file_snapshot_effort ON file_snapshot(effort_id);
      `);
    },
  },
  {
    version: 36,
    name: "drop agent_turn + work_item_effort_turn + file_snapshot.turn_id",
    up: (db) => {
      // Agent-turn tracking is fully removed. The runtime/IPC/UI no longer
      // reads or writes any of these tables, and snapshots have been
      // re-anchored to efforts via `file_snapshot.effort_id` (v35). Drop
      // the now-unused junction, the agent_turn table itself, and the
      // legacy `turn_id` column on `file_snapshot`. SQLite >= 3.35
      // supports ALTER TABLE DROP COLUMN, which is what bun:sqlite ships,
      // so the column drop is a one-liner.
      // v20 already rebuilt file_snapshot without `turn_id`, so no column
      // drop is needed — only the index (if it lingers from v8/v9 on
      // databases that bypassed the rebuild for some reason). The
      // `DROP INDEX IF EXISTS` handles both cases gracefully.
      db.exec(`
        PRAGMA defer_foreign_keys = 1;

        DROP INDEX IF EXISTS idx_work_item_effort_turn_turn;
        DROP TABLE IF EXISTS work_item_effort_turn;

        DROP INDEX IF EXISTS idx_agent_turn_thread;
        DROP INDEX IF EXISTS idx_agent_turn_batch;
        DROP INDEX IF EXISTS idx_agent_turn_item;
        DROP TABLE IF EXISTS agent_turn;

        DROP INDEX IF EXISTS idx_file_snapshot_turn;
      `);
    },
  },
  {
    version: 37,
    name: "work_item_effort.summary",
    up: (db) => {
      // Per-effort summary text written when an effort closes (the
      // `note` arg on `complete_task`). Replaces appending the same text
      // to the work-item note history — efforts are now the unit of
      // attribution and carry their own one-line summary of what
      // shipped in that effort.
      db.exec(`
        ALTER TABLE work_item_effort ADD COLUMN summary TEXT;
      `);
    },
  },
  {
    version: 38,
    name: "wiki_note.body",
    up: (db) => {
      // Mirror the on-disk note body in SQLite so the agent can run
      // content searches without reading every file. The watcher
      // (`syncNoteFromDisk`) writes both metadata and body on every
      // upsert, so the column stays in sync with the filesystem
      // source-of-truth. Existing rows default to '' and are populated
      // on the next watcher pass (`scanAndSyncAll` runs at startup).
      db.exec(`ALTER TABLE wiki_note ADD COLUMN body TEXT NOT NULL DEFAULT '';`);
    },
  },
  {
    version: 39,
    name: "usage_event + wiki_note_fts",
    up: (db) => {
      // Generic (kind, key) usage tracking. Append-only event log so
      // rollups stay derived rather than persisted — adding a new "kind"
      // (editor file, work item, future) doesn't require a new table.
      // `stream_id` is nullable so cross-stream targets (e.g. work items
      // viewed from the backlog) can record without a stream context.
      // The runtime writer coalesces events arriving < 30s apart for the
      // same (kind, key, event) by bumping `occurred_at` instead of
      // inserting, so rapid re-selects of the same target don't spam
      // history.
      db.exec(`
        CREATE TABLE usage_event (
          id INTEGER PRIMARY KEY,
          stream_id TEXT,
          kind TEXT NOT NULL,
          key TEXT NOT NULL,
          event TEXT NOT NULL,
          occurred_at TEXT NOT NULL
        );
        CREATE INDEX idx_usage_event_lookup ON usage_event(kind, key, occurred_at DESC);
        CREATE INDEX idx_usage_event_stream ON usage_event(stream_id, kind, occurred_at DESC);
      `);

      // FTS5 virtual table mirroring wiki_note (title + body) for ranked
      // full-text search. `content='wiki_note'` makes this a contentless
      // index so the source rows aren't duplicated; triggers keep the
      // index in sync. Backfill fires once via the special 'rebuild'
      // command for any rows that already exist.
      db.exec(`
        CREATE VIRTUAL TABLE wiki_note_fts USING fts5(
          slug UNINDEXED,
          title,
          body,
          content='wiki_note',
          content_rowid='rowid'
        );

        CREATE TRIGGER wiki_note_ai AFTER INSERT ON wiki_note BEGIN
          INSERT INTO wiki_note_fts(rowid, slug, title, body)
            VALUES (new.rowid, new.slug, new.title, new.body);
        END;

        CREATE TRIGGER wiki_note_ad AFTER DELETE ON wiki_note BEGIN
          INSERT INTO wiki_note_fts(wiki_note_fts, rowid, slug, title, body)
            VALUES ('delete', old.rowid, old.slug, old.title, old.body);
        END;

        CREATE TRIGGER wiki_note_au AFTER UPDATE ON wiki_note BEGIN
          INSERT INTO wiki_note_fts(wiki_note_fts, rowid, slug, title, body)
            VALUES ('delete', old.rowid, old.slug, old.title, old.body);
          INSERT INTO wiki_note_fts(rowid, slug, title, body)
            VALUES (new.rowid, new.slug, new.title, new.body);
        END;

        INSERT INTO wiki_note_fts(wiki_note_fts) VALUES('rebuild');
      `);
    },
  },
  {
    version: 40,
    name: "usage_event.thread_id",
    up: (db) => {
      // Per-thread scope on usage rows. Stream is the user-visible
      // workspace tab; thread is the work context within it. Queries
      // can group either dimension (or both, intersected) to answer
      // "what did this thread care about?" vs "what did this stream
      // care about?". Nullable so kinds without a thread context (e.g.
      // wiki-note visits today) and pre-migration rows are handled.
      db.exec(`
        ALTER TABLE usage_event ADD COLUMN thread_id TEXT;
        CREATE INDEX idx_usage_event_thread ON usage_event(thread_id, kind, occurred_at DESC);
      `);
    },
  },
  {
    version: 41,
    name: "code_quality_scan + code_quality_finding",
    up: (db) => {
      // Deterministic, language-agnostic code-quality findings sourced
      // from external CLIs (lizard, jscpd). One scan = one CLI invocation
      // for one (stream, tool, scope) combination. Findings are wiped on
      // re-scan via the store's retention pass; the schema doesn't enforce
      // FK cascade so the store stays in control of what counts as
      // "current" for a given stream + tool + scope.
      db.exec(`
        CREATE TABLE code_quality_scan (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          stream_id TEXT NOT NULL,
          tool TEXT NOT NULL,
          scope TEXT NOT NULL,
          base_ref TEXT,
          status TEXT NOT NULL,
          error_message TEXT,
          started_at TEXT NOT NULL,
          completed_at TEXT
        );
        CREATE INDEX idx_code_quality_scan_stream_tool_started
          ON code_quality_scan(stream_id, tool, started_at DESC);

        CREATE TABLE code_quality_finding (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          scan_id INTEGER NOT NULL,
          path TEXT NOT NULL,
          start_line INTEGER NOT NULL,
          end_line INTEGER NOT NULL,
          kind TEXT NOT NULL,
          metric_value REAL NOT NULL,
          extra_json TEXT
        );
        CREATE INDEX idx_code_quality_finding_scan
          ON code_quality_finding(scan_id);
        CREATE INDEX idx_code_quality_finding_scan_path
          ON code_quality_finding(scan_id, path);
        CREATE INDEX idx_code_quality_finding_scan_kind_value
          ON code_quality_finding(scan_id, kind, metric_value DESC);
      `);
    },
  },
  {
    version: 42,
    name: "drop commit/wait point markers + auto_commit",
    up: (db) => {
      // Commits are now driven exclusively by the user (CLI / Bash). The
      // queueable commit/wait point markers and the per-thread auto_commit
      // toggle are gone — Stop hook no longer emits commit directives, and
      // the runtime no longer runs `git commit`. Drop the marker tables
      // and the column outright; existing rows are not migrated forward
      // because no surface in the new system reads them. SQLite supports
      // DROP COLUMN since 3.35; bun's bundled sqlite is current enough.
      db.exec(`
        DROP INDEX IF EXISTS idx_commit_point_thread_sort;
        DROP INDEX IF EXISTS idx_commit_point_thread_status;
        DROP INDEX IF EXISTS idx_wait_point_thread_sort;
        DROP INDEX IF EXISTS idx_wait_point_thread_status;
        DROP TABLE IF EXISTS commit_point;
        DROP TABLE IF EXISTS wait_point;
        ALTER TABLE threads DROP COLUMN auto_commit;
      `);
    },
  },
  {
    version: 43,
    name: "drop_human_check_status",
    up: (db) => {
      // The human_check status was a "agent says done, user verifies"
      // intermediate. It produced enough churn (extra section, descending
      // sort logic, redo-hint surface, recent-human-check reminders) for
      // little real value — agents now self-mark `done` directly. Migrate
      // every existing row so older DBs keep working.
      db.exec(`UPDATE work_items SET status = 'done', completed_at = COALESCE(completed_at, updated_at) WHERE status = 'human_check';`);
    },
  },
  {
    version: 44,
    name: "thread closed_at + drop completed status",
    up: (db) => {
      // Threads gain a `closed_at` timestamp (nullable). Closing a thread
      // is now decoupled from active/queued status — closed threads stay
      // queued, just hidden from the rail. The legacy "completed" status
      // is gone; any existing completed rows get migrated to `queued` with
      // closed_at populated from updated_at so they show up in the new
      // Closed Threads page rather than vanishing.
      db.exec(`
        ALTER TABLE threads ADD COLUMN closed_at TEXT;
        UPDATE threads SET closed_at = updated_at, status = 'queued' WHERE status = 'completed';
      `);
    },
  },
  {
    version: 45,
    name: "finished_seen watermark",
    up: (db) => {
      db.exec(`
        CREATE TABLE finished_seen (
          scope TEXT PRIMARY KEY,
          t TEXT NOT NULL
        );
      `);
    },
  },
  {
    version: 45,
    name: "page_visit event log",
    up: (db) => {
      db.exec(`
        CREATE TABLE page_visit (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          t TEXT NOT NULL,
          stream_id TEXT,
          thread_id TEXT,
          ref_kind TEXT NOT NULL,
          ref_id TEXT NOT NULL,
          payload_json TEXT NOT NULL,
          label TEXT NOT NULL,
          source TEXT
        );
        CREATE INDEX idx_page_visit_t ON page_visit(t DESC);
        CREATE INDEX idx_page_visit_ref ON page_visit(ref_id);
        CREATE INDEX idx_page_visit_thread_t ON page_visit(thread_id, t DESC);
        CREATE INDEX idx_page_visit_kind_t ON page_visit(ref_kind, t DESC);
      `);
    },
  },
];

export function runMigrations(driver: SqlDriver, logger?: Logger): void {
  const row = driver.get<{ user_version: number }>("PRAGMA user_version");
  const current = row?.user_version ?? 0;
  const target = MIGRATIONS.at(-1)?.version ?? 0;
  if (current === target) return;
  if (current > target) {
    throw new Error(
      `database is at migration version ${current} but this build only knows up to ${target}; you are running an older binary against a newer database`,
    );
  }
  for (const migration of MIGRATIONS) {
    if (migration.version <= current) continue;
    driver.transaction(() => {
      migration.up(driver);
      // PRAGMA statements don't accept ? placeholders; `version` is a
      // controlled integer from MIGRATIONS, not user input.
      driver.exec(`PRAGMA user_version = ${migration.version}`);
    });
    logger?.info("applied migration", { version: migration.version, name: migration.name });
  }
}
