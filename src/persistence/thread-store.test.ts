import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ThreadStore } from "./thread-store.js";
import type { Stream } from "./stream-store.js";

describe("ThreadStore", () => {
  test("initializes a stream with one active thread", () => {
    const dir = mkdtempSync(join(tmpdir(), "oxplow-threads-"));
    const store = new ThreadStore(dir);

    const state = store.ensureStream(makeStream());

    expect(state.activeThreadId).toBe(state.threads[0]?.id);
    expect(state.selectedThreadId).toBe(state.threads[0]?.id);
    expect(state.threads[0]?.status).toBe("active");
    expect(state.threads[0]?.pane_target).toBe("oxplow-demo:working-s-1");
  });

  test("findById resolves a thread by id alone, even across streams", () => {
    // Regression: MCP tools used to require streamId alongside threadId, which
    // broke when the UI's "current stream" drifted from where the agent was
    // writing. findById is the single-stream-free lookup that lets the
    // server derive streamId from the thread row itself.
    const dir = mkdtempSync(join(tmpdir(), "oxplow-threads-"));
    const store = new ThreadStore(dir);
    const streamA = makeStream();
    const streamB: Stream = { ...makeStream(), id: "s-2", title: "Other" };
    store.ensureStream(streamA);
    const stateB = store.ensureStream(streamB);

    const threadA = store.list(streamA.id).threads[0]!;
    const threadB = stateB.threads[0]!;

    expect(store.findById(threadA.id)?.stream_id).toBe(streamA.id);
    expect(store.findById(threadB.id)?.stream_id).toBe(streamB.id);
    expect(store.findById("b-does-not-exist")).toBeNull();
  });

  test("reorderThreads writes sort_index in the provided order", () => {
    const dir = mkdtempSync(join(tmpdir(), "oxplow-threads-"));
    const stream = makeStream();
    const store = new ThreadStore(dir);
    store.ensureStream(stream);
    store.create(stream, { title: "Second" });
    const state0 = store.create(stream, { title: "Third" });
    const ids = state0.threads.map((b) => b.id);
    expect(ids.length).toBe(3);

    // Reverse the order.
    const reversed = ids.slice().reverse();
    store.reorderThreads(stream.id, reversed);
    const state1 = store.list(stream.id);
    expect(state1.threads.map((b) => b.id)).toEqual(reversed);
    expect(state1.threads.map((b) => b.sort_index)).toEqual([0, 1, 2]);
  });

  test("creates, reorders, promotes, and completes threads", () => {
    const dir = mkdtempSync(join(tmpdir(), "oxplow-threads-"));
    const stream = makeStream();
    const store = new ThreadStore(dir);
    store.ensureStream(stream);

    let state = store.create(stream, { title: "Next thread" });
    const queued = state.threads.find((thread) => thread.status === "queued");
    expect(queued?.title).toBe("Next thread");
    expect(state.selectedThreadId).toBe(queued?.id);
    expect(queued?.sort_index).toBe(1);

    state = store.create(stream, { title: "Follow-up" });
    const followUp = state.threads.find((thread) => thread.title === "Follow-up");
    state = store.reorder(stream.id, followUp!.id, 1);
    expect(state.threads[1]?.id).toBe(followUp?.id);

    // Promote changes status but preserves sort order (no auto-reorder-to-front).
    const orderBeforePromote = state.threads.map((b) => b.id);
    state = store.promote(stream.id, queued!.id);
    expect(state.activeThreadId).toBe(queued?.id);
    expect(state.threads.map((b) => b.id)).toEqual(orderBeforePromote);
    expect(state.threads.find((thread) => thread.id === queued!.id)?.status).toBe("active");

    state = store.complete(stream.id, queued!.id);
    expect(state.threads.find((thread) => thread.id === queued!.id)?.status).toBe("completed");
    expect(state.threads.find((thread) => thread.id === state.activeThreadId)?.title).toBe("Default");
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
      working: "oxplow-demo:working-s-1",
      talking: "oxplow-demo:talking-s-1",
    },
    resume: {
      working_session_id: "resume-working",
      talking_session_id: "",
    },
  };
}
