import { describe, expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ThreadStore } from "./thread-store.js";
import { SnapshotStore } from "./snapshot-store.js";
import { StreamStore } from "./stream-store.js";
import { WorkItemStore } from "./work-item-store.js";
import { WorkItemEffortStore } from "./work-item-effort-store.js";

function seed() {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-effort-"));
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
  return { dir, workItems, snapshots, efforts, threadId, itemId: item.id, streamId: stream.id };
}

function createSnapshot(dir: string, snapshots: SnapshotStore, streamId: string, name: string, content: string): string {
  writeFileSync(join(dir, `${name}.txt`), content);
  const result = snapshots.flushSnapshot({
    source: "task-start",
    streamId,
    worktreePath: dir,
    dirtyPaths: [`${name}.txt`],
  });
  return result.id;
}

describe("WorkItemEffortStore", () => {
  test("openEffort + closeEffort roundtrip", () => {
    const { efforts, itemId, snapshots, dir, streamId } = seed();
    const startSnap = createSnapshot(dir, snapshots, streamId, "a", "v1");
    const opened = efforts.openEffort({ workItemId: itemId, startSnapshotId: startSnap });
    expect(opened.ended_at).toBeNull();
    expect(opened.start_snapshot_id).toBe(startSnap);
    expect(efforts.getOpenEffort(itemId)?.id).toBe(opened.id);

    const endSnap = createSnapshot(dir, snapshots, streamId, "b", "v2");
    const closed = efforts.closeEffort({ workItemId: itemId, endSnapshotId: endSnap });
    expect(closed?.ended_at).not.toBeNull();
    expect(closed?.end_snapshot_id).toBe(endSnap);
    expect(efforts.getOpenEffort(itemId)).toBeNull();
  });

  test("openEffort is idempotent — returns existing open effort", () => {
    const { efforts, itemId } = seed();
    const first = efforts.openEffort({ workItemId: itemId, startSnapshotId: null });
    const second = efforts.openEffort({ workItemId: itemId, startSnapshotId: null });
    expect(second.id).toBe(first.id);
  });

  test("reopening creates a new effort after the previous closes", () => {
    const { efforts, itemId } = seed();
    const first = efforts.openEffort({ workItemId: itemId, startSnapshotId: null });
    efforts.closeEffort({ workItemId: itemId, endSnapshotId: null });
    const second = efforts.openEffort({ workItemId: itemId, startSnapshotId: null });
    expect(second.id).not.toBe(first.id);
    const all = efforts.listEffortsForWorkItem(itemId);
    expect(all).toHaveLength(2);
  });

  test("setEffortSummary persists and surfaces on read methods", () => {
    const { efforts, itemId } = seed();
    const opened = efforts.openEffort({ workItemId: itemId, startSnapshotId: null });
    expect(opened.summary).toBeNull();
    efforts.setEffortSummary(opened.id, "Shipped: see commit deadbeef");
    expect(efforts.getById(opened.id)?.summary).toBe("Shipped: see commit deadbeef");
    expect(efforts.getOpenEffort(itemId)?.summary).toBe("Shipped: see commit deadbeef");
    const closed = efforts.closeEffort({ workItemId: itemId, endSnapshotId: null });
    expect(closed?.summary).toBe("Shipped: see commit deadbeef");
    const all = efforts.listEffortsForWorkItem(itemId);
    expect(all[0]!.summary).toBe("Shipped: see commit deadbeef");
  });

  test("direct insert of a second open effort fails (UNIQUE partial index)", async () => {
    const { efforts, itemId, dir } = seed();
    efforts.openEffort({ workItemId: itemId, startSnapshotId: null });
    // Try to bypass the store's idempotent openEffort by talking to the DB
    // directly. Migration v21 should reject this with UNIQUE constraint.
    const { getStateDatabase } = await import("./state-db.js");
    const db = getStateDatabase(dir);
    let threw = false;
    try {
      db.run(
        `INSERT INTO work_item_effort (id, work_item_id, started_at) VALUES (?, ?, ?)`,
        "eff-duplicate",
        itemId,
        new Date().toISOString(),
      );
    } catch (err) {
      threw = true;
      expect(String(err)).toMatch(/UNIQUE/i);
    }
    expect(threw).toBe(true);
  });

  test("listEffortsForPath returns closed efforts ordered by ended_at DESC, with work-item title", () => {
    const { efforts, workItems, threadId, snapshots, dir, streamId } = seed();
    // Three tasks, each touching a common path via work_item_effort_file, each
    // with distinct ended_at timestamps. listEffortsForPath should return them
    // in newest-first order and include the work-item title.
    const items = [0, 1, 2].map((i) =>
      workItems.createItem({
        threadId,
        kind: "task",
        title: `Task ${i}`,
        createdBy: "user",
        actorId: "test",
      }),
    );
    const closedIds: string[] = [];
    for (let i = 0; i < items.length; i++) {
      const startSnap = createSnapshot(dir, snapshots, streamId, `start-${i}`, `v${i}a`);
      const eff = efforts.openEffort({ workItemId: items[i]!.id, startSnapshotId: startSnap });
      efforts.recordEffortFile(eff.id, "shared.txt");
      const endSnap = createSnapshot(dir, snapshots, streamId, `end-${i}`, `v${i}b`);
      efforts.closeEffort({ workItemId: items[i]!.id, endSnapshotId: endSnap });
      closedIds.push(eff.id);
      // Ensure distinct ended_at ordering by sleeping 2ms between closes
      // (ISO timestamps have ms resolution).
      Bun.sleepSync?.(3);
    }
    const rows = efforts.listEffortsForPath("shared.txt");
    expect(rows).toHaveLength(3);
    // Newest-first: closed index 2 should come first.
    expect(rows[0]!.effortId).toBe(closedIds[2]!);
    expect(rows[1]!.effortId).toBe(closedIds[1]!);
    expect(rows[2]!.effortId).toBe(closedIds[0]!);
    expect(rows[0]!.title).toBe("Task 2");
    expect(rows[0]!.workItemId).toBe(items[2]!.id);
    // Rows for a path that no effort touched.
    expect(efforts.listEffortsForPath("nothing.txt")).toHaveLength(0);
  });

  test("listEffortsForPath excludes still-open efforts", () => {
    const { efforts, workItems, threadId } = seed();
    const item = workItems.createItem({
      threadId,
      kind: "task",
      title: "Open",
      createdBy: "user",
      actorId: "test",
    });
    const eff = efforts.openEffort({ workItemId: item.id, startSnapshotId: null });
    efforts.recordEffortFile(eff.id, "open.txt");
    expect(efforts.listEffortsForPath("open.txt")).toHaveLength(0);
  });

  test("listClosedEffortsForThreadAfter returns closed efforts newer than the cutoff, null cutoff = all", async () => {
    const { efforts, workItems, threadId } = seed();
    const a = workItems.createItem({
      threadId, kind: "task", title: "A", createdBy: "user", actorId: "test",
    });
    const b = workItems.createItem({
      threadId, kind: "bug", title: "B", createdBy: "user", actorId: "test",
    });
    efforts.openEffort({ workItemId: a.id, startSnapshotId: null });
    efforts.closeEffort({ workItemId: a.id, endSnapshotId: null });
    await new Promise((r) => setTimeout(r, 5));
    const cutoff = new Date().toISOString();
    await new Promise((r) => setTimeout(r, 5));
    efforts.openEffort({ workItemId: b.id, startSnapshotId: null });
    efforts.closeEffort({ workItemId: b.id, endSnapshotId: null });

    const all = efforts.listClosedEffortsForThreadAfter(threadId, null);
    expect(all.map((r) => r.title).sort()).toEqual(["A", "B"]);

    const afterCutoff = efforts.listClosedEffortsForThreadAfter(threadId, cutoff);
    expect(afterCutoff.map((r) => r.title)).toEqual(["B"]);
    expect(afterCutoff[0]!.kind).toBe("bug");
  });

  test("listClosedEffortsForThreadAfter excludes open efforts and soft-deleted work items", () => {
    const { efforts, workItems, threadId } = seed();
    const open = workItems.createItem({
      threadId, kind: "task", title: "Open", createdBy: "user", actorId: "test",
    });
    efforts.openEffort({ workItemId: open.id, startSnapshotId: null });
    // deletedItem is closed but its work_item is soft-deleted — join must skip.
    const gone = workItems.createItem({
      threadId, kind: "task", title: "Deleted", createdBy: "user", actorId: "test",
    });
    efforts.openEffort({ workItemId: gone.id, startSnapshotId: null });
    efforts.closeEffort({ workItemId: gone.id, endSnapshotId: null });
    workItems.deleteItem(threadId, gone.id, "user", "test");

    const rows = efforts.listClosedEffortsForThreadAfter(threadId, null);
    expect(rows.map((r) => r.title)).toEqual([]);
  });

});
