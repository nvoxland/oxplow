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
const { badgeCount, panelAlerts, panelChoices, panelCount, panelParams, useExtensionPanelRuns } = await import("./usePanelRuns.js");

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
    "x/q": { body: null, badge: firing, collapsed: null, count: 2 },
    "x/r": { body: null, badge: quiet, collapsed: null, count: null },
    "x/p": { body: null, badge: null, collapsed: null, count: null },
  });
  expect(alerts).toEqual([{ id: "x/q-badge", title: "Q", message: "2 rows" }]);
});

// tsk1086: a panel's header opens the page it names (Comments opens the
// inbox), else its body lens.
test("a panel's header opens the page it names, else its body lens", async () => {
  const { panelOpenRef } = await import("./panelLayout.js");
  const { lensRef } = await import("../../tabs/pageRefs.js");
  expect(panelOpenRef(panel("stream", { open: "page:comments" })).id).toBe("page:comments");
  expect(panelOpenRef(panel("stream"))).toEqual(lensRef("x/body"));
});

// tsk1089: a panel's `count` lens gives its header count — its row count,
// or a `number` lens's value — without raising an alert; it wins over the
// badge's count, and the badge still feeds Alerts.
test("a count lens's rows (or number) are the header count, over the badge's", () => {
  const rows = (n: number) =>
    ({ lens: { viz: "list" }, result: { columns: ["a"], rows: Array.from({ length: n }, () => ["x"]) } }) as unknown as LensRun;
  const number = (v: SqlCell) => ({ lens: { viz: "number" }, result: { columns: ["n"], rows: [[v]] } }) as unknown as LensRun;
  const firing = { alert: { firing: true, count: 9, value: null, message: "" } } as unknown as LensRun;
  expect(panelCount(rows(3), null)).toBe(3);
  expect(panelCount(rows(0), firing)).toBe(0);
  expect(panelCount(number(7), firing)).toBe(7);
  expect(panelCount(number("x"), null)).toBeNull();
  expect(panelCount(null, firing)).toBe(9);
  expect(panelCount(null, null)).toBeNull();
});

// The one owner runs a panel's collapsed and count lenses with its body and
// badge — each distinct lens once, however many roles it plays.
test("the rail runs a panel's collapsed and count lenses, each lens once", async () => {
  runs.length = 0;
  const panels = [panel("stream", { body: "x/body", collapsed: "x/line", count: "x/body" })];
  const view = renderHook(() => useExtensionPanelRuns(panels, "str2", null));
  await waitFor(() => expect(view.result.current["x/p"]?.collapsed?.lens.id).toBe("x/line"));
  expect(runs.map((r) => r.id).sort()).toEqual(["x/body", "x/line"]);
  expect(view.result.current["x/p"]?.count).toBe(0);
});

// tsk1100: a body lens's choice params are the panel's header toggle; the
// viewer's pick binds the body lens only (the others don't declare it).
test("a choice param binds the viewer's pick to the body lens only", async () => {
  runs.length = 0;
  const panels = [panel("thread", { body: "x/body", collapsed: "x/line" })];
  const view = renderHook(() => useExtensionPanelRuns(panels, "str2", "thr7", { "x/p": { mode: "top" } }));
  await waitFor(() => expect(view.result.current["x/p"]?.collapsed?.lens.id).toBe("x/line"));
  const of = (id: string) => runs.find((r) => r.id === id)?.params;
  expect(of("x/body")).toEqual({ thread_id: 7, mode: "top" });
  expect(of("x/line")).toEqual({ thread_id: 7 });
});

test("the toggle shows each choice param at the pick, else its default", () => {
  const body = {
    lens: {
      params: [
        { name: "thread_id", label: null, default: null, options: [] },
        { name: "mode", label: "Show", default: "recent", options: [{ value: "recent", label: "Recent" }, { value: "top", label: "Most visited" }] },
      ],
    },
  } as unknown as LensRun;
  expect(panelChoices(body, {})).toEqual([
    { name: "mode", label: "Show", value: "recent", options: [{ value: "recent", label: "Recent" }, { value: "top", label: "Most visited" }] },
  ]);
  expect(panelChoices(body, { mode: "top" })[0]!.value).toBe("top");
  // A pick the lens no longer offers falls back to the default.
  expect(panelChoices(body, { mode: "gone" })[0]!.value).toBe("recent");
  expect(panelChoices(null, {})).toEqual([]);
});
