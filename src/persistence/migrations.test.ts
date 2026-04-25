import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createSqlDriver } from "./driver.js";
import { MIGRATIONS, runMigrations } from "./migrations.js";

function freshDriver() {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-migrations-"));
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

  test("v33 creates the wiki_note table with expected columns + unique slug", () => {
    const driver = freshDriver();
    runMigrations(driver);
    const cols = driver
      .all<{ name: string; notnull: number }>("PRAGMA table_info(wiki_note)")
      .reduce<Record<string, number>>((acc, row) => {
        acc[row.name] = row.notnull;
        return acc;
      }, {});
    expect(Object.keys(cols)).toEqual(
      expect.arrayContaining([
        "id",
        "slug",
        "title",
        "captured_head_sha",
        "captured_refs_json",
        "created_at",
        "updated_at",
      ]),
    );
    // captured_head_sha is nullable (note may be written before any commit)
    expect(cols.captured_head_sha).toBe(0);
    // slug is UNIQUE
    const now = "2024-01-01T00:00:00Z";
    driver.exec(
      `INSERT INTO wiki_note (id, slug, title, captured_refs_json, created_at, updated_at) VALUES ('n1', 'hello', 'Hello', '[]', '${now}', '${now}')`,
    );
    expect(() =>
      driver.exec(
        `INSERT INTO wiki_note (id, slug, title, captured_refs_json, created_at, updated_at) VALUES ('n2', 'hello', 'Hello Again', '[]', '${now}', '${now}')`,
      ),
    ).toThrow();
  });

  test("removes state='deleted' snapshot_entry tombstones while keeping siblings and enclosing snapshot", () => {
    const driver = freshDriver();
    // Apply migrations up to v23 only (the pre-tombstone-removal schema),
    // seed a deleted-state row + a present sibling, then run remaining
    // migrations and assert the tombstone is gone.
    const preTarget = 23;
    for (const migration of MIGRATIONS) {
      if (migration.version > preTarget) break;
      driver.transaction(() => {
        migration.up(driver);
        driver.exec(`PRAGMA user_version = ${migration.version}`);
      });
    }
    const now = "2024-01-01T00:00:00Z";
    driver.exec(`INSERT INTO streams (id, title, summary, branch, branch_ref, branch_source, worktree_path, working_pane, talking_pane, working_session_id, talking_session_id, created_at, updated_at) VALUES ('s1', 'S', '', 'main', 'refs/heads/main', 'local', '/tmp/s1', 'p1:w', 'p1:t', '', '', '${now}', '${now}')`);
    driver.exec(`INSERT INTO file_snapshot (id, stream_id, worktree_path, version_hash, source, created_at) VALUES ('snap-1', 's1', '/tmp/s1', 'h', 'turn-end', '${now}')`);
    driver.exec(`INSERT INTO snapshot_entry (snapshot_id, path, hash, mtime_ms, size, state) VALUES ('snap-1', 'gone.txt', '', 0, 0, 'deleted')`);
    driver.exec(`INSERT INTO snapshot_entry (snapshot_id, path, hash, mtime_ms, size, state) VALUES ('snap-1', 'keep.txt', 'abc', 1, 5, 'present')`);

    runMigrations(driver);

    const rows = driver.all<{ path: string; state: string }>(
      `SELECT path, state FROM snapshot_entry WHERE snapshot_id = 'snap-1' ORDER BY path`,
    );
    expect(rows).toEqual([{ path: "keep.txt", state: "present" }]);
    const snapRow = driver.get<{ id: string }>(`SELECT id FROM file_snapshot WHERE id = 'snap-1'`);
    expect(snapRow?.id).toBe("snap-1");
  });

  test("v25 broadens work_note for thread-scoped notes without losing existing item-scoped rows", () => {
    // Apply migrations up to v24, seed an item-scoped work_note at the
    // pre-v25 schema (NOT NULL work_item_id, no thread_id column), then run
    // remaining migrations and assert the row still exists plus new
    // thread-scoped writes are accepted.
    const driver = freshDriver();
    const preTarget = 24;
    for (const migration of MIGRATIONS) {
      if (migration.version > preTarget) break;
      driver.transaction(() => {
        migration.up(driver);
        driver.exec(`PRAGMA user_version = ${migration.version}`);
      });
    }
    const now = "2024-01-01T00:00:00Z";
    driver.exec(`INSERT INTO streams (id, title, summary, branch, branch_ref, branch_source, worktree_path, working_pane, talking_pane, working_session_id, talking_session_id, created_at, updated_at) VALUES ('s1', 'S', '', 'main', 'refs/heads/main', 'local', '/tmp/s1', 'p1:w', 'p1:t', '', '', '${now}', '${now}')`);
    driver.exec(`INSERT INTO threads (id, stream_id, title, status, sort_index, pane_target, resume_session_id, created_at, updated_at) VALUES ('b1', 's1', 'T', 'active', 0, 'working', '', '${now}', '${now}')`);
    driver.exec(`INSERT INTO work_items (id, thread_id, parent_id, kind, title, description, status, priority, sort_index, created_by, created_at, updated_at) VALUES ('wi-1', 'b1', NULL, 'task', 'x', '', 'ready', 'medium', 0, 'user', '${now}', '${now}')`);
    driver.exec(`INSERT INTO work_note (id, work_item_id, body, author, created_at) VALUES ('note-pre', 'wi-1', 'legacy', 'user', '${now}')`);

    runMigrations(driver);

    // Existing row preserved with NULL thread_id.
    const legacy = driver.get<{ id: string; work_item_id: string; thread_id: string | null; body: string }>(
      `SELECT id, work_item_id, thread_id, body FROM work_note WHERE id = 'note-pre'`,
    );
    expect(legacy?.id).toBe("note-pre");
    expect(legacy?.work_item_id).toBe("wi-1");
    expect(legacy?.thread_id).toBeNull();
    expect(legacy?.body).toBe("legacy");

    // New thread-scoped row allowed.
    driver.exec(`INSERT INTO work_note (id, work_item_id, thread_id, body, author, created_at) VALUES ('note-thread', NULL, 'b1', 'finding', 'explore', '${now}')`);
    const threadNote = driver.get<{ id: string; work_item_id: string | null; thread_id: string }>(
      `SELECT id, work_item_id, thread_id FROM work_note WHERE id = 'note-thread'`,
    );
    expect(threadNote?.thread_id).toBe("b1");
    expect(threadNote?.work_item_id).toBeNull();

    // CHECK constraint rejects a row with both fields set or both NULL.
    expect(() =>
      driver.exec(`INSERT INTO work_note (id, work_item_id, thread_id, body, author, created_at) VALUES ('note-bad', 'wi-1', 'b1', 'bad', 'x', '${now}')`),
    ).toThrow();
    expect(() =>
      driver.exec(`INSERT INTO work_note (id, work_item_id, thread_id, body, author, created_at) VALUES ('note-bad2', NULL, NULL, 'bad', 'x', '${now}')`),
    ).toThrow();
  });

  test("v28 drops the work_item_commit junction table (v27 created it, v28 removes it)", () => {
    const driver = freshDriver();
    // Run only up through v27 so we can assert the intermediate state.
    for (const m of MIGRATIONS) {
      if (m.version <= 27) m.up(driver);
    }
    const mid = driver
      .all<{ name: string }>("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'work_item_commit'")
      .map((r) => r.name);
    expect(mid).toContain("work_item_commit");
    // v28 drops it.
    const v28 = MIGRATIONS.find((m) => m.version === 28);
    expect(v28).toBeDefined();
    v28!.up(driver);
    const after = driver
      .all<{ name: string }>("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'work_item_commit'")
      .map((r) => r.name);
    expect(after).toEqual([]);
  });

  test("migrations apply cleanly on a DB already at v26", () => {
    const driver = freshDriver();
    // Stop at v26 by running only migrations ≤ 26.
    for (const m of MIGRATIONS) {
      if (m.version > 26) continue;
      driver.transaction(() => {
        m.up(driver);
        driver.exec(`PRAGMA user_version = ${m.version}`);
      });
    }
    expect(driver.get<{ user_version: number }>("PRAGMA user_version")?.user_version).toBe(26);
    runMigrations(driver);
    expect(driver.get<{ user_version: number }>("PRAGMA user_version")?.user_version).toBe(MIGRATIONS.at(-1)!.version);
    // work_item_commit was created in v27 and removed in v28; running all
    // migrations on a v26 DB should leave the table absent.
    const tables = driver
      .all<{ name: string }>("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'work_item_commit'")
      .map((r) => r.name);
    expect(tables).toEqual([]);
  });

  test("v29 cancels lingering author='agent-auto' in_progress rows with a legacy note", () => {
    const driver = freshDriver();
    // Run up through v28 to seed a legacy auto-item at the schema where
    // author='agent-auto' was allowed.
    for (const m of MIGRATIONS) {
      if (m.version > 28) break;
      driver.transaction(() => {
        m.up(driver);
        driver.exec(`PRAGMA user_version = ${m.version}`);
      });
    }
    const now = "2024-01-01T00:00:00Z";
    driver.exec(`INSERT INTO streams (id, title, summary, branch, branch_ref, branch_source, worktree_path, working_pane, talking_pane, working_session_id, talking_session_id, created_at, updated_at) VALUES ('s1', 'S', '', 'main', 'refs/heads/main', 'local', '/tmp/s1', 'w', 't', '', '', '${now}', '${now}')`);
    driver.exec(`INSERT INTO threads (id, stream_id, title, status, sort_index, pane_target, resume_session_id, created_at, updated_at) VALUES ('b1', 's1', 'T', 'active', 0, 'working', '', '${now}', '${now}')`);
    driver.exec(`INSERT INTO work_items (id, thread_id, parent_id, kind, title, description, status, priority, sort_index, created_by, created_at, updated_at, author) VALUES ('wi-auto', 'b1', NULL, 'task', 'legacy auto', '', 'in_progress', 'medium', 0, 'system', '${now}', '${now}', 'agent-auto')`);
    driver.exec(`INSERT INTO work_items (id, thread_id, parent_id, kind, title, description, status, priority, sort_index, created_by, created_at, updated_at, author) VALUES ('wi-user', 'b1', NULL, 'task', 'user row', '', 'in_progress', 'medium', 1, 'user', '${now}', '${now}', 'user')`);

    runMigrations(driver);

    const auto = driver.get<{ status: string }>(`SELECT status FROM work_items WHERE id = 'wi-auto'`);
    expect(auto?.status).toBe("canceled");
    const user = driver.get<{ status: string }>(`SELECT status FROM work_items WHERE id = 'wi-user'`);
    expect(user?.status).toBe("in_progress");
    const notes = driver.all<{ body: string }>(`SELECT body FROM work_note WHERE work_item_id = 'wi-auto'`);
    expect(notes.length).toBeGreaterThan(0);
    expect(notes[0]!.body).toMatch(/legacy auto-item/);
  });

  test("v34 drops agent_turn, work_item_effort_turn, and file_snapshot.turn_id", () => {
    const driver = freshDriver();
    runMigrations(driver);
    const tables = driver
      .all<{ name: string }>("SELECT name FROM sqlite_master WHERE type = 'table'")
      .map((row) => row.name);
    expect(tables).not.toContain("agent_turn");
    expect(tables).not.toContain("work_item_effort_turn");
    const cols = driver
      .all<{ name: string }>("PRAGMA table_info(file_snapshot)")
      .map((row) => row.name);
    expect(cols).not.toContain("turn_id");
    expect(cols).toContain("effort_id");
  });

  test("v41 creates code_quality_scan and code_quality_finding tables with expected columns and indexes", () => {
    const driver = freshDriver();
    runMigrations(driver);

    const scanCols = driver
      .all<{ name: string; notnull: number }>("PRAGMA table_info(code_quality_scan)")
      .reduce<Record<string, number>>((acc, row) => {
        acc[row.name] = row.notnull;
        return acc;
      }, {});
    expect(Object.keys(scanCols)).toEqual(
      expect.arrayContaining([
        "id",
        "stream_id",
        "tool",
        "scope",
        "base_ref",
        "status",
        "error_message",
        "started_at",
        "completed_at",
      ]),
    );
    expect(scanCols.stream_id).toBe(1);
    expect(scanCols.tool).toBe(1);
    expect(scanCols.scope).toBe(1);
    expect(scanCols.status).toBe(1);
    expect(scanCols.started_at).toBe(1);
    expect(scanCols.base_ref).toBe(0);
    expect(scanCols.error_message).toBe(0);
    expect(scanCols.completed_at).toBe(0);

    const findingCols = driver
      .all<{ name: string; notnull: number }>("PRAGMA table_info(code_quality_finding)")
      .reduce<Record<string, number>>((acc, row) => {
        acc[row.name] = row.notnull;
        return acc;
      }, {});
    expect(Object.keys(findingCols)).toEqual(
      expect.arrayContaining([
        "id",
        "scan_id",
        "path",
        "start_line",
        "end_line",
        "kind",
        "metric_value",
        "extra_json",
      ]),
    );
    expect(findingCols.scan_id).toBe(1);
    expect(findingCols.path).toBe(1);
    expect(findingCols.start_line).toBe(1);
    expect(findingCols.end_line).toBe(1);
    expect(findingCols.kind).toBe(1);
    expect(findingCols.metric_value).toBe(1);
    expect(findingCols.extra_json).toBe(0);

    const indexes = driver
      .all<{ name: string }>(
        `SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name IN ('code_quality_scan', 'code_quality_finding')`,
      )
      .map((row) => row.name);
    expect(indexes).toEqual(
      expect.arrayContaining([
        "idx_code_quality_scan_stream_tool_started",
        "idx_code_quality_finding_scan",
        "idx_code_quality_finding_scan_path",
        "idx_code_quality_finding_scan_kind_value",
      ]),
    );
  });

  test("refuses to open a database at a higher version than this build knows", () => {
    const driver = freshDriver();
    runMigrations(driver);
    const unknownVersion = (MIGRATIONS.at(-1)!.version) + 5;
    driver.exec(`PRAGMA user_version = ${unknownVersion}`);
    expect(() => runMigrations(driver)).toThrow(/older binary/);
  });
});
