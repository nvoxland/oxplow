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
const { badgeCount, panelAlerts, panelParams, useExtensionPanelRuns } = await import("./usePanelRuns.js");

const panel = (scope: ExtensionPanel["scope"], over: Partial<ExtensionPanel> = {}): ExtensionPanel =>
  ({ id: "x/p", extension: "x", title: "P", icon: null, scope, body: "x/body", badge: null, ...over }) as ExtensionPanel;

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

// One owner runs every panel's lenses — each body and badge once per
// refresh — and both the panel's header and the Alerts panel read from
// those runs, so a badge never runs twice and the two never disagree.
test("the rail runs each panel's body and badge once; a thread-scoped panel re-runs for its thread", async () => {
  runs.length = 0;
  const panels = [panel("thread"), panel("project", { id: "x/q", body: "x/q-body", badge: "x/q-badge" })];
  const view = renderHook(
    ({ threadId }: { threadId: string | null }) => useExtensionPanelRuns(panels, "str2", threadId),
    { initialProps: { threadId: "thr1" } },
  );
  await waitFor(() => expect(Object.keys(view.result.current).sort()).toEqual(["x/p", "x/q"]));
  expect(runs.map((r) => r.id).sort()).toEqual(["x/body", "x/q-badge", "x/q-body"]);
  expect(runs.find((r) => r.id === "x/body")).toEqual({ id: "x/body", params: { thread_id: 1 }, streamId: "str2" });
  view.rerender({ threadId: "thr2" });
  await waitFor(() => expect(runs.filter((r) => r.id === "x/body").length).toBe(2));
  expect(runs.filter((r) => r.id === "x/body")[1]?.params).toEqual({ thread_id: 2 });
});

test("the alerts are the badges that fire, from the same runs", () => {
  const firing = { lens: { id: "x/q-badge", title: "Q" }, alert: { firing: true, count: 2, value: null, message: "2 rows" } } as unknown as LensRun;
  const quiet = { lens: { id: "x/r-badge", title: "R" }, alert: { firing: false, count: 0, value: null, message: "" } } as unknown as LensRun;
  const panels = [
    panel("project", { id: "x/q", badge: "x/q-badge" }),
    panel("project", { id: "x/r", badge: "x/r-badge" }),
    panel("project", { id: "x/p" }),
  ];
  const alerts = panelAlerts(panels, {
    "x/q": { body: null, badge: firing, count: 2 },
    "x/r": { body: null, badge: quiet, count: null },
    "x/p": { body: null, badge: null, count: null },
  });
  expect(alerts).toEqual([{ id: "x/q-badge", title: "Q", message: "2 rows" }]);
});
