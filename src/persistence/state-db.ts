import { existsSync, mkdirSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";
import type { Logger } from "../core/logger.js";

const dbCache = new Map<string, StateDatabase>();
const require = createRequire(import.meta.url);

export class StateDatabase {
  readonly path: string;
  private readonly driver: SqlDriver;
  private closed = false;

  constructor(projectDir: string, private readonly logger?: Logger) {
    const rootDir = join(projectDir, ".newde");
    mkdirSync(rootDir, { recursive: true });
    this.path = join(rootDir, "state.sqlite");
    const preExisted = existsSync(this.path);
    this.driver = createSqlDriver(this.path);
    this.driver.exec("PRAGMA journal_mode = WAL;");
    this.driver.exec("PRAGMA foreign_keys = ON;");
    this.ensureSchema();
    this.logger?.debug("opened sqlite state database", {
      path: this.path,
      existed: preExisted,
      driver: this.driver.name,
    });
  }

  all<T>(sql: string, ...params: SQLiteValue[]): T[] {
    assertParamCount(sql, params);
    return this.driver.all<T>(sql, ...params);
  }

  get<T>(sql: string, ...params: SQLiteValue[]): T | null {
    assertParamCount(sql, params);
    return this.driver.get<T>(sql, ...params);
  }

  run(sql: string, ...params: SQLiteValue[]) {
    assertParamCount(sql, params);
    return this.driver.run(sql, ...params);
  }

  exec(sql: string): void {
    this.driver.exec(sql);
  }

  transaction<T>(fn: () => T): T {
    return this.driver.transaction(fn);
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    try {
      this.driver.close();
    } catch (error) {
      this.logger?.warn("failed to close sqlite state database", {
        path: this.path,
        error: error instanceof Error ? error.message : String(error),
      });
    }
    for (const [key, value] of dbCache) {
      if (value === this) dbCache.delete(key);
    }
  }

  private ensureSchema(): void {
    this.driver.exec(`
      CREATE TABLE IF NOT EXISTS streams (
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

      CREATE TABLE IF NOT EXISTS runtime_state (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        current_stream_id TEXT
      );

      CREATE TABLE IF NOT EXISTS batches (
        id TEXT PRIMARY KEY,
        stream_id TEXT NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
        title TEXT NOT NULL,
        status TEXT NOT NULL,
        sort_index INTEGER NOT NULL,
        pane_target TEXT NOT NULL,
        resume_session_id TEXT NOT NULL DEFAULT '',
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
      );

      CREATE TABLE IF NOT EXISTS batch_selection (
        stream_id TEXT PRIMARY KEY REFERENCES streams(id) ON DELETE CASCADE,
        selected_batch_id TEXT
      );

      CREATE TABLE IF NOT EXISTS work_items (
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
        completed_at TEXT
      );

      CREATE TABLE IF NOT EXISTS work_item_links (
        id TEXT PRIMARY KEY,
        batch_id TEXT NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
        from_item_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
        to_item_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
        link_type TEXT NOT NULL,
        created_at TEXT NOT NULL,
        CHECK (from_item_id <> to_item_id)
      );

      CREATE TABLE IF NOT EXISTS work_item_events (
        id TEXT PRIMARY KEY,
        batch_id TEXT NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
        item_id TEXT REFERENCES work_items(id) ON DELETE CASCADE,
        event_type TEXT NOT NULL,
        actor_kind TEXT NOT NULL,
        actor_id TEXT NOT NULL,
        payload_json TEXT NOT NULL,
        created_at TEXT NOT NULL
      );

      CREATE INDEX IF NOT EXISTS idx_streams_branch ON streams(branch);
      CREATE INDEX IF NOT EXISTS idx_batches_stream_sort ON batches(stream_id, sort_index);
      CREATE INDEX IF NOT EXISTS idx_work_items_batch_parent ON work_items(batch_id, parent_id, sort_index);
      CREATE INDEX IF NOT EXISTS idx_work_items_batch_status ON work_items(batch_id, status, sort_index);
      CREATE INDEX IF NOT EXISTS idx_work_links_batch_from ON work_item_links(batch_id, from_item_id);
      CREATE INDEX IF NOT EXISTS idx_work_links_batch_to ON work_item_links(batch_id, to_item_id);
      CREATE INDEX IF NOT EXISTS idx_work_events_batch_item ON work_item_events(batch_id, item_id, created_at);

      INSERT INTO runtime_state (id, current_stream_id)
      SELECT 1, NULL
      WHERE NOT EXISTS (SELECT 1 FROM runtime_state WHERE id = 1);
    `);
  }
}

export function getStateDatabase(projectDir: string, logger?: Logger): StateDatabase {
  let existing = dbCache.get(projectDir);
  if (!existing) {
    existing = new StateDatabase(projectDir, logger);
    dbCache.set(projectDir, existing);
  }
  return existing;
}

function createSqlDriver(path: string): SqlDriver {
  if ("Bun" in globalThis) {
    const { Database } = require("bun:sqlite") as { Database: BunDatabaseCtor };
    const db = new Database(path, { create: true, strict: true });
    return {
      name: "bun:sqlite",
      all<T>(sql: string, ...params: SQLiteValue[]) {
        return db.query(sql).all(...params) as T[];
      },
      get<T>(sql: string, ...params: SQLiteValue[]) {
        return (db.query(sql).get(...params) as T | null) ?? null;
      },
      run(sql: string, ...params: SQLiteValue[]) {
        return db.query(sql).run(...params);
      },
      exec(sql: string) {
        db.exec(sql);
      },
      transaction<T>(fn: () => T) {
        return db.transaction(fn)();
      },
      close() {
        db.close();
      },
    };
  }

  const { DatabaseSync } = require("node:sqlite") as { DatabaseSync: NodeDatabaseCtor };
  const db = new DatabaseSync(path);
  return {
    name: "node:sqlite",
    all<T>(sql: string, ...params: SQLiteValue[]) {
      return db.prepare(sql).all(...params) as T[];
    },
    get<T>(sql: string, ...params: SQLiteValue[]) {
      return (db.prepare(sql).get(...params) as T | null) ?? null;
    },
    run(sql: string, ...params: SQLiteValue[]) {
      return db.prepare(sql).run(...params);
    },
    exec(sql: string) {
      db.exec(sql);
    },
    transaction<T>(fn: () => T) {
      db.exec("BEGIN");
      try {
        const result = fn();
        db.exec("COMMIT");
        return result;
      } catch (error) {
        db.exec("ROLLBACK");
        throw error;
      }
    },
    close() {
      db.close();
    },
  };
}

function assertParamCount(sql: string, params: readonly SQLiteValue[]): void {
  const expected = (sql.match(/\?/g) ?? []).length;
  if (expected !== params.length) {
    throw new Error(
      `sqlite parameter count mismatch: sql expects ${expected}, received ${params.length}`,
    );
  }
}

interface SqlDriver {
  name: string;
  all<T>(sql: string, ...params: SQLiteValue[]): T[];
  get<T>(sql: string, ...params: SQLiteValue[]): T | null;
  run(sql: string, ...params: SQLiteValue[]): unknown;
  exec(sql: string): void;
  transaction<T>(fn: () => T): T;
  close(): void;
}

type BunDatabaseCtor = new (
  path: string,
  options?: { create?: boolean; strict?: boolean },
) => {
  query(sql: string): {
    all(...params: SQLiteValue[]): unknown[];
    get(...params: SQLiteValue[]): unknown;
    run(...params: SQLiteValue[]): unknown;
  };
  exec(sql: string): void;
  transaction<T>(fn: () => T): () => T;
  close(): void;
};

type NodeDatabaseCtor = new (path: string) => {
  prepare(sql: string): {
    all(...params: SQLiteValue[]): unknown[];
    get(...params: SQLiteValue[]): unknown;
    run(...params: SQLiteValue[]): unknown;
  };
  exec(sql: string): void;
  close(): void;
};

type SQLiteValue = string | number | bigint | boolean | Uint8Array | null;
