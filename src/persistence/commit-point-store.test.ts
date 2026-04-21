import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BatchStore } from "./batch-store.js";
import { CommitPointStore } from "./commit-point-store.js";
import type { Stream } from "./stream-store.js";

function seed() {
  const dir = mkdtempSync(join(tmpdir(), "newde-commit-point-"));
  const batchStore = new BatchStore(dir);
  const stream: Stream = {
    id: "s-1",
    title: "Demo",
    summary: "",
    branch: "main",
    branch_ref: "refs/heads/main",
    branch_source: "local",
    worktree_path: "/tmp/demo",
    custom_prompt: null,
    created_at: "2024-01-01T00:00:00.000Z",
    updated_at: "2024-01-01T00:00:00.000Z",
    panes: { working: "newde-demo:working-s-1", talking: "newde-demo:talking-s-1" },
    resume: { working_session_id: "", talking_session_id: "" },
  };
  const state = batchStore.ensureStream(stream);
  const batchId = state.batches[0]!.id;
  const store = new CommitPointStore(dir);
  return { store, batchId };
}

describe("CommitPointStore", () => {
  test("create starts pending and lists in sort order", () => {
    const { store, batchId } = seed();
    const a = store.create({ batchId, sortIndex: 10 });
    const b = store.create({ batchId, sortIndex: 20 });
    expect(a.status).toBe("pending");
    const list = store.listForBatch(batchId);
    expect(list.map((c) => c.id)).toEqual([a.id, b.id]);
  });

  test("update changes the mode and leaves status untouched", () => {
    const { store, batchId } = seed();
    const cp = store.create({ batchId, sortIndex: 1 });
    expect(cp.mode).toBe("approve");
    const updated = store.update(cp.id, { mode: "auto" });
    expect(updated.mode).toBe("auto");
    expect(updated.status).toBe("pending");
  });

  test("markCommitted records the sha and flips to done", () => {
    const { store, batchId } = seed();
    const cp = store.create({ batchId, sortIndex: 1 });
    const done = store.markCommitted(cp.id, "final message", "abc123");
    expect(done.status).toBe("done");
    expect(done.commit_sha).toBe("abc123");
    expect(() => store.markCommitted(cp.id, "x", "def")).toThrow();
  });

  test("delete refuses on completed commit points", () => {
    const { store, batchId } = seed();
    const cp = store.create({ batchId, sortIndex: 1 });
    store.markCommitted(cp.id, "msg", "abc");
    expect(() => store.delete(cp.id)).toThrow();
  });
});
