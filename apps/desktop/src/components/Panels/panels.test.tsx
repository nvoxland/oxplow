import { expect, test } from "bun:test";
import { cleanup, renderHook, waitFor } from "@testing-library/react";

import type { Extension, ExtensionPanel, PageRun } from "../../tauri-bridge/generated/bindings.js";
import { decl, fakePage, fakePageRunsBy, fakeRun, loader, node } from "../../pages/testing/fakeRun.js";
import { extPageUri } from "../../uri/uri.js";
import { badgeCount, pageCount, panelAlerts, panelChoices, useExtensionPanelRuns } from "./usePanelRuns.js";
import { panelOpenRef } from "./panelLayout.js";

const panel = (scope: ExtensionPanel["scope"], over: Partial<ExtensionPanel> = {}): ExtensionPanel =>
  ({ id: "x/p", extension: "x", title: "P", icon: null, scope, body: "x/body", badge: null, ...over }) as ExtensionPanel;

/** The page run for page `id` (`<extension>/<slug>`): one table of `rows` rows. */
const runOf = (id: string, rows = 2, over: Partial<PageRun> = {}): PageRun => {
  const [extension, slug] = id.split("/") as [string, string];
  const page = fakePage(node.viz("rows", "rows"), { id, extension, slug, title: id, loaders: [decl("rows")] });
  return {
    ...fakeRun(page, { loaders: { rows: loader(["a"], Array.from({ length: rows }, () => ["x"])) } }),
    ...over,
  } as PageRun;
};

// P6.G1: a panel whose badge page fires shows the alert's count.
test("a firing badge's count; a quiet one has none", () => {
  const run = (firing: boolean, count: number) => ({ alert: { firing, count, value: null, message: "" } }) as unknown as PageRun;
  expect(badgeCount(run(true, 3))).toBe(3);
  expect(badgeCount(run(false, 3))).toBeNull();
  expect(badgeCount(null)).toBeNull();
});

// One owner opens every panel's pages — each body and badge once — and
// both the panel's header and the Alerts panel read from those runs, so a
// badge never runs twice and the two never disagree.
test("the rail opens each panel's body and badge once; a thread-scoped panel opens again for its thread", async () => {
  const { runs, opened } = fakePageRunsBy((key) => runOf(key.page));
  const panels = [panel("thread"), panel("project", { id: "x/q", body: "x/q-body", badge: "x/q-badge" })];
  const view = renderHook(
    ({ threadId }: { threadId: string | null }) => useExtensionPanelRuns(panels, "str2", threadId, undefined, runs),
    { initialProps: { threadId: "thr1" } },
  );
  await waitFor(() => expect(view.result.current["x/q"]?.badge?.page.id).toBe("x/q-badge"));
  await waitFor(() => expect(view.result.current["x/p"]?.body?.run.page.id).toBe("x/body"));
  expect(opened.map((o) => o.key.page).sort()).toEqual(["x/body", "x/q-badge", "x/q-body"]);
  // The daemon runs the part (`open_panel_part`): it binds the scope's id
  // from the view, so the window writes no param of its own.
  expect(opened.find((o) => o.key.page === "x/body")?.key).toEqual({
    page: "x/body",
    panel: { panel: "x/p", part: "body", choices: {} },
    params: {},
    streamId: "str2",
    threadId: "thr1",
  });
  view.rerender({ threadId: "thr2" });
  await waitFor(() => expect(opened.filter((o) => o.key.page === "x/body").length).toBe(2));
  expect(opened.filter((o) => o.key.page === "x/body")[1]?.key.threadId).toBe("thr2");
  view.unmount();
  cleanup();
});

test("the alerts are the badges that fire, from the same runs", () => {
  const firing = runOf("x/q-badge", 0, { alert: { firing: true, count: 2, value: null, message: "2 rows" } } as never);
  const quiet = runOf("x/r-badge", 0, { alert: { firing: false, count: 0, value: null, message: "" } } as never);
  const panels = [
    panel("project", { id: "x/q", badge: "x/q-badge" }),
    panel("project", { id: "x/r", badge: "x/r-badge" }),
    panel("project", { id: "x/p" }),
  ];
  const alerts = panelAlerts(
    panels,
    {
      "x/q": { body: null, badge: firing, collapsed: null, count: 2 },
      "x/r": { body: null, badge: quiet, collapsed: null, count: null },
      "x/p": { body: null, badge: null, collapsed: null, count: null },
    },
    [{ name: "x", namespace: "x", enabled: true }] as unknown as Extension[],
  );
  expect(alerts).toEqual([{ id: "x/q-badge", title: "x/q-badge", message: "2 rows", open: "oxplow:oxplow/page?page=x/q-badge" }]);
});

// A panel's header opens the page it names (Comments opens the inbox),
// else its body page.
test("a panel's header opens the page it names, else its body page", () => {
  expect(panelOpenRef(panel("stream", { open: "oxplow:oxplow/page?page=comments" })).id).toBe("oxplow:oxplow/page?page=comments");
  expect(panelOpenRef(panel("stream"))).toEqual({ id: extPageUri("x", "body") });
});

// A panel's `count` page gives its header count — its first loader's row
// count, or its badge — without raising an alert; it wins over the badge's
// count.
test("a count page's number: its badge, else its first loader's rows", () => {
  const run = runOf("x/n", 3);
  expect(pageCount(run)).toBe(3);
  expect(pageCount({ ...run, badge: "5" })).toBe(5);
  expect(pageCount({ ...run, badge: 5 })).toBe(5);
  expect(pageCount(null)).toBeNull();
});

// The one owner opens a panel's collapsed and count pages with its body and
// badge — one run per part, the header count the count page's.
test("the rail opens a panel's collapsed and count pages with its body", async () => {
  const { runs, opened } = fakePageRunsBy((key) => runOf(key.page, key.page === "x/n" ? 3 : 1));
  const panels = [panel("stream", { body: "x/body", collapsed: "x/line", count: "x/n" })];
  const view = renderHook(() => useExtensionPanelRuns(panels, "str2", null, undefined, runs));
  await waitFor(() => expect(view.result.current["x/p"]?.collapsed?.run.page.id).toBe("x/line"));
  await waitFor(() => expect(view.result.current["x/p"]?.count).toBe(3));
  expect(opened.map((o) => o.key.page).sort()).toEqual(["x/body", "x/line", "x/n"]);
  view.unmount();
  cleanup();
});

// A body page's choice params are the panel's header toggle; the
// viewer's pick goes with the body part only (the others don't declare it).
test("a choice param sends the viewer's pick with the body part only", async () => {
  const { runs, opened } = fakePageRunsBy((key) => runOf(key.page));
  const panels = [panel("thread", { body: "x/body", collapsed: "x/line" })];
  const view = renderHook(() => useExtensionPanelRuns(panels, "str2", "thr7", { "x/p": { mode: "top" } }, runs));
  await waitFor(() => expect(view.result.current["x/p"]?.collapsed?.run.page.id).toBe("x/line"));
  const of = (page: string) => opened.find((o) => o.key.page === page)?.key.panel;
  expect(of("x/body")).toEqual({ panel: "x/p", part: "body", choices: { mode: "top" } });
  expect(of("x/line")).toEqual({ panel: "x/p", part: "collapsed", choices: {} });
  view.unmount();
  cleanup();
});

// The value in force is the daemon's (`pages/panels.rs` `choices`: the pick
// while the body offers it, else its default), read off the body's run.
test("the toggle shows each choice param at the value the body ran with", () => {
  const options = [{ value: "recent", label: "Recent" }, { value: "top", label: "Most visited" }];
  const page = fakePage(node.viz("rows", "rows"), {
    params: [
      { name: "thread_id", label: null, default: null, options: [] },
      { name: "mode", label: "Show", default: "recent", options },
    ] as never,
  });
  const ran = (mode: string) => ({ ...fakeRun(page), params: { thread_id: 7, mode } }) as PageRun;
  expect(panelChoices(ran("recent"))).toEqual([{ name: "mode", label: "Show", value: "recent", options }]);
  expect(panelChoices(ran("top"))[0]!.value).toBe("top");
  expect(panelChoices(null)).toEqual([]);
});

// One open page run per part, the body with the picks; the badge page's alert
// or badge is the count and the Alerts row, and a count page wins.
test("a panel opens a run per part, the body with the picks", async () => {
  const { runs, opened } = fakePageRunsBy((key) => {
    if (key.page === "y/badge") return runOf(key.page, 2, { alert: { firing: true, count: 4, value: null, message: "4 waiting" } } as never);
    if (key.page === "y/count") return runOf(key.page, 2, { badge: 9 } as never);
    return runOf(key.page);
  });
  const panels = [
    panel("thread", { id: "y/p", body: "y/body", badge: "y/badge", collapsed: "y/line" }),
    panel("project", { id: "y/q", body: "y/body2", badge: "y/badge", count: "y/count" }),
  ];
  const view = renderHook(() => useExtensionPanelRuns(panels, "str2", "thr7", { "y/p": { mode: "top" } }, runs));
  await waitFor(() => expect(view.result.current["y/p"]?.collapsed?.run.page.id).toBe("y/line"));
  const of = (page: string) => opened.find((o) => o.key.page === page)?.key.panel;
  expect(of("y/body")).toEqual({ panel: "y/p", part: "body", choices: { mode: "top" } });
  expect(of("y/line")).toEqual({ panel: "y/p", part: "collapsed", choices: {} });
  expect(of("y/body2")).toEqual({ panel: "y/q", part: "body", choices: {} });
  await waitFor(() => expect(view.result.current["y/p"]?.count).toBe(4));
  await waitFor(() => expect(view.result.current["y/q"]?.count).toBe(9));
  expect(view.result.current["y/p"]?.body?.runId).toBe("run-1");

  const exts = [{ name: "y", namespace: "y", enabled: true }] as unknown as Extension[];
  const alerts = panelAlerts(panels, view.result.current, exts);
  expect(alerts[0]).toEqual({ id: "y/badge", title: "y/badge", message: "4 waiting", open: "oxplow:oxplow/page?page=y/badge" });
  view.unmount();
  cleanup();
});
