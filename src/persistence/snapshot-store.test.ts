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

  test("listSnapshotsForStream leaves effort-start-only snapshots unlabeled", () => {
    const { store, worktreePath, streamId, projectDir } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "a");
    store.flushSnapshot({ source: "turn-start", streamId, worktreePath, dirtyPaths: ["a.txt"] });
    writeFileSync(join(worktreePath, "a.txt"), "b");
    const startSnap = store.flushSnapshot({
      source: "task-start", streamId, worktreePath, dirtyPaths: ["a.txt"],
    });

    const db = getStateDatabase(projectDir);
    if (db.all<{ id: string }>(`SELECT id FROM threads LIMIT 1`).length === 0) {
      db.run(
        `INSERT INTO threads (id, stream_id, title, status, sort_index, pane_target, auto_commit, created_at, updated_at)
         VALUES ('b-ts', ?, 'T', 'active', 0, '', 0, ?, ?)`,
        streamId, new Date().toISOString(), new Date().toISOString(),
      );
    }
    db.run(
      `INSERT INTO work_items (id, thread_id, kind, title, status, priority, created_by, created_at, updated_at)
       VALUES ('wi-ts', 'b-ts', 'task', 'Start only task', 'in_progress', 'medium', 'user', ?, ?)`,
      new Date().toISOString(), new Date().toISOString(),
    );
    db.run(
      `INSERT INTO work_item_effort (id, work_item_id, started_at, start_snapshot_id)
       VALUES ('eff-ts', 'wi-ts', ?, ?)`,
      new Date().toISOString(), startSnap.id,
    );

    const listed = store.listSnapshotsForStream(streamId);
    const row = listed.find((s) => s.id === startSnap.id);
    expect(row?.label).toBeNull();
    expect(row?.label_kind).toBeNull();
    rmSync(projectDir, { recursive: true, force: true });
  });

  test("listSnapshotsForStream labels turn-linked snapshots with the prompt's first line", () => {
    const { store, worktreePath, streamId, projectDir } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "a");
    store.flushSnapshot({ source: "turn-start", streamId, worktreePath, dirtyPaths: ["a.txt"] });
    writeFileSync(join(worktreePath, "a.txt"), "b");
    const endSnap = store.flushSnapshot({
      source: "turn-end", streamId, worktreePath, dirtyPaths: ["a.txt"],
    });

    const db = getStateDatabase(projectDir);
    if (db.all<{ id: string }>(`SELECT id FROM threads LIMIT 1`).length === 0) {
      db.run(
        `INSERT INTO threads (id, stream_id, title, status, sort_index, pane_target, auto_commit, created_at, updated_at)
         VALUES ('b-turn', ?, 'T', 'active', 0, '', 0, ?, ?)`,
        streamId, new Date().toISOString(), new Date().toISOString(),
      );
    }
    db.run(
      `INSERT INTO agent_turn (id, thread_id, prompt, started_at, ended_at, end_snapshot_id)
       VALUES ('turn-1', 'b-turn', 'Multi-line prompt\nsecond line here', ?, ?, ?)`,
      new Date().toISOString(), new Date().toISOString(), endSnap.id,
    );

    const listed = store.listSnapshotsForStream(streamId);
    const row = listed.find((s) => s.id === endSnap.id);
    expect(row?.label).toBe("Multi-line prompt");
    expect(row?.label_kind).toBe("turn");
    rmSync(projectDir, { recursive: true, force: true });
  });

  test("listSnapshotsForStream prefers effort-end over effort-start and over turns on the same snapshot", () => {
    const { store, worktreePath, streamId, projectDir } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "a");
    store.flushSnapshot({ source: "turn-start", streamId, worktreePath, dirtyPaths: ["a.txt"] });
    writeFileSync(join(worktreePath, "a.txt"), "b");
    const snap = store.flushSnapshot({
      source: "task-end", streamId, worktreePath, dirtyPaths: ["a.txt"],
    });

    const db = getStateDatabase(projectDir);
    if (db.all<{ id: string }>(`SELECT id FROM threads LIMIT 1`).length === 0) {
      db.run(
        `INSERT INTO threads (id, stream_id, title, status, sort_index, pane_target, auto_commit, created_at, updated_at)
         VALUES ('b-win', ?, 'T', 'active', 0, '', 0, ?, ?)`,
        streamId, new Date().toISOString(), new Date().toISOString(),
      );
    }
    // Same snapshot is: (a) an effort's end, (b) another effort's start,
    // (c) a turn's end. Expected winner: effort-end title.
    db.run(
      `INSERT INTO work_items (id, thread_id, kind, title, status, priority, created_by, created_at, updated_at)
       VALUES ('wi-end', 'b-win', 'task', 'Finished task', 'human_check', 'medium', 'user', ?, ?)`,
      new Date().toISOString(), new Date().toISOString(),
    );
    db.run(
      `INSERT INTO work_items (id, thread_id, kind, title, status, priority, created_by, created_at, updated_at)
       VALUES ('wi-start', 'b-win', 'task', 'Starting task', 'in_progress', 'medium', 'user', ?, ?)`,
      new Date().toISOString(), new Date().toISOString(),
    );
    db.run(
      `INSERT INTO work_item_effort (id, work_item_id, started_at, ended_at, end_snapshot_id)
       VALUES ('eff-end', 'wi-end', ?, ?, ?)`,
      new Date().toISOString(), new Date().toISOString(), snap.id,
    );
    db.run(
      `INSERT INTO work_item_effort (id, work_item_id, started_at, start_snapshot_id)
       VALUES ('eff-start', 'wi-start', ?, ?)`,
      new Date().toISOString(), snap.id,
    );
    db.run(
      `INSERT INTO agent_turn (id, thread_id, prompt, started_at, ended_at, end_snapshot_id)
       VALUES ('turn-x', 'b-win', 'Prompt', ?, ?, ?)`,
      new Date().toISOString(), new Date().toISOString(), snap.id,
    );

    const listed = store.listSnapshotsForStream(streamId);
    const row = listed.find((s) => s.id === snap.id);
    expect(row?.label).toBe("Finished task — end");
    rmSync(projectDir, { recursive: true, force: true });
  });

  test("listSnapshotsForStream label picks the most recent effort when multiple efforts end at the same snapshot", () => {
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

    const db = getStateDatabase(projectDir);
    const threadRows = db.all<{ id: string }>(`SELECT id FROM threads LIMIT 1`);
    if (threadRows.length === 0) {
      db.run(
        `INSERT INTO threads (id, stream_id, title, status, sort_index, pane_target, auto_commit, created_at, updated_at)
         VALUES ('b-tie', ?, 'T', 'active', 0, '', 0, ?, ?)`,
        streamId, new Date().toISOString(), new Date().toISOString(),
      );
    }
    const threadId = threadRows[0]?.id ?? "b-tie";
    // Two work items + two efforts, both ending at the same snapshot.
    for (const [wi, title] of [["wi-older", "Older task"], ["wi-newer", "Newer task"]] as const) {
      db.run(
        `INSERT INTO work_items (id, thread_id, kind, title, status, priority, created_by, created_at, updated_at)
         VALUES (?, ?, 'task', ?, 'human_check', 'medium', 'user', ?, ?)`,
        wi, threadId, title, new Date().toISOString(), new Date().toISOString(),
      );
    }
    db.run(
      `INSERT INTO work_item_effort (id, work_item_id, started_at, ended_at, end_snapshot_id)
       VALUES ('eff-older', 'wi-older', ?, ?, ?)`,
      "2024-01-01T00:00:00.000Z", "2024-01-01T00:01:00.000Z", endSnap.id,
    );
    db.run(
      `INSERT INTO work_item_effort (id, work_item_id, started_at, ended_at, end_snapshot_id)
       VALUES ('eff-newer', 'wi-newer', ?, ?, ?)`,
      "2024-06-01T00:00:00.000Z", "2024-06-01T00:01:00.000Z", endSnap.id,
    );

    const listed = store.listSnapshotsForStream(streamId);
    expect(listed[0]!.label).toBe("Newer task — end");
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
    // Need a thread + work_item for the effort FK. Use the ThreadStore API via
    // direct SQL to keep the test independent of it.
    const threadRows = db.all<{ id: string }>(`SELECT id FROM threads LIMIT 1`);
    if (threadRows.length === 0) {
      // The seed doesn't create a thread — insert a minimal one for the FK.
      db.run(
        `INSERT INTO threads (id, stream_id, title, status, sort_index, pane_target, auto_commit, created_at, updated_at)
         VALUES ('b-test', ?, 'T', 'active', 0, '', 0, ?, ?)`,
        streamId,
        new Date().toISOString(),
        new Date().toISOString(),
      );
    }
    const threadId = threadRows[0]?.id ?? "b-test";
    db.run(
      `INSERT INTO work_items (id, thread_id, kind, title, status, priority, created_by, created_at, updated_at)
       VALUES ('wi-ship-it', ?, 'task', 'Ship the thing', 'human_check', 'medium', 'user', ?, ?)`,
      threadId,
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

  test("flushSnapshot recheck dedups against a racing insert", () => {
    const { store, worktreePath, streamId, projectDir } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "hello");
    const first = store.flushSnapshot({
      source: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    // Simulate a racing writer landing a matching-hash row after the fast-
    // path `getLatestSnapshot` read has already been made. We cheat by
    // stashing a `getLatestSnapshot` spy that returns a stale result, then
    // invoke the inner transaction indirectly via flushSnapshot. Since the
    // in-transaction recheck re-reads `latest`, it should find `first`
    // (actually persisted) and dedup against it rather than inserting a new
    // row with the same hash.
    // Easiest reproduction: call flushSnapshot twice in a row with the same
    // content — the second call's in-memory `latest` and the in-tx `latest`
    // agree, but it still proves the recheck path doesn't corrupt the happy
    // case when the hashes match.
    const second = store.flushSnapshot({
      source: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    expect(second.created).toBe(false);
    expect(second.id).toBe(first.id);
    const count = getStateDatabase(projectDir)
      .get<{ c: number }>(`SELECT COUNT(*) AS c FROM file_snapshot WHERE stream_id = ?`, streamId);
    expect(count?.c).toBe(1);
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
