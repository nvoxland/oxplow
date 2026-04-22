import { describe, expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BatchStore } from "./batch-store.js";
import { SnapshotStore } from "./snapshot-store.js";
import { StreamStore } from "./stream-store.js";
import { TurnStore } from "./turn-store.js";
import { WorkItemStore } from "./work-item-store.js";
import { WorkItemEffortStore } from "./work-item-effort-store.js";

function seed() {
  const dir = mkdtempSync(join(tmpdir(), "newde-effort-"));
  const streamStore = new StreamStore(dir);
  const stream = streamStore.create({
    title: "Demo",
    branch: "main",
    worktreePath: dir,
    projectBase: "demo",
  });
  const batchStore = new BatchStore(dir);
  const state = batchStore.ensureStream(stream);
  const batchId = state.batches[0]!.id;
  const workItems = new WorkItemStore(dir);
  const item = workItems.createItem({
    batchId,
    kind: "task",
    title: "T",
    createdBy: "user",
    actorId: "test",
  });
  const turns = new TurnStore(dir);
  const snapshots = new SnapshotStore(dir);
  const efforts = new WorkItemEffortStore(dir);
  return { dir, workItems, turns, snapshots, efforts, batchId, itemId: item.id, streamId: stream.id };
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

  test("linkEffortTurn + listTurnsForEffort + listEffortsForTurn", () => {
    const { efforts, turns, itemId, batchId } = seed();
    const effort = efforts.openEffort({ workItemId: itemId, startSnapshotId: null });
    const turnA = turns.openTurn({ batchId, prompt: "A" });
    const turnB = turns.openTurn({ batchId, prompt: "B" });
    efforts.linkEffortTurn(effort.id, turnA.id);
    efforts.linkEffortTurn(effort.id, turnB.id);
    efforts.linkEffortTurn(effort.id, turnA.id); // duplicate — ignored

    expect(efforts.listTurnsForEffort(effort.id).sort()).toEqual([turnA.id, turnB.id].sort());
    expect(efforts.listEffortsForTurn(turnA.id).map((e) => e.id)).toEqual([effort.id]);
  });
});
