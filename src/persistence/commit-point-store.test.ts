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
    const a = store.create({ batchId, mode: "auto", sortIndex: 10 });
    const b = store.create({ batchId, mode: "approval", sortIndex: 20 });
    expect(a.status).toBe("pending");
    expect(a.mode).toBe("auto");
    const list = store.listForBatch(batchId);
    expect(list.map((c) => c.id)).toEqual([a.id, b.id]);
  });

  test("propose on auto mode jumps straight to approved with the same message", () => {
    const { store, batchId } = seed();
    const cp = store.create({ batchId, mode: "auto", sortIndex: 1 });
    const proposed = store.propose(cp.id, "  feat: add thing  ");
    expect(proposed.status).toBe("approved");
    expect(proposed.approved_message).toBe("feat: add thing");
    expect(proposed.proposed_message).toBe("feat: add thing");
  });

  test("propose on approval mode parks at proposed for user review", () => {
    const { store, batchId } = seed();
    const cp = store.create({ batchId, mode: "approval", sortIndex: 1 });
    const proposed = store.propose(cp.id, "draft message");
    expect(proposed.status).toBe("proposed");
    expect(proposed.approved_message).toBeNull();
  });

  test("approve accepts an edited message and transitions to approved", () => {
    const { store, batchId } = seed();
    const cp = store.create({ batchId, mode: "approval", sortIndex: 1 });
    store.propose(cp.id, "draft");
    const approved = store.approve(cp.id, "final message");
    expect(approved.status).toBe("approved");
    expect(approved.approved_message).toBe("final message");
  });

  test("reject parks the point and stores the note; resetToPending clears proposals", () => {
    const { store, batchId } = seed();
    const cp = store.create({ batchId, mode: "approval", sortIndex: 1 });
    store.propose(cp.id, "bad message");
    const rejected = store.reject(cp.id, "include a ticket number");
    expect(rejected.status).toBe("rejected");
    expect(rejected.rejection_note).toContain("ticket");
    const reset = store.resetToPending(cp.id);
    expect(reset.status).toBe("pending");
    expect(reset.proposed_message).toBeNull();
  });

  test("markDone requires approved status and records the sha", () => {
    const { store, batchId } = seed();
    const cp = store.create({ batchId, mode: "auto", sortIndex: 1 });
    store.propose(cp.id, "msg");
    const done = store.markDone(cp.id, "abc123");
    expect(done.status).toBe("done");
    expect(done.commit_sha).toBe("abc123");
    expect(() => store.markDone(cp.id, "def")).toThrow();
  });

  test("setMode refuses once a proposal exists", () => {
    const { store, batchId } = seed();
    const cp = store.create({ batchId, mode: "approval", sortIndex: 1 });
    store.propose(cp.id, "draft");
    expect(() => store.setMode(cp.id, "auto")).toThrow();
  });

  test("failExecution moves an approved point to rejected (so a failed git commit doesn't loop forever)", () => {
    const { store, batchId } = seed();
    const cp = store.create({ batchId, mode: "auto", sortIndex: 1 });
    store.propose(cp.id, "msg"); // auto → approved
    expect(store.get(cp.id)?.status).toBe("approved");
    const failed = store.failExecution(cp.id, "commit failed: nothing to commit");
    expect(failed.status).toBe("rejected");
    expect(failed.rejection_note).toContain("commit failed");
    // Listed-approved is now empty so the startup recovery loop won't retry.
    expect(store.listApproved().some((p) => p.id === cp.id)).toBe(false);
  });

  test("failExecution refuses to fire from non-approved status", () => {
    const { store, batchId } = seed();
    const cp = store.create({ batchId, mode: "approval", sortIndex: 1 });
    expect(() => store.failExecution(cp.id, "x")).toThrow();
    store.propose(cp.id, "draft");
    // proposed (not approved) → still refused; only the runtime's executor
    // path should reach failExecution, and only after status moved to approved.
    expect(() => store.failExecution(cp.id, "x")).toThrow();
  });

  test("listApproved only returns commit points ready for the runtime to commit", () => {
    const { store, batchId } = seed();
    const a = store.create({ batchId, mode: "auto", sortIndex: 1 });
    const b = store.create({ batchId, mode: "approval", sortIndex: 2 });
    const c = store.create({ batchId, mode: "auto", sortIndex: 3 });
    store.propose(a.id, "msg a");
    store.propose(b.id, "draft b"); // stays proposed
    store.propose(c.id, "msg c");
    const approved = store.listApproved().map((cp) => cp.id).sort();
    expect(approved).toEqual([a.id, c.id].sort());
  });
});
