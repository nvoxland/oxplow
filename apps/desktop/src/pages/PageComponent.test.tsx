import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";

// A page file's components: a `component` node is fed its page's loaders
// (every loader's rows by name, the params, its props) and told what
// changed; a page whose view is one component node is that component,
// filling the page, its title and actions drawn by core, its rail the
// page's.
const realApi = await import("../api.js");
const realTransport = await import("../tauri-bridge/transport.js");
const loads: string[] = [];
mock.module("../api.js", () => ({
  ...realApi,
  loadComponent: async (id: string) => {
    loads.push(id);
    return "v-1";
  },
  recordUsage: async () => {},
  subscribeOxplowEvents: () => () => {},
}));
mock.module("../tauri-bridge/transport.js", () => ({ ...realTransport, remoteBaseUrl: () => "http://127.0.0.1:9" }));

const { PageTree } = await import("./PageTree.js");
const { PageHost } = await import("./PageHost.js");
const { decl, fakePage, fakePageRuns, fakeRun, loader, node } = await import("./testing/fakeRun.js");

afterEach(() => {
  cleanup();
  loads.length = 0;
});

/** Load the frame with a stub window that keeps the port `init` carries. */
async function startFrame(frame: HTMLElement) {
  let init: Record<string, unknown> | null = null;
  let port: MessagePort | null = null;
  Object.defineProperty(frame, "contentWindow", {
    value: {
      postMessage: (msg: Record<string, unknown>, _origin: string, ports: MessagePort[]) => {
        init = msg;
        port = ports[0] ?? null;
      },
    },
  });
  fireEvent.load(frame);
  const received: unknown[] = [];
  port!.onmessage = (e) => received.push(e.data);
  return { init: init! as Record<string, unknown>, port: port! as MessagePort, received };
}

function stackWithComponent(rows: string[][]) {
  const page = fakePage(
    node.stack("stack", [
      node.viz("items", "items", { columns: [{ key: "id" }] }),
      { ...node.component("chart", "chart"), props: { color: "accent" } } as never,
    ]),
    { loaders: [decl("items")] },
  );
  return fakeRun(page, { params: { q: "x" }, loaders: { items: loader(["id"], rows) } });
}

test("a component node is fed every loader of its page, its params and props, and hears which loaders changed", async () => {
  const view = render(<PageTree run={stackWithComponent([["a"]])} runId="run-1" streamId="str1" />);
  const frame = await view.findByTestId("custom-component-frame");
  expect(loads).toEqual(["acme/chart"]);
  const { init, port, received } = await startFrame(frame);
  expect(init).toMatchObject({
    protocol: 12,
    run: null,
    data: { items: { columns: ["id"], rows: [["a"]], truncated: false } },
    params: { q: "x" },
    props: { color: "accent" },
    page: null,
  });
  port.postMessage({ type: "ready" });
  view.rerender(<PageTree run={stackWithComponent([["a"], ["b"]])} runId="run-1" streamId="str1" />);
  await waitFor(() => expect(received).toContainEqual(expect.objectContaining({ type: "update", changed: ["items"] })));
});

test("a page whose view is one component fills the page: its title and actions are core's, its rail the page's", async () => {
  const page = fakePage(node.component("board", "board"), {
    title: "Board",
    loaders: [decl("cards")],
    rail: node.viz("legend", "legend", { columns: [{ key: "k" }] }),
  });
  const run = fakeRun(page, { loaders: { cards: loader(["id"], [["c1"]]), legend: loader(["k"], [["key-1"]]) } });
  const { runs } = fakePageRuns(run);
  const titles: string[] = [];
  const nav = {
    setTitle: (t: string) => titles.push(t),
    viewParams: { from: "2026-10-01" },
    historyState: {},
    setHistoryState: () => {},
    patchViewParams: () => {},
    navigate: () => {},
    goBack: () => {},
    goForward: () => {},
    canGoBack: false,
    canGoForward: false,
  } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <PageHost pageKey={{ page: "acme/page", params: {}, streamId: "str1", threadId: "thr1" }} runs={runs} />
    </PageNavigationContext.Provider>,
  );
  const frame = await view.findByTestId("custom-component-frame");
  const { init, port, received } = await startFrame(frame);
  expect(init).toMatchObject({
    data: { cards: { columns: ["id"], rows: [["c1"]], truncated: false }, legend: { columns: ["k"], rows: [["key-1"]], truncated: false } },
    page: { streamId: "str1", threadId: "thr1" },
    view: { viewParams: { from: "2026-10-01" } },
  });
  // No title of the page's own in its body: the component is the page.
  expect(view.container.querySelector("h1.ox-title")).toBeNull();
  // Its rail is the page's.
  expect(view.getByText("key-1")).toBeTruthy();
  port.postMessage({ type: "ready" });
  port.postMessage({ id: "1", method: "setTitle", title: "October" });
  port.postMessage({ id: "2", method: "setActions", actions: [{ id: "fit", label: "Fit", prominent: true }] });
  await waitFor(() => expect(titles.at(-1)).toBe("October"));
  const fit = await view.findByTestId("action-fit");
  await act(async () => {
    fireEvent.click(fit);
  });
  await waitFor(() => expect(received).toContainEqual({ type: "action", id: "fit" }));
});
