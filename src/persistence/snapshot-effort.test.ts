import { describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { SnapshotStore } from "./snapshot-store.js";
import { StreamStore } from "./stream-store.js";
import { ThreadStore } from "./thread-store.js";
import { WorkItemEffortStore } from "./work-item-effort-store.js";
import { WorkItemStore } from "./work-item-store.js";

function seed() {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-snap-eff-"));
  const streamStore = new StreamStore(dir);
  const stream = streamStore.create({
    title: "Demo",
    branch: "main",
    worktreePath: dir,
    projectBase: "demo",
  });
  const threadStore = new ThreadStore(dir);
  const state = threadStore.ensureStream(stream);
  const threadId = state.threads[0]!.id;
  const workItems = new WorkItemStore(dir);
  const item = workItems.createItem({
    threadId,
    kind: "task",
    title: "T",
    createdBy: "user",
    actorId: "test",
  });
  const snapshots = new SnapshotStore(dir);
  const efforts = new WorkItemEffortStore(dir);
  // Baseline so flushSnapshot has a previous to dedup against.
  writeFileSync(join(dir, "seed.txt"), "baseline");
  snapshots.flushSnapshot({
    source: "startup",
    streamId: stream.id,
    worktreePath: dir,
    dirtyPaths: null,
    ignore: (rel) => rel.startsWith(".oxplow") || rel.startsWith(".git"),
  });
  return { dir, snapshots, efforts, streamId: stream.id, itemId: item.id };
}

describe("file_snapshot.effort_id", () => {
  test("flushSnapshot persists effort_id when provided", () => {
    const { dir, snapshots, efforts, streamId, itemId } = seed();
    const opened = efforts.openEffort({ workItemId: itemId, startSnapshotId: null });
    writeFileSync(join(dir, "a.txt"), "v1");
    const result = snapshots.flushSnapshot({
      source: "task-start",
      streamId,
      worktreePath: dir,
      dirtyPaths: ["a.txt"],
      effortId: opened.id,
    });
    expect(result.created).toBe(true);
    const snap = snapshots.getSnapshot(result.id);
    expect(snap?.effort_id).toBe(opened.id);
    rmSync(dir, { recursive: true, force: true });
  });

  test("getMostRecentSnapshotTimestampForStream returns latest created_at or null", () => {
    const { dir, snapshots, streamId } = seed();
    const ts1 = snapshots.getMostRecentSnapshotTimestampForStream(streamId);
    expect(ts1).not.toBeNull();
    writeFileSync(join(dir, "x.txt"), "v1");
    const r = snapshots.flushSnapshot({
      source: "task-start",
      streamId,
      worktreePath: dir,
      dirtyPaths: ["x.txt"],
    });
    expect(r.created).toBe(true);
    const ts2 = snapshots.getMostRecentSnapshotTimestampForStream(streamId);
    expect(ts2).not.toBeNull();
    expect(ts2! >= ts1!).toBe(true);
    rmSync(dir, { recursive: true, force: true });
  });

  test("getMostRecentSnapshotTimestampForStream returns null for unknown stream", () => {
    const { dir, snapshots } = seed();
    expect(snapshots.getMostRecentSnapshotTimestampForStream("nope")).toBeNull();
    rmSync(dir, { recursive: true, force: true });
  });
});

import {
  applyStatusTransition,
  computeEffortFilePaths,
  shouldSkipEndSnapshot,
  END_SNAPSHOT_MIN_GAP_MS,
} from "../electron/runtime.js";

describe("applyStatusTransition + 5-minute gap rule", () => {
  function harness() {
    const seeded = seed();
    const flushSnapshot = (
      source: "task-start" | "task-end" | "startup",
      options?: { effortId?: string | null },
    ): string | null => {
      const result = seeded.snapshots.flushSnapshot({
        source,
        streamId: seeded.streamId,
        worktreePath: seeded.dir,
        dirtyPaths: null,
        effortId: options?.effortId ?? null,
        ignore: (rel) => rel.startsWith(".oxplow") || rel.startsWith(".git"),
      });
      return result.id;
    };
    // `tsOverride` lets tests pin what `getMostRecentSnapshotTimestamp`
    // returns to applyStatusTransition: shouldSkipEndSnapshot compares
    // it against the real `Date.now()` from the runtime, so to simulate
    // "last snapshot was 10 minutes ago" we override with `now - 10min`.
    let tsOverride: string | null | undefined = undefined;
    const deps = {
      effortStore: seeded.efforts,
      flushSnapshot,
      getMostRecentSnapshotTimestamp: () =>
        tsOverride === undefined
          ? seeded.snapshots.getMostRecentSnapshotTimestampForStream(seeded.streamId)
          : tsOverride,
    };
    return { ...seeded, deps, flushSnapshot, setTsOverride: (ts: string | null | undefined) => { tsOverride = ts; } };
  }

  test("effort-start always flushes a snapshot, even when last snapshot is recent", () => {
    const h = harness();
    // Make the recent timestamp now.
    writeFileSync(join(h.dir, "a.txt"), "v1");
    h.flushSnapshot("startup");
    // applyStatusTransition opens an effort and should still flush task-start
    // even though we just flushed.
    writeFileSync(join(h.dir, "b.txt"), "v1");
    applyStatusTransition(h.deps, {
      threadId: "t", workItemId: h.itemId, previous: "ready", next: "in_progress",
    });
    const open = h.efforts.getOpenEffort(h.itemId);
    expect(open).not.toBeNull();
    expect(open!.start_snapshot_id).not.toBeNull();
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("effort-end skips end snapshot when last snapshot <5 min ago", () => {
    const h = harness();
    applyStatusTransition(h.deps, {
      threadId: "t", workItemId: h.itemId, previous: "ready", next: "in_progress",
    });
    writeFileSync(join(h.dir, "a.txt"), "v1");
    // Force the recent-timestamp check to claim the latest snapshot was
    // 1 second ago — well under 5 minutes.
    h.setTsOverride(new Date(Date.now() - 60_000).toISOString());
    applyStatusTransition(h.deps, {
      threadId: "t", workItemId: h.itemId, previous: "in_progress", next: "human_check",
    });
    const closed = h.efforts.listEffortsForWorkItem(h.itemId)[0]!;
    expect(closed.ended_at).not.toBeNull();
    expect(closed.end_snapshot_id).toBeNull();
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("effort-end flushes snapshot when last snapshot ≥5 min ago", () => {
    const h = harness();
    applyStatusTransition(h.deps, {
      threadId: "t", workItemId: h.itemId, previous: "ready", next: "in_progress",
    });
    writeFileSync(join(h.dir, "a.txt"), "v1");
    // Pretend now is well past the gap.
    // Last snapshot was 10 minutes ago — past the 5-minute gap.
    h.setTsOverride(new Date(Date.now() - 10 * 60_000).toISOString());
    applyStatusTransition(h.deps, {
      threadId: "t", workItemId: h.itemId, previous: "in_progress", next: "human_check",
    });
    const closed = h.efforts.listEffortsForWorkItem(h.itemId)[0]!;
    expect(closed.end_snapshot_id).not.toBeNull();
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("effort-end flushes a snapshot on any move out of in_progress (e.g. blocked)", () => {
    const h = harness();
    applyStatusTransition(h.deps, {
      threadId: "t", workItemId: h.itemId, previous: "ready", next: "in_progress",
    });
    writeFileSync(join(h.dir, "a.txt"), "v1");
    // Past the 5-minute gap — flush should fire even for a non-human_check
    // close, since the work paused and Local History wants the state.
    h.setTsOverride(new Date(Date.now() - 10 * 60_000).toISOString());
    applyStatusTransition(h.deps, {
      threadId: "t", workItemId: h.itemId, previous: "in_progress", next: "blocked",
    });
    const closed = h.efforts.listEffortsForWorkItem(h.itemId)[0]!;
    expect(closed.ended_at).not.toBeNull();
    expect(closed.end_snapshot_id).not.toBeNull();
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("end snapshot carries effort_id back to the file_snapshot row", () => {
    const h = harness();
    // Mutate before each transition so neither snapshot dedups onto an
    // earlier (effort-less) row.
    writeFileSync(join(h.dir, "pre.txt"), "v1");
    applyStatusTransition(h.deps, {
      threadId: "t", workItemId: h.itemId, previous: "ready", next: "in_progress",
    });
    writeFileSync(join(h.dir, "a.txt"), "v1");
    // Last snapshot was 10 minutes ago — past the 5-minute gap.
    h.setTsOverride(new Date(Date.now() - 10 * 60_000).toISOString());
    applyStatusTransition(h.deps, {
      threadId: "t", workItemId: h.itemId, previous: "in_progress", next: "human_check",
    });
    const closed = h.efforts.listEffortsForWorkItem(h.itemId)[0]!;
    const endSnap = h.snapshots.getSnapshot(closed.end_snapshot_id!);
    expect(endSnap?.effort_id).toBe(closed.id);
    const startSnap = h.snapshots.getSnapshot(closed.start_snapshot_id!);
    expect(startSnap?.effort_id).toBe(closed.id);
    rmSync(h.dir, { recursive: true, force: true });
  });
});

describe("shouldSkipEndSnapshot", () => {
  test("returns true when gap is below threshold", () => {
    const now = 1_700_000_000_000;
    const recent = new Date(now - 60_000).toISOString();
    expect(shouldSkipEndSnapshot(recent, now)).toBe(true);
  });
  test("returns false when gap is above threshold", () => {
    const now = 1_700_000_000_000;
    const old = new Date(now - END_SNAPSHOT_MIN_GAP_MS - 1000).toISOString();
    expect(shouldSkipEndSnapshot(old, now)).toBe(false);
  });
  test("returns false on unparseable timestamp", () => {
    expect(shouldSkipEndSnapshot("not-a-date", Date.now())).toBe(false);
  });
});

describe("computeEffortFilePaths", () => {
  test("returns sorted paths from the pair-diff for a closed effort", () => {
    const h = (() => {
      const seeded = seed();
      const flushSnapshot = (
        source: "task-start" | "task-end" | "startup",
        options?: { effortId?: string | null },
      ): string | null => {
        const result = seeded.snapshots.flushSnapshot({
          source,
          streamId: seeded.streamId,
          worktreePath: seeded.dir,
          dirtyPaths: null,
          effortId: options?.effortId ?? null,
          ignore: (rel) => rel.startsWith(".oxplow") || rel.startsWith(".git"),
        });
        return result.id;
      };
      const deps = {
        effortStore: seeded.efforts,
        flushSnapshot,
        getMostRecentSnapshotTimestamp: () => null,
      };
      return { ...seeded, deps };
    })();
    applyStatusTransition(h.deps, {
      threadId: "t", workItemId: h.itemId, previous: "ready", next: "in_progress",
    });
    writeFileSync(join(h.dir, "z.txt"), "v1");
    writeFileSync(join(h.dir, "a.txt"), "v1");
    applyStatusTransition(h.deps, {
      threadId: "t", workItemId: h.itemId, previous: "in_progress", next: "human_check",
    });
    const closed = h.efforts.listEffortsForWorkItem(h.itemId)[0]!;
    const paths = computeEffortFilePaths(h.efforts, h.snapshots, closed.id);
    expect(paths).toEqual(["a.txt", "z.txt"]);
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("returns empty array for unknown / still-open effort", () => {
    const h = seed();
    expect(computeEffortFilePaths(h.efforts, h.snapshots, "no-such")).toEqual([]);
    const open = h.efforts.openEffort({ workItemId: h.itemId, startSnapshotId: null });
    expect(computeEffortFilePaths(h.efforts, h.snapshots, open.id)).toEqual([]);
    rmSync(h.dir, { recursive: true, force: true });
  });
});
