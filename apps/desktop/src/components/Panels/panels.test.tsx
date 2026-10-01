import { expect, mock, test } from "bun:test";
import { renderHook, waitFor } from "@testing-library/react";

import type { ExtensionPanel, LensRun, SqlCell } from "../../tauri-bridge/generated/bindings.js";

// Bun's `mock.module` is process-wide: everything not faked here delegates
// to the real module so other test files keep working.
const realApi = await import("../../api.js");
const runs: { id: string; params: Record<string, SqlCell>; streamId: string | null }[] = [];
mock.module("../../api.js", () => ({
  ...realApi,
  runLens: async (id: string, params: Record<string, SqlCell>, streamId: string | null): Promise<LensRun> => {
    runs.push({ id, params, streamId });
    return { lens: { id } as LensRun["lens"], params, result: { columns: [], rows: [], truncated: false }, alert: null };
  },
}));
const { badgeCount, panelParams, usePanelRuns } = await import("./usePanelRuns.js");

const panel = (scope: ExtensionPanel["scope"]): ExtensionPanel =>
  ({ id: "x/p", extension: "x", title: "P", icon: null, scope, body: "x/body", badge: null }) as ExtensionPanel;

// P6.G1: a panel whose badge lens fires shows the alert's count.
test("a firing badge's count; a quiet one has none", () => {
  const run = (firing: boolean, count: number) =>
    ({ alert: { firing, count, value: null, message: "" } }) as unknown as LensRun;
  expect(badgeCount(run(true, 3))).toBe(3);
  expect(badgeCount(run(false, 3))).toBeNull();
  expect(badgeCount(null)).toBeNull();
});

// A panel's scope says what its lenses are bound to, and the nav states
// that binding rather than leaving the backend to infer the thread from
// the selection — so a thread-scoped panel follows the thread the person
// is looking at.
test("a panel's scope binds the stream or thread it's shown for", () => {
  expect(panelParams("project", "str2", "thr7")).toEqual({});
  expect(panelParams("stream", "str2", "thr7")).toEqual({ stream_id: 2 });
  expect(panelParams("thread", "str2", "thr7")).toEqual({ thread_id: 7 });
  expect(panelParams("thread", "str2", null)).toEqual({});
});

test("a thread-scoped panel re-runs for the thread it's shown for", async () => {
  runs.length = 0;
  const view = renderHook(({ threadId }: { threadId: string | null }) => usePanelRuns(panel("thread"), "str2", threadId), {
    initialProps: { threadId: "thr1" },
  });
  await waitFor(() => expect(runs.length).toBe(1));
  expect(runs[0]).toEqual({ id: "x/body", params: { thread_id: 1 }, streamId: "str2" });
  view.rerender({ threadId: "thr2" });
  await waitFor(() => expect(runs.length).toBe(2));
  expect(runs[1]?.params).toEqual({ thread_id: 2 });
});
