import { describe, expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BatchStore } from "./batch-store.js";
import { SnapshotStore } from "./snapshot-store.js";
import { StreamStore } from "./stream-store.js";
import { TurnStore, type TurnChange } from "./turn-store.js";

function seed() {
  const dir = mkdtempSync(join(tmpdir(), "newde-turns-"));
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
  const turns = new TurnStore(dir);
  const snapshots = new SnapshotStore(dir);
  return { turns, batchId, snapshots, worktreePath: dir, streamId: stream.id };
}

describe("TurnStore", () => {
  test("openTurn + closeTurn roundtrip and emit change events", () => {
    const { turns, batchId } = seed();
    const changes: TurnChange[] = [];
    turns.subscribe((change) => changes.push(change));

    const open = turns.openTurn({ batchId, prompt: "Do the thing", sessionId: "sess-1" });
    expect(open.prompt).toBe("Do the thing");
    expect(open.ended_at).toBeNull();
    expect(turns.currentOpenTurn(batchId)?.id).toBe(open.id);

    const closed = turns.closeTurn(open.id, { answer: "Did the thing" });
    expect(closed?.answer).toBe("Did the thing");
    expect(closed?.ended_at).not.toBeNull();
    expect(turns.currentOpenTurn(batchId)).toBeNull();

    expect(changes.map((c) => c.kind)).toEqual(["opened", "closed"]);
  });

  test("setStartSnapshot and setEndSnapshot populate the turn", () => {
    const { turns, batchId, snapshots, worktreePath, streamId } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "v1");
    const startSnap = snapshots.flushSnapshot({
      source: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    writeFileSync(join(worktreePath, "a.txt"), "v2");
    const endSnap = snapshots.flushSnapshot({
      source: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    const open = turns.openTurn({ batchId, prompt: "P" });
    turns.setStartSnapshot(open.id, startSnap.id);
    turns.setEndSnapshot(open.id, endSnap.id);
    const read = turns.getById(open.id);
    expect(read?.start_snapshot_id).toBe(startSnap.id);
    expect(read?.end_snapshot_id).toBe(endSnap.id);
  });

  test("closeTurn on an already-closed turn returns existing without re-emitting", () => {
    const { turns, batchId } = seed();
    const open = turns.openTurn({ batchId, prompt: "P" });
    turns.closeTurn(open.id, { answer: "A" });

    const events: TurnChange[] = [];
    turns.subscribe((c) => events.push(c));
    const again = turns.closeTurn(open.id, { answer: "B" });
    expect(again?.answer).toBe("A");
    expect(events).toHaveLength(0);
  });

  test("listForBatch returns newest-first", () => {
    const { turns, batchId } = seed();
    const a = turns.openTurn({ batchId, prompt: "first" });
    turns.closeTurn(a.id, { answer: "a-done" });
    const b = turns.openTurn({ batchId, prompt: "second" });
    const list = turns.listForBatch(batchId);
    expect(list).toHaveLength(2);
    expect(list[0]!.id).toBe(b.id);
    expect(list[1]!.id).toBe(a.id);
  });

  test("prompt longer than cap is truncated with ellipsis", () => {
    const { turns, batchId } = seed();
    const huge = "x".repeat(25_000);
    const open = turns.openTurn({ batchId, prompt: huge });
    expect(open.prompt.length).toBe(20_000);
    expect(open.prompt.endsWith("…")).toBe(true);
  });
});
