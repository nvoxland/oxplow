import { expect, test } from "bun:test";
import { cleanup, renderHook, waitFor } from "@testing-library/react";

import type { Extension, ExtensionPanel, PageRun, PartView } from "../../tauri-bridge/generated/bindings.js";
import { decl, fakePage, fakePageRunsBy, fakeRun, loader, node } from "../../pages/testing/fakeRun.js";
import { extPageUri } from "../../uri/uri.js";
import { panelAlerts, useExtensionPanelRuns } from "./usePanelRuns.js";
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
      "x/q": { body: null, badge: firing, collapsed: null, count: 2, choices: [] },
      "x/r": { body: null, badge: quiet, collapsed: null, count: null, choices: [] },
      "x/p": { body: null, badge: null, collapsed: null, count: null, choices: [] },
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

/** What the daemon says a part gives its panel (`pages/panels.rs`
 *  `part_view`): the window takes it as it is. */
const counts = (count: number | null): PartView => ({ counts: true, count, choices: [] });
const quiet: PartView = { counts: false, count: null, choices: [] };

// The one owner opens a panel's collapsed and count pages with its body and
// badge — one run per part, the header count the one the daemon says.
test("the rail opens a panel's collapsed and count pages with its body", async () => {
  const { runs, opened } = fakePageRunsBy(
    (key) => runOf(key.page!, key.page === "x/n" ? 3 : 1),
    (key) => (key.panel?.part === "count" ? counts(3) : quiet),
  );
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

// One open page run per part, the body with the picks; the header's count is
// the part the daemon says counts, the toggles the body's, and the badge
// page's alert the Alerts row.
test("a panel opens a run per part, the body with the picks", async () => {
  const mode = { name: "mode", label: "Show", options: [{ value: "top", label: "Top" }], value: "top" };
  const { runs, opened } = fakePageRunsBy(
    (key) => {
      if (key.page === "y/badge") return runOf(key.page, 2, { alert: { firing: true, count: 4, value: null, message: "4 waiting" } } as never);
      return runOf(key.page!);
    },
    (key) => {
      if (key.panel?.panel === "y/p" && key.panel.part === "badge") return counts(4);
      if (key.panel?.panel === "y/q" && key.panel.part === "count") return counts(9);
      if (key.panel?.panel === "y/p" && key.panel.part === "body") return { ...quiet, choices: [mode] };
      return quiet;
    },
  );
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
  expect(view.result.current["y/p"]?.choices).toEqual([mode]);
  expect(view.result.current["y/q"]?.choices).toEqual([]);

  const exts = [{ name: "y", namespace: "y", enabled: true }] as unknown as Extension[];
  const alerts = panelAlerts(panels, view.result.current, exts);
  expect(alerts[0]).toEqual({ id: "y/badge", title: "y/badge", message: "4 waiting", open: "oxplow:oxplow/page?page=y/badge" });
  view.unmount();
  cleanup();
});
