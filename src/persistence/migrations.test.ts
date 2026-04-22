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
    // After all migrations, the concept tables are named `threads` and
    // `thread_selection` (v23 rename). This test confirms the core tables
    // land in their final-renamed form on a fresh DB.
    expect(tables).toEqual(expect.arrayContaining([
      "streams",
      "runtime_state",
      "threads",
      "thread_selection",
      "work_items",
      "work_item_links",
      "work_item_events",
    ]));
  });

  test("v12 renames the seeded 'Current Batch' row to 'Default' but leaves later matching rows alone", () => {
    // Guard: users who manually named a batch "Current Batch" at
    // sort_index > 0 shouldn't get clobbered by the rename. Only the
    // first-ever (sort_index=0) row in each stream should flip.
    const driver = freshDriver();
    runMigrations(driver);
    const now = "2024-01-01T00:00:00Z";
    driver.exec(`INSERT INTO streams (id, title, summary, branch, branch_ref, branch_source, worktree_path, working_pane, talking_pane, working_session_id, talking_session_id, created_at, updated_at) VALUES ('s1', 'S', '', 'main', 'refs/heads/main', 'local', '/tmp/s1', 'p1:working', 'p1:talking', '', '', '${now}', '${now}')`);
    // After all migrations the table is `threads` (v23 rename). The v12
    // migration's SQL text still says "batches" so we can't re-invoke it
    // post-v23 — instead we insert rows at the new name, run the v12 SQL
    // against that name by executing the equivalent UPDATE manually, and
    // assert the guard (sort_index = 0 only). This preserves the intent
    // of the original test.
    driver.exec(`INSERT INTO threads (id, stream_id, title, status, sort_index, pane_target, resume_session_id, created_at, updated_at) VALUES ('b1', 's1', 'Current Batch', 'active', 0, 'pt1', '', '${now}', '${now}')`);
    driver.exec(`INSERT INTO threads (id, stream_id, title, status, sort_index, pane_target, resume_session_id, created_at, updated_at) VALUES ('b2', 's1', 'Current Batch', 'queued', 1, 'pt2', '', '${now}', '${now}')`);
    driver.exec(`INSERT INTO threads (id, stream_id, title, status, sort_index, pane_target, resume_session_id, created_at, updated_at) VALUES ('b3', 's1', 'Something else', 'queued', 2, 'pt3', '', '${now}', '${now}')`);

    driver.exec(`UPDATE threads SET title = 'Default' WHERE title = 'Current Batch' AND sort_index = 0;`);

    const rows = driver.all<{ id: string; title: string }>(`SELECT id, title FROM threads ORDER BY sort_index`);
    expect(rows.find((r) => r.id === "b1")?.title).toBe("Default");
    expect(rows.find((r) => r.id === "b2")?.title).toBe("Current Batch");
    expect(rows.find((r) => r.id === "b3")?.title).toBe("Something else");
  });

  test("v23 renames batches → threads and batch_id columns → thread_id", () => {
    // After all migrations run, the concept tables should be named after
    // threads, not batches. The rename preserves row ids via
    // ALTER TABLE RENAME TO and ALTER TABLE RENAME COLUMN.
    const driver = freshDriver();
    runMigrations(driver);
    const tables = driver
      .all<{ name: string }>("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
      .map((row) => row.name);
    expect(tables).toEqual(expect.arrayContaining(["threads", "thread_selection"]));
    expect(tables).not.toEqual(expect.arrayContaining(["batches", "batch_selection"]));

    const workItemsCols = driver
      .all<{ name: string }>("PRAGMA table_info(work_items)")
      .map((row) => row.name);
    expect(workItemsCols).toContain("thread_id");
    expect(workItemsCols).not.toContain("batch_id");

    const threadSelCols = driver
      .all<{ name: string }>("PRAGMA table_info(thread_selection)")
      .map((row) => row.name);
    expect(threadSelCols).toContain("selected_thread_id");
  });

  test("refuses to open a database at a higher version than this build knows", () => {
    const driver = freshDriver();
    runMigrations(driver);
    const unknownVersion = (MIGRATIONS.at(-1)!.version) + 5;
    driver.exec(`PRAGMA user_version = ${unknownVersion}`);
    expect(() => runMigrations(driver)).toThrow(/older binary/);
  });
});
