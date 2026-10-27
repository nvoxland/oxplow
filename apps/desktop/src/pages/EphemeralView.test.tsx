import { afterEach, expect, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";
import { createPageRuns, type PageKey } from "../pageRuns.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import { EphemeralView } from "./EphemeralView.js";
import { tablePage } from "./testing/fakeRun.js";

afterEach(cleanup);

test("a shown view keeps its source in the entry, and a restored tab supplies it when opening", async () => {
  const source = "# original\ntitle: Inline\nview: { markdown: Hello }\n";
  const opened: PageKey[] = [];
  let historyState: Record<string, unknown> = {};
  const runs = createPageRuns({
    open: async (key) => { opened.push(key); return { runId: `run-${opened.length}`, run: tablePage(), specYaml: source }; },
    close: async () => {}, fetch: async () => ({}), setParams: async () => tablePage(), rerun: async () => tablePage(),
    onChange: () => () => {}, onReconnect: () => () => {},
  });
  const draw = () => render(<PageNavigationContext.Provider value={{
    ref: { id: "oxplow:oxplow/view?id=v1" }, historyState,
    setHistoryState: (patch) => { historyState = { ...historyState, ...patch }; },
    navigate: () => {}, goBack: () => {}, goForward: () => {}, canGoBack: false, canGoForward: false,
  } as PageNavigation}>
    <EphemeralView view="v1" stream={null} threadId="thr1" runs={runs} />
  </PageNavigationContext.Provider>);
  draw();
  await waitFor(() => expect(historyState["view.specYaml"]).toBe(source));
  expect(opened).toHaveLength(1);
  cleanup();
  draw();
  await waitFor(() => expect(opened).toHaveLength(2));
  expect(opened[1]).toMatchObject({ view: "v1", specYaml: source, threadId: "thr1" });
});

test("`#node=` is where the view lands, never a param of its run", async () => {
  const opened: PageKey[] = [];
  const runs = createPageRuns({
    open: async (key) => { opened.push(key); return { runId: "run-1", run: tablePage() }; },
    close: async () => {}, fetch: async () => ({}), setParams: async () => tablePage(), rerun: async () => tablePage(),
    onChange: () => () => {}, onReconnect: () => () => {},
  });
  render(<PageNavigationContext.Provider value={{
    ref: { id: "oxplow:oxplow/view?id=v1#node=t&q=a" }, viewParams: { node: "t", q: "a", "oxplow.renderer": "x" }, historyState: {},
    setHistoryState: () => {}, navigate: () => {}, goBack: () => {}, goForward: () => {}, canGoBack: false, canGoForward: false,
  } as PageNavigation}>
    <EphemeralView view="v1" stream={null} threadId="thr1" runs={runs} />
  </PageNavigationContext.Provider>);
  await waitFor(() => expect(opened).toHaveLength(1));
  expect(opened[0]!.params).toEqual({ q: "a" });
});
