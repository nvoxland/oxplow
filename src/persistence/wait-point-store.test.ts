import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ThreadStore } from "./thread-store.js";
import { WaitPointStore } from "./wait-point-store.js";
import type { Stream } from "./stream-store.js";

function seed() {
  const dir = mkdtempSync(join(tmpdir(), "newde-wait-point-"));
  const threadStore = new ThreadStore(dir);
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
  const state = threadStore.ensureStream(stream);
  const threadId = state.threads[0]!.id;
  return { store: new WaitPointStore(dir), threadId };
}

describe("WaitPointStore", () => {
  test("create starts pending and lists in order", () => {
    const { store, threadId } = seed();
    const a = store.create({ threadId, sortIndex: 5, note: "before merge" });
    const b = store.create({ threadId, sortIndex: 10 });
    expect(a.status).toBe("pending");
    expect(a.note).toBe("before merge");
    expect(store.listForThread(threadId).map((p) => p.id)).toEqual([a.id, b.id]);
  });

  test("trigger flips pending → triggered but is idempotent", () => {
    const { store, threadId } = seed();
    const wp = store.create({ threadId, sortIndex: 1 });
    const triggered = store.trigger(wp.id);
    expect(triggered.status).toBe("triggered");
    // calling again on a non-pending point returns it unchanged without error
    expect(store.trigger(wp.id).status).toBe("triggered");
  });

  test("setNote updates the note on a live point", () => {
    const { store, threadId } = seed();
    const wp = store.create({ threadId, sortIndex: 1, note: "old" });
    const updated = store.setNote(wp.id, "new");
    expect(updated.note).toBe("new");
  });
});
