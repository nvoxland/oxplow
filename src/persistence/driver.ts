import { createRequire } from "node:module";

const require = createRequire(import.meta.url);

export type SQLiteValue = string | number | bigint | boolean | Uint8Array | null;

export interface SqlDriver {
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

export function createSqlDriver(path: string): SqlDriver {
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

export function assertParamCount(sql: string, params: readonly SQLiteValue[]): void {
  const expected = (sql.match(/\?/g) ?? []).length;
  if (expected !== params.length) {
    throw new Error(
      `sqlite parameter count mismatch: sql expects ${expected}, received ${params.length}`,
    );
  }
}
