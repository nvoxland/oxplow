import { existsSync } from "node:fs";
import { join } from "node:path";
import type { Logger } from "../core/logger.js";
import { ensureOxplowRoot } from "../core/oxplow-dir.js";
import { assertParamCount, createSqlDriver, type SqlDriver, type SQLiteValue } from "./driver.js";
import { runMigrations } from "./migrations.js";

const dbCache = new Map<string, StateDatabase>();

export class StateDatabase {
  readonly path: string;
  private readonly driver: SqlDriver;
  private closed = false;

  constructor(projectDir: string, private readonly logger?: Logger) {
    const rootDir = ensureOxplowRoot(projectDir);
    this.path = join(rootDir, "state.sqlite");
    const preExisted = existsSync(this.path);
    this.driver = createSqlDriver(this.path);
    this.driver.exec("PRAGMA journal_mode = WAL;");
    this.driver.exec("PRAGMA foreign_keys = ON;");
    runMigrations(this.driver, this.logger?.child({ subsystem: "migrations" }));
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
}

export function getStateDatabase(projectDir: string, logger?: Logger): StateDatabase {
  let existing = dbCache.get(projectDir);
  if (!existing) {
    existing = new StateDatabase(projectDir, logger);
    dbCache.set(projectDir, existing);
  }
  return existing;
}
