import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BatchStore } from "./batch-store.js";
import { TurnStore, type TurnChange } from "./turn-store.js";
import { WorkItemStore } from "./work-item-store.js";
import type { Stream } from "./stream-store.js";

function seed() {
  const dir = mkdtempSync(join(tmpdir(), "newde-turns-"));
  const batchStore = new BatchStore(dir);
  const stream: Stream = {
    id: "s-1",
    title: "Demo",
    summary: "",
    branch: "main",
    branch_ref: "refs/heads/main",
    branch_source: "local",
    worktree_path: "/tmp/demo",
    created_at: "2024-01-01T00:00:00.000Z",
    updated_at: "2024-01-01T00:00:00.000Z",
    panes: { working: "newde-demo:working-s-1", talking: "newde-demo:talking-s-1" },
    resume: { working_session_id: "", talking_session_id: "" },
  };
  const state = batchStore.ensureStream(stream);
  const batchId = state.batches[0]!.id;
  const turns = new TurnStore(dir);
  const workItems = new WorkItemStore(dir);
  return { turns, workItems, batchId };
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

    const closed = turns.closeTurn(open.id, { workItemId: null, answer: "Did the thing" });
    expect(closed?.answer).toBe("Did the thing");
    expect(closed?.ended_at).not.toBeNull();
    expect(turns.currentOpenTurn(batchId)).toBeNull();

    expect(changes.map((c) => c.kind)).toEqual(["opened", "closed"]);
  });

  test("closeTurn with workItemId associates the turn with the item", () => {
    const { turns, workItems, batchId } = seed();
    const item = workItems.createItem({
      batchId,
      kind: "task",
      title: "T",
      createdBy: "agent",
      actorId: "mcp",
    });
    const open = turns.openTurn({ batchId, prompt: "P" });
    const closed = turns.closeTurn(open.id, { workItemId: item.id, answer: null });
    expect(closed?.work_item_id).toBe(item.id);
    expect(closed?.answer).toBeNull();
  });

  test("closeTurn on an already-closed turn returns existing without re-emitting", () => {
    const { turns, batchId } = seed();
    const open = turns.openTurn({ batchId, prompt: "P" });
    turns.closeTurn(open.id, { workItemId: null, answer: "A" });

    const events: TurnChange[] = [];
    turns.subscribe((c) => events.push(c));
    const again = turns.closeTurn(open.id, { workItemId: null, answer: "B" });
    expect(again?.answer).toBe("A");
    expect(events).toHaveLength(0);
  });

  test("listForBatch returns newest-first", () => {
    const { turns, batchId } = seed();
    const a = turns.openTurn({ batchId, prompt: "first" });
    turns.closeTurn(a.id, { workItemId: null, answer: "a-done" });
    // Ensure the two rows' started_at differ enough to order deterministically
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
