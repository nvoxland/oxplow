import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BatchStore } from "./batch-store.js";
import type { Stream } from "./stream-store.js";

describe("BatchStore", () => {
  test("initializes a stream with one active batch", () => {
    const dir = mkdtempSync(join(tmpdir(), "newde-batches-"));
    const store = new BatchStore(dir);

    const state = store.ensureStream(makeStream());

    expect(state.activeBatchId).toBe(state.batches[0]?.id);
    expect(state.selectedBatchId).toBe(state.batches[0]?.id);
    expect(state.batches[0]?.status).toBe("active");
    expect(state.batches[0]?.pane_target).toBe("newde-demo:working-s-1");
  });

  test("findById resolves a batch by id alone, even across streams", () => {
    // Regression: MCP tools used to require streamId alongside batchId, which
    // broke when the UI's "current stream" drifted from where the agent was
    // writing. findById is the single-stream-free lookup that lets the
    // server derive streamId from the batch row itself.
    const dir = mkdtempSync(join(tmpdir(), "newde-batches-"));
    const store = new BatchStore(dir);
    const streamA = makeStream();
    const streamB: Stream = { ...makeStream(), id: "s-2", title: "Other" };
    store.ensureStream(streamA);
    const stateB = store.ensureStream(streamB);

    const batchA = store.list(streamA.id).batches[0]!;
    const batchB = stateB.batches[0]!;

    expect(store.findById(batchA.id)?.stream_id).toBe(streamA.id);
    expect(store.findById(batchB.id)?.stream_id).toBe(streamB.id);
    expect(store.findById("b-does-not-exist")).toBeNull();
  });

  test("reorderBatches writes sort_index in the provided order", () => {
    const dir = mkdtempSync(join(tmpdir(), "newde-batches-"));
    const stream = makeStream();
    const store = new BatchStore(dir);
    store.ensureStream(stream);
    store.create(stream, { title: "Second" });
    const state0 = store.create(stream, { title: "Third" });
    const ids = state0.batches.map((b) => b.id);
    expect(ids.length).toBe(3);

    // Reverse the order.
    const reversed = ids.slice().reverse();
    store.reorderBatches(stream.id, reversed);
    const state1 = store.list(stream.id);
    expect(state1.batches.map((b) => b.id)).toEqual(reversed);
    expect(state1.batches.map((b) => b.sort_index)).toEqual([0, 1, 2]);
  });

  test("creates, reorders, promotes, and completes batches", () => {
    const dir = mkdtempSync(join(tmpdir(), "newde-batches-"));
    const stream = makeStream();
    const store = new BatchStore(dir);
    store.ensureStream(stream);

    let state = store.create(stream, { title: "Next batch" });
    const queued = state.batches.find((batch) => batch.status === "queued");
    expect(queued?.title).toBe("Next batch");
    expect(state.selectedBatchId).toBe(queued?.id);
    expect(queued?.sort_index).toBe(1);

    state = store.create(stream, { title: "Follow-up" });
    const followUp = state.batches.find((batch) => batch.title === "Follow-up");
    state = store.reorder(stream.id, followUp!.id, 1);
    expect(state.batches[1]?.id).toBe(followUp?.id);

    // Promote changes status but preserves sort order (no auto-reorder-to-front).
    const orderBeforePromote = state.batches.map((b) => b.id);
    state = store.promote(stream.id, queued!.id);
    expect(state.activeBatchId).toBe(queued?.id);
    expect(state.batches.map((b) => b.id)).toEqual(orderBeforePromote);
    expect(state.batches.find((batch) => batch.id === queued!.id)?.status).toBe("active");

    state = store.complete(stream.id, queued!.id);
    expect(state.batches.find((batch) => batch.id === queued!.id)?.status).toBe("completed");
    expect(state.batches.find((batch) => batch.id === state.activeBatchId)?.title).toBe("Default");
  });
});

function makeStream(): Stream {
  return {
    id: "s-1",
    title: "Demo",
    summary: "",
    branch: "main",
    branch_ref: "refs/heads/main",
    branch_source: "local",
    worktree_path: "/tmp/demo",
    created_at: "2024-01-01T00:00:00.000Z",
    updated_at: "2024-01-01T00:00:00.000Z",
    custom_prompt: null,
    panes: {
      working: "newde-demo:working-s-1",
      talking: "newde-demo:talking-s-1",
    },
    resume: {
      working_session_id: "resume-working",
      talking_session_id: "",
    },
  };
}
