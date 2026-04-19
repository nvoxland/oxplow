import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync, utimesSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { SnapshotStore } from "./snapshot-store.js";
import { StreamStore } from "./stream-store.js";
import { getStateDatabase } from "./state-db.js";

function backdateSnapshot(projectDir: string, store: SnapshotStore, snapshotId: string, daysAgo: number) {
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
    const { store } = seed();
    const a = store.writeBlob(Buffer.from("hello world"));
    const b = store.writeBlob(Buffer.from("hello world"));
    expect(a).toBe(b);
    expect(a).toMatch(/^[0-9a-f]{64}$/);
    expect(store.readBlob(a).toString()).toBe("hello world");
  });

  test("flushSnapshot returns null when dirtyPaths is empty", () => {
    const { store, worktreePath, streamId } = seed();
    const id = store.flushSnapshot({
      kind: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: [],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    });
    expect(id).toBeNull();
  });

  test("flushSnapshot writes blobs, manifest, and file_snapshot row", () => {
    const { store, worktreePath, streamId } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "A content");
    writeFileSync(join(worktreePath, "b.txt"), "B content");
    const id = store.flushSnapshot({
      kind: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt", "b.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    });
    expect(id).not.toBeNull();
    const snap = store.getSnapshot(id!);
    expect(snap).not.toBeNull();
    expect(snap!.kind).toBe("turn-start");
    const entries = store.loadManifestEntries(id!);
    expect(Object.keys(entries).sort()).toEqual(["a.txt", "b.txt"]);
    expect(store.readBlob(entries["a.txt"]!.hash).toString()).toBe("A content");
  });

  test("deleted files are recorded as tombstones in the manifest", () => {
    const { store, worktreePath, streamId } = seed();
    writeFileSync(join(worktreePath, "gone.txt"), "will be gone");
    const first = store.flushSnapshot({
      kind: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["gone.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    rmSync(join(worktreePath, "gone.txt"));
    const second = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["gone.txt"],
      parentSnapshotId: first,
      turnId: null,
      batchId: null,
    })!;
    const entries = store.loadManifestEntries(second);
    expect(entries["gone.txt"]).not.toBeUndefined();
    expect(entries["gone.txt"]!.deleted).toBe(true);
    expect(store.resolvePath(second, "gone.txt")).toBeNull();
    expect(store.resolvePath(first, "gone.txt")).not.toBeNull();
  });

  test("resolvePath walks parent chain", () => {
    const { store, worktreePath, streamId } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "A1");
    writeFileSync(join(worktreePath, "b.txt"), "B1");
    const s1 = store.flushSnapshot({
      kind: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt", "b.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    writeFileSync(join(worktreePath, "a.txt"), "A2");
    const s2 = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
      parentSnapshotId: s1,
      turnId: null,
      batchId: null,
    })!;
    const aHash = store.resolvePath(s2, "a.txt");
    const bHash = store.resolvePath(s2, "b.txt");
    expect(aHash).not.toBeNull();
    expect(bHash).not.toBeNull();
    expect(store.readBlob(aHash!).toString()).toBe("A2");
    expect(store.readBlob(bHash!).toString()).toBe("B1");
  });

  test("resolveEntries returns merged view of full chain", () => {
    const { store, worktreePath, streamId } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "A1");
    writeFileSync(join(worktreePath, "b.txt"), "B1");
    const s1 = store.flushSnapshot({
      kind: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt", "b.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    writeFileSync(join(worktreePath, "a.txt"), "A2");
    writeFileSync(join(worktreePath, "c.txt"), "C1");
    const s2 = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt", "c.txt"],
      parentSnapshotId: s1,
      turnId: null,
      batchId: null,
    })!;
    const entries = store.resolveEntries(s2);
    expect(Object.keys(entries).sort()).toEqual(["a.txt", "b.txt", "c.txt"]);
    expect(entries["a.txt"]!.hash).toBe(store.resolvePath(s2, "a.txt"));
  });

  test("diffPath returns before/after across snapshots", () => {
    const { store, worktreePath, streamId } = seed();
    writeFileSync(join(worktreePath, "f.txt"), "before");
    const s1 = store.flushSnapshot({
      kind: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["f.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    writeFileSync(join(worktreePath, "f.txt"), "after");
    const s2 = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["f.txt"],
      parentSnapshotId: s1,
      turnId: null,
      batchId: null,
    })!;
    const diff = store.diffPath(s1, s2, "f.txt");
    expect(diff.before).toBe("before");
    expect(diff.after).toBe("after");
  });

  test("reconcileWorktree seeds dirty list from mtime/size drift", () => {
    const { store, worktreePath, streamId } = seed();
    writeFileSync(join(worktreePath, "stable.txt"), "stable");
    writeFileSync(join(worktreePath, "changed.txt"), "v1");
    const s1 = store.flushSnapshot({
      kind: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["stable.txt", "changed.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    // Simulate the app being closed: modify a file (bump mtime).
    writeFileSync(join(worktreePath, "changed.txt"), "v2");
    const future = new Date(Date.now() + 60_000);
    utimesSync(join(worktreePath, "changed.txt"), future, future);
    // Create a new file as well.
    writeFileSync(join(worktreePath, "added.txt"), "added");
    // Delete one.
    rmSync(join(worktreePath, "stable.txt"));

    const dirty = store.reconcileWorktree(s1, worktreePath, (relpath) => {
      // Basic ignore: skip node_modules, .git, .newde
      return /^(node_modules|\.git|\.newde)(\/|$)/.test(relpath);
    });
    expect(dirty.sort()).toEqual(["added.txt", "changed.txt", "stable.txt"]);
  });

  test("reconcileWorktree skips unchanged files by mtime+size", () => {
    const { store, worktreePath, streamId } = seed();
    writeFileSync(join(worktreePath, "stable.txt"), "stable");
    const s1 = store.flushSnapshot({
      kind: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["stable.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    // Don't touch the file. Reconcile should return nothing.
    const dirty = store.reconcileWorktree(s1, worktreePath, (rel) => rel.startsWith(".newde"));
    expect(dirty).toEqual([]);
  });

  test("getSnapshotSummary classifies entries as A/M/D", () => {
    const { store, worktreePath, streamId } = seed();
    writeFileSync(join(worktreePath, "keep.txt"), "keep v1");
    writeFileSync(join(worktreePath, "gone.txt"), "gone");
    const s1 = store.flushSnapshot({
      kind: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["keep.txt", "gone.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    writeFileSync(join(worktreePath, "keep.txt"), "keep v2");
    writeFileSync(join(worktreePath, "new.txt"), "new");
    rmSync(join(worktreePath, "gone.txt"));
    const s2 = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["keep.txt", "new.txt", "gone.txt"],
      parentSnapshotId: s1,
      turnId: null,
      batchId: null,
    })!;
    const summary = store.getSnapshotSummary(s2);
    expect(summary).not.toBeNull();
    expect(summary!.counts).toEqual({ created: 1, updated: 1, deleted: 1 });
    expect(summary!.files["new.txt"]!.kind).toBe("created");
    expect(summary!.files["keep.txt"]!.kind).toBe("updated");
    expect(summary!.files["gone.txt"]!.kind).toBe("deleted");
  });

  test("getSnapshotFileDiff walks parent chain for before", () => {
    const { store, worktreePath, streamId } = seed();
    writeFileSync(join(worktreePath, "f.txt"), "v1");
    const s1 = store.flushSnapshot({
      kind: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["f.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    writeFileSync(join(worktreePath, "f.txt"), "v2");
    const s2 = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["f.txt"],
      parentSnapshotId: s1,
      turnId: null,
      batchId: null,
    })!;
    const diff = store.getSnapshotFileDiff(s2, "f.txt");
    expect(diff.before).toBe("v1");
    expect(diff.after).toBe("v2");
  });

  test("getSnapshotFileDiff returns null before for brand-new paths", () => {
    const { store, worktreePath, streamId } = seed();
    writeFileSync(join(worktreePath, "fresh.txt"), "fresh content");
    const s1 = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["fresh.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    const diff = store.getSnapshotFileDiff(s1, "fresh.txt");
    expect(diff.before).toBeNull();
    expect(diff.after).toBe("fresh content");
  });

  test("cleanupOldSnapshots prunes stale snapshots, preserves per-stream latest, GCs blobs", () => {
    const { store, worktreePath, streamId } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "A1");
    writeFileSync(join(worktreePath, "b.txt"), "B1");

    // Old snapshot: created 30 days ago. Forged via manifest rewrite below.
    const old = store.flushSnapshot({
      kind: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    backdateSnapshot(worktreePath, store, old, 30);

    // Old child snapshot (also stale), with a different content blob that
    // nothing newer references — should be GC'd.
    writeFileSync(join(worktreePath, "a.txt"), "A-stale");
    const oldChild = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
      parentSnapshotId: old,
      turnId: null,
      batchId: null,
    })!;
    backdateSnapshot(worktreePath, store, oldChild, 20);

    // Recent snapshot — should be kept because it's newer than the window.
    writeFileSync(join(worktreePath, "a.txt"), "A-fresh");
    writeFileSync(join(worktreePath, "b.txt"), "B-fresh");
    const fresh = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt", "b.txt"],
      parentSnapshotId: oldChild,
      turnId: null,
      batchId: null,
    })!;

    // Point the stream at the freshest snapshot so the "latest per stream"
    // guard kicks in if we ever delete it (sanity check).
    const { StreamStore } = require("./stream-store.js") as typeof import("./stream-store.js");
    const streams = new StreamStore(join(worktreePath));
    streams.setCurrentSnapshotId(streamId, fresh);

    const result = store.cleanupOldSnapshots(7);
    expect(result.snapshotsDeleted).toBe(2);
    // Fresh snapshot's blobs are still present; the stale A-stale blob was
    // unreferenced (nothing newer pointed at it) and should be gone.
    expect(store.getSnapshot(old)).toBeNull();
    expect(store.getSnapshot(oldChild)).toBeNull();
    expect(store.getSnapshot(fresh)).not.toBeNull();
    // A-fresh and B-fresh blobs must still be readable via resolution.
    expect(store.resolvePath(fresh, "a.txt")).not.toBeNull();
    expect(store.resolvePath(fresh, "b.txt")).not.toBeNull();
    expect(result.blobsDeleted).toBeGreaterThan(0);
  });

  test("cleanupOldSnapshots always keeps the most recent snapshot per stream, even if it's older than the window", () => {
    const { store, worktreePath, streamId } = seed();
    writeFileSync(join(worktreePath, "lonely.txt"), "only one");
    const only = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["lonely.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    backdateSnapshot(worktreePath, store, only, 30);

    const result = store.cleanupOldSnapshots(7);
    expect(result.snapshotsDeleted).toBe(0);
    expect(store.getSnapshot(only)).not.toBeNull();
  });

  test("cleanupOldSnapshots with retention 0 is a no-op", () => {
    const { store, worktreePath, streamId } = seed();
    writeFileSync(join(worktreePath, "x.txt"), "x");
    const id = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["x.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    backdateSnapshot(worktreePath, store, id, 365);

    const result = store.cleanupOldSnapshots(0);
    expect(result.snapshotsDeleted).toBe(0);
    expect(result.blobsDeleted).toBe(0);
  });

  test("getSnapshotPairDiff diffs arbitrary snapshots", () => {
    const { store, worktreePath, streamId } = seed();
    writeFileSync(join(worktreePath, "f.txt"), "first");
    const s1 = store.flushSnapshot({
      kind: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["f.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    writeFileSync(join(worktreePath, "f.txt"), "second");
    const s2 = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["f.txt"],
      parentSnapshotId: s1,
      turnId: null,
      batchId: null,
    })!;
    writeFileSync(join(worktreePath, "f.txt"), "third");
    const s3 = store.flushSnapshot({
      kind: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["f.txt"],
      parentSnapshotId: s2,
      turnId: null,
      batchId: null,
    })!;
    const diff = store.getSnapshotPairDiff(s1, s3, "f.txt");
    expect(diff.before).toBe("first");
    expect(diff.after).toBe("third");
  });
});
