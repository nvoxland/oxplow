import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createSqlDriver } from "./driver.js";
import { MIGRATIONS, runMigrations } from "./migrations.js";

function freshDriver() {
  const dir = mkdtempSync(join(tmpdir(), "newde-migrations-"));
  return createSqlDriver(join(dir, "state.sqlite"));
}

describe("runMigrations", () => {
  test("applies every migration on a fresh database and bumps user_version", () => {
    const driver = freshDriver();
    runMigrations(driver);
    const target = MIGRATIONS.at(-1)!.version;
    expect(driver.get<{ user_version: number }>("PRAGMA user_version")?.user_version).toBe(target);
  });

  test("is a no-op when run twice", () => {
    const driver = freshDriver();
    runMigrations(driver);
    expect(() => runMigrations(driver)).not.toThrow();
    const target = MIGRATIONS.at(-1)!.version;
    expect(driver.get<{ user_version: number }>("PRAGMA user_version")?.user_version).toBe(target);
  });

  test("v1 creates the expected core tables", () => {
    const driver = freshDriver();
    runMigrations(driver);
    const tables = driver
      .all<{ name: string }>("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
      .map((row) => row.name);
    expect(tables).toEqual(expect.arrayContaining([
      "streams",
      "runtime_state",
      "batches",
      "batch_selection",
      "work_items",
      "work_item_links",
      "work_item_events",
    ]));
  });

  test("refuses to open a database at a higher version than this build knows", () => {
    const driver = freshDriver();
    runMigrations(driver);
    const unknownVersion = (MIGRATIONS.at(-1)!.version) + 5;
    driver.exec(`PRAGMA user_version = ${unknownVersion}`);
    expect(() => runMigrations(driver)).toThrow(/older binary/);
  });
});
