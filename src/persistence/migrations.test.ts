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

  test("v12 renames the seeded 'Current Batch' row to 'Default' but leaves later matching rows alone", () => {
    // Guard: users who manually named a batch "Current Batch" at
    // sort_index > 0 shouldn't get clobbered by the rename. Only the
    // first-ever (sort_index=0) row in each stream should flip.
    const driver = freshDriver();
    runMigrations(driver);
    const now = "2024-01-01T00:00:00Z";
    driver.exec(`INSERT INTO streams (id, title, summary, branch, branch_ref, branch_source, worktree_path, working_pane, talking_pane, working_session_id, talking_session_id, created_at, updated_at) VALUES ('s1', 'S', '', 'main', 'refs/heads/main', 'local', '/tmp/s1', 'p1:working', 'p1:talking', '', '', '${now}', '${now}')`);
    driver.exec(`INSERT INTO batches (id, stream_id, title, status, sort_index, pane_target, resume_session_id, created_at, updated_at) VALUES ('b1', 's1', 'Current Batch', 'active', 0, 'pt1', '', '${now}', '${now}')`);
    driver.exec(`INSERT INTO batches (id, stream_id, title, status, sort_index, pane_target, resume_session_id, created_at, updated_at) VALUES ('b2', 's1', 'Current Batch', 'queued', 1, 'pt2', '', '${now}', '${now}')`);
    driver.exec(`INSERT INTO batches (id, stream_id, title, status, sort_index, pane_target, resume_session_id, created_at, updated_at) VALUES ('b3', 's1', 'Something else', 'queued', 2, 'pt3', '', '${now}', '${now}')`);

    // Re-invoke the v12 up directly so the assertion is scoped to that
    // migration's effect, independent of whether it already ran as part of
    // runMigrations above. (Idempotent — re-running produces the same
    // post-state.)
    const v12 = MIGRATIONS.find((m) => m.version === 12)!;
    v12.up(driver);

    const rows = driver.all<{ id: string; title: string }>(`SELECT id, title FROM batches ORDER BY sort_index`);
    expect(rows.find((r) => r.id === "b1")?.title).toBe("Default");
    expect(rows.find((r) => r.id === "b2")?.title).toBe("Current Batch");
    expect(rows.find((r) => r.id === "b3")?.title).toBe("Something else");
  });

  test("refuses to open a database at a higher version than this build knows", () => {
    const driver = freshDriver();
    runMigrations(driver);
    const unknownVersion = (MIGRATIONS.at(-1)!.version) + 5;
    driver.exec(`PRAGMA user_version = ${unknownVersion}`);
    expect(() => runMigrations(driver)).toThrow(/older binary/);
  });
});
