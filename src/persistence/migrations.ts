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
      // `.newde/snapshots/manifests/<id>.json` and holds the dirty-path
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
      // Move manifest storage out of .newde/snapshots/manifests/*.json into
      // SQLite. Also drops the now-unused manifest_path column on
      // file_snapshot. Existing snapshot rows are wiped since their
      // manifests are on-disk JSON we no longer read; cascades clear
      // streams.current_snapshot_id and batch_file_change.snapshot_id for
      // us. Blobs in .newde/snapshots/objects/ are orphaned by this and
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
