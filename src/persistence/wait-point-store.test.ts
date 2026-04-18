import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BatchStore } from "./batch-store.js";
import { WaitPointStore } from "./wait-point-store.js";
import type { Stream } from "./stream-store.js";

function seed() {
  const dir = mkdtempSync(join(tmpdir(), "newde-wait-point-"));
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
  return { store: new WaitPointStore(dir), batchId };
}

describe("WaitPointStore", () => {
  test("create starts pending and lists in order", () => {
    const { store, batchId } = seed();
    const a = store.create({ batchId, sortIndex: 5, note: "before merge" });
    const b = store.create({ batchId, sortIndex: 10 });
    expect(a.status).toBe("pending");
    expect(a.note).toBe("before merge");
    expect(store.listForBatch(batchId).map((p) => p.id)).toEqual([a.id, b.id]);
  });

  test("trigger flips pending → triggered but is idempotent", () => {
    const { store, batchId } = seed();
    const wp = store.create({ batchId, sortIndex: 1 });
    const triggered = store.trigger(wp.id);
    expect(triggered.status).toBe("triggered");
    // calling again on a non-pending point returns it unchanged without error
    expect(store.trigger(wp.id).status).toBe("triggered");
  });

  test("setNote updates the note on a live point", () => {
    const { store, batchId } = seed();
    const wp = store.create({ batchId, sortIndex: 1, note: "old" });
    const updated = store.setNote(wp.id, "new");
    expect(updated.note).toBe("new");
  });
});
