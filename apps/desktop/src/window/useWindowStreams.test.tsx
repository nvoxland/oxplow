import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";

// The window's streams and each stream's threads: loaded on mount and again
// on a reconnect, a switch picks the stream's first thread when none is
// selected, and the person's thread commands keep each stream's state.

const realApi = { ...(await import("../api.js")) }; // a copy: `mock.module` patches a loaded module in place
const stream = (id: string, created_at: string) => ({ id, title: id, branch: id, created_at }) as never;
const thread = (id: string) => ({ id, title: id }) as never;
const calls: string[] = [];
let threadsOf: Record<string, { selectedThreadId: string | null; threads: unknown[] }> = {};
let reconnect: (() => void) | null = null;
mock.module("../api.js", () => ({
  ...realApi,
  listStreams: async () => [stream("s1", "1"), stream("s2", "2")],
  getCurrentStream: async () => stream("s1", "1"),
  getWorkspaceContext: async () => ({ vcsEnabled: true }),
  getThreadState: async (id: string) => {
    calls.push(`state ${id}`);
    return threadsOf[id];
  },
  switchStream: async (id: string) => {
    calls.push(`switch ${id}`);
    return stream(id, id === "s1" ? "1" : "2");
  },
  selectThread: async (streamId: string, threadId: string) => {
    calls.push(`select ${streamId} ${threadId}`);
    threadsOf[streamId] = { ...threadsOf[streamId], selectedThreadId: threadId };
    return threadsOf[streamId];
  },
  createThread: async (streamId: string, title: string) => {
    calls.push(`create ${streamId} ${title}`);
    const made = thread(`new-${title}`);
    threadsOf[streamId] = { ...threadsOf[streamId]!, threads: [...threadsOf[streamId]!.threads, made] };
    return { state: threadsOf[streamId] };
  },
  onRemoteReconnect: (handler: () => void) => {
    reconnect = handler;
    return () => {
      reconnect = null;
    };
  },
}));
const { useWindowStreams } = await import("./useWindowStreams.js");

function setup() {
  calls.length = 0;
  threadsOf = {
    s1: { selectedThreadId: "t1", threads: [thread("t1")] },
    s2: { selectedThreadId: null, threads: [thread("t2"), thread("t3")] },
  };
  const errors: Array<string | null> = [];
  const hook = renderHook(() =>
    useWindowStreams({ setError: (e) => errors.push(e), setDaemonUnavailable: () => {} }),
  );
  return { hook, errors };
}

afterEach(() => cleanup());

test("the window loads every stream, the current one and its selected thread", async () => {
  const { hook } = setup();
  await waitFor(() => expect(hook.result.current.selectedThread?.id).toBe("t1"));
  expect(hook.result.current.streams.map((s) => s.id)).toEqual(["s1", "s2"]);
  expect(hook.result.current.stream?.id).toBe("s1");
  expect(hook.result.current.workspaceContext).toEqual({ vcsEnabled: true });
  // The other stream's threads are there too, for the navigator.
  await waitFor(() => expect(hook.result.current.threadStates.s2?.threads).toHaveLength(2));
});

test("switching to a stream with no selected thread selects its first", async () => {
  const { hook } = setup();
  await waitFor(() => expect(hook.result.current.stream?.id).toBe("s1"));
  await act(() => hook.result.current.switchStream("s2"));
  expect(hook.result.current.stream?.id).toBe("s2");
  expect(hook.result.current.selectedThread?.id).toBe("t2");
  expect(calls).toContain("select s2 t2");
});

test("selecting a thread in another stream switches to that stream first", async () => {
  const { hook } = setup();
  await waitFor(() => expect(hook.result.current.stream?.id).toBe("s1"));
  await act(() => hook.result.current.selectThread("s2", "t3"));
  expect(hook.result.current.stream?.id).toBe("s2");
  expect(hook.result.current.selectedThread?.id).toBe("t3");
  expect(calls.indexOf("switch s2")).toBeLessThan(calls.indexOf("select s2 t3"));
});

test("a reconnect loads the window's state again", async () => {
  const { hook } = setup();
  await waitFor(() => expect(hook.result.current.selectedThread?.id).toBe("t1"));
  threadsOf.s1 = { selectedThreadId: "t9", threads: [thread("t1"), thread("t9")] };
  act(() => reconnect!());
  await waitFor(() => expect(hook.result.current.selectedThread?.id).toBe("t9"));
});

// A thread is made in the stream it is asked for, whatever stream is shown
// (the switcher lists "+ New thread" under every stream).
test("a new thread is made in the stream named, not the one shown", async () => {
  const { hook } = setup();
  await waitFor(() => expect(hook.result.current.stream?.id).toBe("s1"));
  await act(async () => void (await hook.result.current.createThread("s2", "Plan")));
  expect(calls).toContain("create s2 Plan");
  expect(hook.result.current.threadStates.s2?.threads).toHaveLength(3);
  expect(hook.result.current.stream?.id).toBe("s1");
});
