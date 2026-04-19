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

  test("recordSummary updates summary fields and emits an event", () => {
    const dir = mkdtempSync(join(tmpdir(), "newde-batches-"));
    const stream = makeStream();
    const store = new BatchStore(dir);
    const initial = store.ensureStream(stream);
    const batchId = initial.batches[0]!.id;

    expect(initial.batches[0]?.summary).toBe("");
    expect(initial.batches[0]?.summary_updated_at).toBeNull();

    const changes: string[] = [];
    store.subscribe((change) => {
      if (change.batchId === batchId) changes.push(change.kind);
    });

    const updated = store.recordSummary(stream.id, batchId, "  Read the README. Planning next steps.  ");
    expect(updated.summary).toBe("Read the README. Planning next steps.");
    expect(updated.summary_updated_at).not.toBeNull();
    expect(changes).toContain("summary-updated");

    expect(() => store.recordSummary(stream.id, batchId, "   ")).toThrow(/required/);
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

    state = store.promote(stream.id, queued!.id);
    expect(state.activeBatchId).toBe(queued?.id);
    expect(state.batches[0]?.id).toBe(queued?.id);

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
