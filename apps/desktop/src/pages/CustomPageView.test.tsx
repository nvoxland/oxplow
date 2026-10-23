import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { ExtensionPage } from "../tauri-bridge/generated/bindings.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";

// A declared page whose body is a custom component: core's chrome and
// nav bar around the component's sandboxed frame, which sets the title and
// actions over the bridge.
const realApi = await import("../api.js");
const { IpcCallError } = await import("../ipc-error.js");
const loads: string[] = [];
const specRuns: unknown[] = [];
const railRuns: unknown[] = [];
const rows = (columns: string[], rows: unknown[][]) => ({ columns, rows, truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} });
const railLens = {
  id: "acme/models",
  title: "Models",
  viz: "list",
  params: [{ name: "model", label: null, default: "", options: [], type: "text", role: "selection", presets: [] }],
  columns: [{ key: "view", label: null, link: null, unit: null, icon: null, tone: null, select: "model" }],
  actions: [],
  parts: [],
};
mock.module("../api.js", () => ({
  ...realApi,
  runLensSpec: async (spec: { query: string; viz: string }, _params: unknown, stream: string | null) => {
    specRuns.push([spec, stream]);
    if (spec.query === "bad") throw new IpcCallError("no such table: nope", "INVALID", "/spec/query");
    return { lens: { id: "spec/shown", title: "Explore", viz: spec.viz, params: [], columns: [], actions: [], parts: [] }, params: {}, result: rows(["title"], [["Echidna"]]) };
  },
  getLens: async () => railLens,
  runLens: async (id: string, params: unknown) => {
    railRuns.push([id, params]);
    return { lens: railLens, params, result: rows(["view"], [["v_work_item"], ["v_effort"]]) };
  },
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
  specRuns.length = 0;
  railRuns.length = 0;
});

const declared = {
  id: "board",
  extension: "acme",
  pageRef: "oxplow:oxplow/page?page=acme/board",
  title: "Board",
  icon: null,
  category: "Work",
  keywords: null,
  params: [],
  count: null,
  body: { kind: "component", component: "acme/board" },
} as unknown as ExtensionPage;
const page = { extension: "acme", page: "board", params: { a: "1" } };

function setup(over: Partial<ExtensionPage> = {}) {
  const titles: string[] = [];
  const patched: [Record<string, unknown>, unknown][] = [];
  const nav = {
    patchViewParams: (patch: Record<string, unknown>, opts: unknown) => patched.push([patch, opts]),
    navigate: () => {},
    goBack: () => {},
    goForward: () => {},
    canGoBack: false,
    canGoForward: false,
    setTitle: (t: string) => titles.push(t),
    viewParams: { a: "1" },
    historyState: { scroll: 3 },
    setHistoryState: () => {},
  } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <CustomPageView page={page} declared={{ ...declared, ...over } as ExtensionPage} stream={{ id: "str2" } as never} threadId="thr1" onOpenPage={() => {}} base="http://127.0.0.1:9" />
    </PageNavigationContext.Provider>,
  );
  return { view, titles, patched };
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

test("a component page mounts a marked frame under the declared title, with its view params and history state", async () => {
  const { view, titles } = setup();
  const { init } = await startFrame(view);
  expect(loads).toEqual(["acme/board"]);
  expect(view.getByTestId("page-custom")).toBeTruthy();
  expect(view.getByTestId("custom-component-badge").textContent).toBe("custom");
  expect(titles[titles.length - 1]).toBe("Board");
  expect(init.run).toBeNull();
  expect(init.page).toEqual({ streamId: "str2", threadId: "thr1" });
  expect(init.view).toEqual({ viewParams: { a: "1" }, historyState: { scroll: 3 } });
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

// The frame hands the page a spec: the page runs it and draws it with the
// kit under the frame, with Save as Lens and Pin to Dashboard.
test("a frame's showSpec is drawn under it; a refused one answers INVALID with its field; null clears it", async () => {
  const { view } = setup();
  const { port, received } = await startFrame(view);
  port.postMessage({ id: "1", method: "showSpec", spec: { title: "Explore", query: "SELECT title FROM v_work_item", viz: "table" } });
  const shown = await view.findByTestId("custom-page-spec");
  await waitFor(() => expect(shown.textContent).toContain("Echidna"));
  expect(view.getByTestId("explore-save-open")).toBeTruthy();
  expect(view.getByTestId("custom-page-pin")).toBeTruthy();
  expect(specRuns[0]).toEqual([{ title: "Explore", query: "SELECT title FROM v_work_item", viz: "table" }, "str2"]);
  await waitFor(() => expect(received).toContainEqual({ id: "1", ok: true, result: { columns: ["title"], rows: [["Echidna"]], truncated: false } }));
  port.postMessage({ id: "2", method: "showSpec", spec: { title: "Explore", query: "bad", viz: "table" } });
  await waitFor(() =>
    expect(received).toContainEqual({ id: "2", ok: false, error: { code: "INVALID", message: "no such table: nope", field: "/spec/query" } }),
  );
  // A refusal leaves what was shown.
  expect(view.getByTestId("custom-page-spec")).toBeTruthy();
  port.postMessage({ id: "3", method: "showSpec", spec: null });
  await waitFor(() => expect(view.queryByTestId("custom-page-spec")).toBeNull());
});

// Protocol 11: a form of the frame's own schema. The host draws it; the
// frame is answered the values submitted, or CANCELLED when it is closed.
test("a frame's own form is drawn by the host and answers the submitted values, or CANCELLED", async () => {
  const { view } = setup();
  const { port, received } = await startFrame(view);
  const schema = { type: "object", properties: { team: { type: "string", title: "Team" } }, required: ["team"] };
  port.postMessage({ id: "1", method: "openForm", form: { title: "Configure", schema, values: { team: "core" } } });
  const input = (await view.findByTestId("custom-page-own-form-fields-team")) as HTMLInputElement;
  expect(input.value).toBe("core");
  fireEvent.change(input, { target: { value: "ops" } });
  fireEvent.click(view.getByTestId("custom-page-own-form-fields-submit"));
  await waitFor(() => expect(received).toContainEqual({ id: "1", ok: true, result: { team: "ops" } }));
  await waitFor(() => expect(view.queryByTestId("custom-page-own-form")).toBeNull());
  port.postMessage({ id: "2", method: "openForm", form: { title: "Configure", schema, values: {} } });
  await view.findByTestId("custom-page-own-form");
  fireEvent.keyDown(document.body, { key: "Escape" });
  await waitFor(() =>
    expect(received).toContainEqual({ id: "2", ok: false, error: { code: "CANCELLED", message: "The person closed the form." } }),
  );
});

test("with a spec shown, the frame sits at the height its content says", async () => {
  const { view } = setup();
  const { port } = await startFrame(view);
  const frame = view.getByTestId("custom-component-frame");
  port.postMessage({ type: "size", height: 260 });
  await new Promise((r) => setTimeout(r, 20));
  // No spec yet: the frame fills the page.
  expect(frame.style.height).toBe("");
  port.postMessage({ id: "1", method: "showSpec", spec: { title: "Explore", query: "SELECT 1", viz: "table" } });
  await view.findByTestId("custom-page-spec");
  await waitFor(() => expect(frame.style.height).toBe("260px"));
});

test("a page's rail lens is drawn in its rail, run with the page's view; a pick pushes the view", async () => {
  const { view, patched } = setup({ rail: "acme/models" } as Partial<ExtensionPage>);
  const rail = await view.findByTestId("custom-page-rail");
  await waitFor(() => expect(rail.textContent).toContain("v_work_item"));
  expect(railRuns[0]).toEqual(["acme/models", {}]);
  fireEvent.click(view.getByTestId("lens-row-0"));
  expect(patched).toEqual([[{ model: "v_work_item" }, { push: true }]]);
});
