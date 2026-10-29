import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, renderHook, waitFor } from "@testing-library/react";

// A thread's tabs follow `useThreadSessions`, so right after a switch it
// must never hand back the previous thread's sessions: the tab strip would
// drop the thread's own session tab and lead with the other thread's.

const realApi = await import("./api.js");
const resolvers: Array<() => void> = [];
mock.module("./api.js", () => ({
  ...realApi,
  querySql: (_sql: string, params: unknown[]) =>
    new Promise((resolve) => {
      resolvers.push(() =>
        resolve({
          columns: ["uri", "thread_uri", "kind", "harness", "title", "native_title", "display_name", "opened_at"],
          rows: [[`oxplow:oxplow/agent_session?id=ses_${String(params[0])}`, `oxplow:oxplow/thread?id=${String(params[0])}`, "terminal", "claude", "", "", "", ""]],
          reads: { sources: [], tables: [] },
        }),
      );
    }),
}));
const { useThreadSessions } = await import("./agentSessions.js");

afterEach(() => {
  resolvers.length = 0;
  cleanup();
});

test("switching threads never returns the previous thread's sessions", async () => {
  // Every render's value counts: an effect clears the list only after the
  // render that already drew the new thread with the old thread's sessions.
  const drawn: Array<{ id: string; threadIds: string[] | null }> = [];
  const { result, rerender } = renderHook(
    ({ id }: { id: string }) => {
      const sessions = useThreadSessions(id);
      drawn.push({ id, threadIds: sessions?.map((s) => s.threadId) ?? null });
      return sessions;
    },
    { initialProps: { id: "thr1" } },
  );
  expect(result.current).toBeNull();
  resolvers.shift()!();
  await waitFor(() => expect(result.current?.map((s) => s.threadId)).toEqual(["1"]));
  rerender({ id: "thr2" });
  expect(drawn.filter((d) => d.id === "thr2").every((d) => d.threadIds === null)).toBe(true);
  resolvers.shift()!();
  await waitFor(() => expect(result.current?.map((s) => s.threadId)).toEqual(["2"]));
});
