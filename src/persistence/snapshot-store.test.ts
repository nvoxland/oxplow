import { describe, expect, test } from "bun:test";
import { mkdtempSync, writeFileSync, rmSync, unlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { SnapshotStore, computeVersionHash } from "./snapshot-store.js";
import { StreamStore } from "./stream-store.js";
import { getStateDatabase } from "./state-db.js";

function backdateSnapshot(projectDir: string, snapshotId: string, daysAgo: number) {
  const when = new Date(Date.now() - daysAgo * 86400_000).toISOString();
  getStateDatabase(projectDir).run(
    "UPDATE file_snapshot SET created_at = ? WHERE id = ?",
    when,
    snapshotId,
  );
}

function seed() {
  const projectDir = mkdtempSync(join(tmpdir(), "newde-snap-"));
  const worktreePath = projectDir;
  const streams = new StreamStore(projectDir);
  const stream = streams.create({
    title: "snap",
    branch: "main",
    worktreePath,
    projectBase: "snap",
  });
  const store = new SnapshotStore(projectDir);
  return { projectDir, worktreePath, store, streamId: stream.id };
}

describe("SnapshotStore", () => {
  test("writeBlob is deterministic and dedupes", () => {
    const { store, projectDir } = seed();
    const a = store.writeBlob(Buffer.from("hello world"));
    const b = store.writeBlob(Buffer.from("hello world"));
    expect(a).toBe(b);
    expect(a).toMatch(/^[0-9a-f]{64}$/);
    expect(store.readBlob(a).toString()).toBe("hello world");
    rmSync(projectDir, { recursive: true, force: true });
  });

  test("flushSnapshot creates a row and returns the id", () => {
    const { store, worktreePath, streamId, projectDir } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "hello");
    const result = store.flushSnapshot({
      source: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    expect(result.created).toBe(true);
    expect(result.id).toMatch(/^snap-/);
    const snap = store.getSnapshot(result.id);
    expect(snap?.version_hash).toBe(result.versionHash);
    expect(snap?.source).toBe("turn-start");
    rmSync(projectDir, { recursive: true, force: true });
  });

  test("flushSnapshot dedupes when version_hash matches the latest", () => {
    const { store, worktreePath, streamId, projectDir } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "hello");
    const first = store.flushSnapshot({
      source: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    const second = store.flushSnapshot({
      source: "task-start",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    expect(second.created).toBe(false);
    expect(second.id).toBe(first.id);
    rmSync(projectDir, { recursive: true, force: true });
  });

  test("flushSnapshot creates a new row when a file changes", () => {
    const { store, worktreePath, streamId, projectDir } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "hello");
    const first = store.flushSnapshot({
      source: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    writeFileSync(join(worktreePath, "a.txt"), "world");
    const second = store.flushSnapshot({
      source: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    expect(second.created).toBe(true);
    expect(second.id).not.toBe(first.id);
    expect(second.versionHash).not.toBe(first.versionHash);
    rmSync(projectDir, { recursive: true, force: true });
  });

  test("flushSnapshot carries forward non-dirty entries", () => {
    const { store, worktreePath, streamId, projectDir } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "hello");
    writeFileSync(join(worktreePath, "b.txt"), "stable");
    store.flushSnapshot({
      source: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt", "b.txt"],
    });
    writeFileSync(join(worktreePath, "a.txt"), "changed");
    const second = store.flushSnapshot({
      source: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    expect(second.created).toBe(true);
    const entries = store.loadManifestEntries(second.id);
    expect(entries["b.txt"]?.state).toBe("present");
    expect(entries["a.txt"]?.state).toBe("present");
    rmSync(projectDir, { recursive: true, force: true });
  });

  test("flushSnapshot drops entries whose files were deleted", () => {
    const { store, worktreePath, streamId, projectDir } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "hello");
    store.flushSnapshot({
      source: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    unlinkSync(join(worktreePath, "a.txt"));
    const second = store.flushSnapshot({
      source: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    expect(second.created).toBe(true);
    const entries = store.loadManifestEntries(second.id);
    expect(entries["a.txt"]?.state).toBe("deleted");
    rmSync(projectDir, { recursive: true, force: true });
  });

  test("a deleted file stays tombstoned only in the first snapshot after deletion", () => {
    const { store, worktreePath, streamId, projectDir } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "hello");
    store.flushSnapshot({
      source: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    unlinkSync(join(worktreePath, "a.txt"));
    const second = store.flushSnapshot({
      source: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    expect(store.loadManifestEntries(second.id)["a.txt"]?.state).toBe("deleted");
    writeFileSync(join(worktreePath, "b.txt"), "other");
    const third = store.flushSnapshot({
      source: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt", "b.txt"],
    });
    expect(store.loadManifestEntries(third.id)["a.txt"]).toBeUndefined();
    rmSync(projectDir, { recursive: true, force: true });
  });

  test("getSnapshotPairDiff returns before/after content", () => {
    const { store, worktreePath, streamId, projectDir } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "v1");
    const first = store.flushSnapshot({
      source: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    writeFileSync(join(worktreePath, "a.txt"), "v2");
    const second = store.flushSnapshot({
      source: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    const diff = store.getSnapshotPairDiff(first.id, second.id, "a.txt");
    expect(diff.before).toBe("v1");
    expect(diff.after).toBe("v2");
    rmSync(projectDir, { recursive: true, force: true });
  });

  test("getSnapshotSummary uses preceding snapshot as baseline", () => {
    const { store, worktreePath, streamId, projectDir } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "hello");
    store.flushSnapshot({
      source: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    writeFileSync(join(worktreePath, "b.txt"), "new");
    writeFileSync(join(worktreePath, "a.txt"), "changed");
    const second = store.flushSnapshot({
      source: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt", "b.txt"],
    });
    const summary = store.getSnapshotSummary(second.id);
    expect(summary?.counts).toEqual({ created: 1, updated: 1, deleted: 0 });
    rmSync(projectDir, { recursive: true, force: true });
  });

  test("listSnapshotsForStream labels snapshots linked to a work_item_effort with the task title", () => {
    const { store, worktreePath, streamId, projectDir } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "a");
    store.flushSnapshot({ source: "task-start", streamId, worktreePath, dirtyPaths: ["a.txt"] });
    writeFileSync(join(worktreePath, "a.txt"), "b");
    const endSnap = store.flushSnapshot({
      source: "task-end",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });

    // Simulate: an effort has this snapshot as its end_snapshot_id.
    const db = getStateDatabase(projectDir);
    // Need a batch + work_item for the effort FK. Use the BatchStore API via
    // direct SQL to keep the test independent of it.
    const batchRows = db.all<{ id: string }>(`SELECT id FROM batches LIMIT 1`);
    if (batchRows.length === 0) {
      // The seed doesn't create a batch — insert a minimal one for the FK.
      db.run(
        `INSERT INTO batches (id, stream_id, title, status, sort_index, pane_target, auto_commit, created_at, updated_at)
         VALUES ('b-test', ?, 'T', 'active', 0, '', 0, ?, ?)`,
        streamId,
        new Date().toISOString(),
        new Date().toISOString(),
      );
    }
    const batchId = batchRows[0]?.id ?? "b-test";
    db.run(
      `INSERT INTO work_items (id, batch_id, kind, title, status, priority, created_by, created_at, updated_at)
       VALUES ('wi-ship-it', ?, 'task', 'Ship the thing', 'human_check', 'medium', 'user', ?, ?)`,
      batchId,
      new Date().toISOString(),
      new Date().toISOString(),
    );
    db.run(
      `INSERT INTO work_item_effort (id, work_item_id, started_at, ended_at, end_snapshot_id)
       VALUES ('eff-1', 'wi-ship-it', ?, ?, ?)`,
      new Date().toISOString(),
      new Date().toISOString(),
      endSnap.id,
    );

    const listed = store.listSnapshotsForStream(streamId);
    expect(listed).toHaveLength(1);
    expect(listed[0]!.id).toBe(endSnap.id);
    expect(listed[0]!.label).toBe("Ship the thing — end");
    expect(listed[0]!.label_kind).toBe("task");
    rmSync(projectDir, { recursive: true, force: true });
  });

  test("listSnapshotsForStream hides the initial baseline (nothing to diff against)", () => {
    const { store, worktreePath, streamId, projectDir } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "a");
    const first = store.flushSnapshot({
      source: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    // Only the baseline exists — nothing should be listed.
    expect(store.listSnapshotsForStream(streamId)).toEqual([]);

    writeFileSync(join(worktreePath, "a.txt"), "b");
    const second = store.flushSnapshot({
      source: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    // The baseline (`first`) is still excluded; only the second shows up.
    const listed = store.listSnapshotsForStream(streamId);
    expect(listed.map((s) => s.id)).toEqual([second.id]);
    // But the baseline is still resolvable (it's the previous for `second`).
    expect(store.getPreviousSnapshot(second.id)?.id).toBe(first.id);
    rmSync(projectDir, { recursive: true, force: true });
  });

  test("cleanupOldSnapshots prunes ancient rows but keeps the latest", () => {
    const { store, worktreePath, streamId, projectDir } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "a");
    const first = store.flushSnapshot({
      source: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    writeFileSync(join(worktreePath, "a.txt"), "b");
    const second = store.flushSnapshot({
      source: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    backdateSnapshot(projectDir, first.id, 30);
    const stats = store.cleanupOldSnapshots(7);
    expect(stats.snapshotsDeleted).toBe(1);
    expect(store.getSnapshot(first.id)).toBeNull();
    expect(store.getSnapshot(second.id)).not.toBeNull();
    rmSync(projectDir, { recursive: true, force: true });
  });

  test("computeVersionHash ignores mtime but reacts to content/size/state", () => {
    const a = computeVersionHash([
      ["a.txt", { hash: "x", mtime_ms: 1, size: 3, state: "present" }],
    ]);
    const sameContentDifferentMtime = computeVersionHash([
      ["a.txt", { hash: "x", mtime_ms: 999, size: 3, state: "present" }],
    ]);
    const differentHash = computeVersionHash([
      ["a.txt", { hash: "y", mtime_ms: 1, size: 3, state: "present" }],
    ]);
    expect(a).toBe(sameContentDifferentMtime);
    expect(a).not.toBe(differentHash);
  });
});
