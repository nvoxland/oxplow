import { describe, expect, test } from "bun:test";
import { mkdtempSync, writeFileSync, utimesSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { SnapshotStore } from "./snapshot-store.js";
import { StreamStore } from "./stream-store.js";
import { BatchStore } from "./batch-store.js";
import { TurnStore } from "./turn-store.js";
import { getStateDatabase } from "./state-db.js";

function backdateSnapshot(projectDir: string, _store: SnapshotStore, snapshotId: string, daysAgo: number) {
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
    expect(entries["a.txt"]!.state).toBe("present");
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
    expect(entries["gone.txt"]!.state).toBe("deleted");
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

  test("oversize files record state='oversize' with stat but no blob", () => {
    const { store, worktreePath, streamId } = seed();
    store.setMaxFileBytes(16);
    writeFileSync(join(worktreePath, "big.bin"), Buffer.alloc(32, 0xAB));
    const id = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["big.bin"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    const entries = store.loadManifestEntries(id);
    expect(entries["big.bin"]!.state).toBe("oversize");
    expect(entries["big.bin"]!.hash).toBe("");
    expect(entries["big.bin"]!.size).toBe(32);
    // Diff should return null for content but the summary still classifies
    // it as created on first appearance.
    const diff = store.getSnapshotFileDiff(id, "big.bin");
    expect(diff.before).toBeNull();
    expect(diff.after).toBeNull();
  });

  test("reconcileWorktree detects oversize files that changed size/mtime", () => {
    const { store, worktreePath, streamId } = seed();
    store.setMaxFileBytes(16);
    writeFileSync(join(worktreePath, "big.bin"), Buffer.alloc(32, 0xAA));
    const s1 = store.flushSnapshot({
      kind: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["big.bin"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    // Grow it — still oversize, but stats change.
    writeFileSync(join(worktreePath, "big.bin"), Buffer.alloc(48, 0xBB));
    const future = new Date(Date.now() + 60_000);
    utimesSync(join(worktreePath, "big.bin"), future, future);
    const dirty = store.reconcileWorktree(s1, worktreePath, (rel) => rel.startsWith(".newde"));
    expect(dirty).toEqual(["big.bin"]);
  });

  test("cleanupOldSnapshots keeps a snapshot pinned by streams.current_snapshot_id even when newer exists", () => {
    const { store, worktreePath, streamId, projectDir } = seed();
    writeFileSync(join(worktreePath, "x.txt"), "v1");
    const pinned = store.flushSnapshot({
      kind: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["x.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    backdateSnapshot(projectDir, store, pinned, 30);
    writeFileSync(join(worktreePath, "x.txt"), "v2");
    const latest = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["x.txt"],
      parentSnapshotId: pinned,
      turnId: null,
      batchId: null,
    })!;
    backdateSnapshot(projectDir, store, latest, 20);

    // Manually pin the OLDER one so the "defensive second pin" path kicks in.
    const streams = new StreamStore(projectDir);
    streams.setCurrentSnapshotId(streamId, pinned);

    const result = store.cleanupOldSnapshots(7);
    // `latest` is the MAX(created_at) for the stream (kept by primary pin);
    // `pinned` is preserved only by the current_snapshot_id guard.
    expect(store.getSnapshot(pinned)).not.toBeNull();
    expect(store.getSnapshot(latest)).not.toBeNull();
    expect(result.snapshotsDeleted).toBe(0);
  });

  test("resolveEntries survives a circular parent chain without looping", () => {
    const { store, worktreePath, streamId, projectDir } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "A");
    const a = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    // Forge a cycle: point a at itself. Defensive code should detect this
    // and stop walking rather than hang.
    getStateDatabase(projectDir).run(
      "UPDATE file_snapshot SET parent_snapshot_id = ? WHERE id = ?",
      a,
      a,
    );
    const entries = store.resolveEntries(a);
    expect(Object.keys(entries)).toEqual(["a.txt"]);
  });

  test("diffPath returns beforeState/afterState matching entry states", () => {
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
    rmSync(join(worktreePath, "f.txt"));
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
    expect(diff.beforeState).toBe("present");
    expect(diff.after).toBeNull();
    expect(diff.afterState).toBe("deleted");
  });

  test("oversize side of diff reports state without content", () => {
    const { store, worktreePath, streamId } = seed();
    store.setMaxFileBytes(16);
    writeFileSync(join(worktreePath, "big.bin"), Buffer.alloc(32, 0xAB));
    const id = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["big.bin"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    const diff = store.getSnapshotFileDiff(id, "big.bin");
    expect(diff.after).toBeNull();
    expect(diff.afterState).toBe("oversize");
    expect(diff.beforeState).toBe("absent");
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

  test("listSnapshotsForStream includes turn_prompt via JOIN", () => {
    const { store, worktreePath, streamId, projectDir } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "content");

    // Create a snapshot without a turn — turn_prompt should be null.
    const s1 = store.flushSnapshot({
      kind: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;

    // Set up a batch and turn using the real stores to satisfy FK constraints.
    const batchStore = new BatchStore(projectDir);
    const stream = new StreamStore(projectDir).get(streamId)!;
    const batchState = batchStore.ensureStream(stream);
    const batchId = batchState.batches[0]!.id;
    const turns = new TurnStore(projectDir);
    const turn = turns.openTurn({ batchId, prompt: "the agent prompt text" });

    // Create a snapshot linked to that turn.
    writeFileSync(join(worktreePath, "a.txt"), "content v2");
    const s2 = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
      parentSnapshotId: s1,
      turnId: turn.id,
      batchId,
    })!;

    const list = store.listSnapshotsForStream(streamId);
    const snap1 = list.find((s) => s.id === s1)!;
    const snap2 = list.find((s) => s.id === s2)!;

    expect(snap1).not.toBeUndefined();
    expect(snap1.turn_prompt).toBeNull();
    expect(snap2).not.toBeUndefined();
    expect(snap2.turn_prompt).toBe("the agent prompt text");
  });

  test("flushSnapshot skips tombstone for a path the parent never had", () => {
    const { store, worktreePath, streamId } = seed();
    writeFileSync(join(worktreePath, "real.txt"), "real content");
    const s1 = store.flushSnapshot({
      kind: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["real.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    // "ghost.txt" doesn't exist on disk and wasn't in the parent. A hook
    // reporting a delete for a never-existed path would otherwise write an
    // empty tombstone row; we want it skipped entirely.
    const s2 = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["ghost.txt"],
      parentSnapshotId: s1,
      turnId: null,
      batchId: null,
    });
    // Only "ghost.txt" was dirty, and it should be skipped → no snapshot.
    expect(s2).toBeNull();
  });

  test("flushSnapshot skips redundant tombstone when parent already has one", () => {
    const { store, worktreePath, streamId } = seed();
    writeFileSync(join(worktreePath, "gone.txt"), "doomed");
    const s1 = store.flushSnapshot({
      kind: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["gone.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    rmSync(join(worktreePath, "gone.txt"));
    // First deletion — parent has "present" entry, tombstone should be kept.
    const s2 = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["gone.txt"],
      parentSnapshotId: s1,
      turnId: null,
      batchId: null,
    })!;
    expect(store.loadManifestEntries(s2)["gone.txt"]!.state).toBe("deleted");
    // Second flush claiming same path is still deleted. Parent already has
    // a tombstone → should skip; with nothing else dirty, snapshot is null.
    const s3 = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["gone.txt"],
      parentSnapshotId: s2,
      turnId: null,
      batchId: null,
    });
    expect(s3).toBeNull();
  });

  test("flushSnapshot keeps other entries when a sibling tombstone is filtered out", () => {
    const { store, worktreePath, streamId } = seed();
    writeFileSync(join(worktreePath, "keeper.txt"), "stays");
    const s1 = store.flushSnapshot({
      kind: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["keeper.txt"],
      parentSnapshotId: null,
      turnId: null,
      batchId: null,
    })!;
    // One real change, plus a bogus delete of a never-existed path.
    writeFileSync(join(worktreePath, "keeper.txt"), "stays v2");
    const s2 = store.flushSnapshot({
      kind: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["keeper.txt", "phantom.txt"],
      parentSnapshotId: s1,
      turnId: null,
      batchId: null,
    })!;
    const entries = store.loadManifestEntries(s2);
    expect(Object.keys(entries).sort()).toEqual(["keeper.txt"]);
    expect(entries["keeper.txt"]!.state).toBe("present");
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
