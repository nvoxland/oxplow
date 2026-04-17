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
