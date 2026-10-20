import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { ExtensionPage } from "../tauri-bridge/generated/bindings.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";

// A declared page whose body is a custom component: core's chrome and
// nav bar around the component's sandboxed frame, which sets the title and
// actions over the bridge.
const realApi = await import("../api.js");
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

const { CustomPageView } = await import("./CustomPageView.js");
const { DeclaredPageView } = await import("./DeclaredPageView.js");

afterEach(() => {
  cleanup();
  loads.length = 0;
});

const declared = {
  id: "board",
  extension: "acme",
  pageRef: "page:acme/board",
  title: "Board",
  icon: null,
  category: "Work",
  keywords: null,
  params: [],
  count: null,
  body: { kind: "component", component: "acme/board" },
} as unknown as ExtensionPage;
const page = { extension: "acme", page: "board", params: { a: "1" } };

function setup() {
  const titles: string[] = [];
  const nav = {
    navigate: () => {},
    goBack: () => {},
    goForward: () => {},
    canGoBack: false,
    canGoForward: false,
    setTitle: (t: string) => titles.push(t),
  } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <CustomPageView page={page} declared={declared} stream={null} threadId="thr1" onOpenPage={() => {}} base="http://127.0.0.1:9" />
    </PageNavigationContext.Provider>,
  );
  return { view, titles };
}

/** Load the frame with a stub window that keeps the port `init` carries:
 *  the frame's end of the bridge. */
async function startFrame(view: ReturnType<typeof render>) {
  const frame = await view.findByTestId("custom-component-frame");
  let init: unknown = null;
  let port: MessagePort | null = null;
  Object.defineProperty(frame, "contentWindow", {
    value: {
      postMessage: (msg: unknown, _origin: string, ports: MessagePort[]) => {
        init = msg;
        port = ports[0] ?? null;
      },
    },
  });
  fireEvent.load(frame);
  const received: unknown[] = [];
  port!.onmessage = (e) => received.push(e.data);
  return { init: init as Record<string, unknown>, port: port! as MessagePort, received };
}

test("a component page mounts a marked frame under the declared title, with the page's params", async () => {
  const { view, titles } = setup();
  const { init } = await startFrame(view);
  expect(loads).toEqual(["acme/board"]);
  expect(view.getByTestId("page-custom")).toBeTruthy();
  expect(view.getByTestId("custom-component-badge").textContent).toBe("custom");
  expect(titles[titles.length - 1]).toBe("Board");
  expect(init.run).toBeNull();
  expect(init.page).toEqual({ params: { a: "1" }, streamId: null, threadId: "thr1" });
});

test("the frame's setTitle changes the page's title", async () => {
  const { view, titles } = setup();
  const { port } = await startFrame(view);
  port.postMessage({ id: "1", method: "setTitle", title: "Sprint 12" });
  await waitFor(() => expect(titles[titles.length - 1]).toBe("Sprint 12"));
});

test("the frame's actions are the page's, and a press reaches the frame", async () => {
  const { view } = setup();
  const { port, received } = await startFrame(view);
  port.postMessage({
    id: "2",
    method: "setActions",
    actions: [{ id: "add", label: "Add card", primary: true, disabled: false }],
  });
  const button = await view.findByTestId("action-add");
  expect(button.textContent).toContain("Add card");
  fireEvent.click(button);
  await waitFor(() => expect(received).toContainEqual({ type: "action", id: "add" }));
});

test("with no daemon the page says it needs one", () => {
  const view = render(<CustomPageView page={page} declared={declared} stream={null} onOpenPage={() => {}} base={null} />);
  expect(view.getByText("Needs the daemon")).toBeTruthy();
  expect(view.queryByTestId("custom-component-frame")).toBeNull();
});

test("DeclaredPageView shows a component page through CustomPageView", () => {
  const view = render(<DeclaredPageView page={page} declared={declared} stream={null} onOpenPage={() => {}} />);
  expect(view.getByTestId("page-custom")).toBeTruthy();
  expect(view.queryByTestId("page-missing")).toBeNull();
});
