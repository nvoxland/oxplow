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

  test("creates, reorders, and promotes threads", () => {
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

    const orderBeforePromote = state.threads.map((b) => b.id);
    state = store.promote(stream.id, queued!.id);
    expect(state.activeThreadId).toBe(queued?.id);
    expect(state.threads.map((b) => b.id)).toEqual(orderBeforePromote);
    expect(state.threads.find((thread) => thread.id === queued!.id)?.status).toBe("active");
  });

  test("close hides a queued thread; reopen brings it back", () => {
    const dir = mkdtempSync(join(tmpdir(), "oxplow-threads-"));
    const stream = makeStream();
    const store = new ThreadStore(dir);
    store.ensureStream(stream);
    const created = store.create(stream, { title: "Side quest" });
    const target = created.threads.find((t) => t.title === "Side quest")!;

    const afterClose = store.close(stream.id, target.id);
    expect(afterClose.threads.some((t) => t.id === target.id)).toBe(false);
    const closed = store.listClosed(stream.id);
    expect(closed.length).toBe(1);
    expect(closed[0]?.id).toBe(target.id);
    expect(closed[0]?.closed_at).toBeTruthy();

    const afterReopen = store.reopen(stream.id, target.id);
    expect(afterReopen.threads.some((t) => t.id === target.id)).toBe(true);
    expect(store.listClosed(stream.id).length).toBe(0);
  });

  test("close refuses the writer thread", () => {
    const dir = mkdtempSync(join(tmpdir(), "oxplow-threads-"));
    const stream = makeStream();
    const store = new ThreadStore(dir);
    const initial = store.ensureStream(stream);
    const writer = initial.threads.find((t) => t.status === "active")!;
    expect(() => store.close(stream.id, writer.id)).toThrow(/writer thread/);
  });

  test("close refuses a thread with open work items", () => {
    const dir = mkdtempSync(join(tmpdir(), "oxplow-threads-"));
    const stream = makeStream();
    const store = new ThreadStore(dir);
    store.ensureStream(stream);
    const created = store.create(stream, { title: "Side" });
    const target = created.threads.find((t) => t.title === "Side")!;

    // Insert a ready work item directly via the shared db.
    const { getStateDatabase } = require("./state-db.js");
    const db = getStateDatabase(dir);
    db.run(
      `INSERT INTO work_items (id, thread_id, kind, title, status, priority, sort_index, created_by, author, created_at, updated_at)
       VALUES ('wi-x', ?, 'task', 'Open task', 'ready', 'medium', 0, 'user', 'user', ?, ?)`,
      target.id,
      new Date().toISOString(),
      new Date().toISOString(),
    );

    expect(() => store.close(stream.id, target.id)).toThrow(/open work items/);
  });
});

function makeStream(): Stream {
  return {
    id: "s-1",
    kind: "worktree",
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
