import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { PageHost } from "./PageHost.js";
import { PageActionContext, PageFormsContext, type PageForms, type RunPageAction } from "./useNodeActions.js";
import { fakeAction, fakePageRuns, gridPage, loader, tablePage } from "./testing/fakeRun.js";
import { createPageRuns, type PageKey } from "../pageRuns.js";
import { IpcCallError } from "../ipc-error.js";
import type { CommandSpec, FormStart, PageRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { paramOf } from "../uri/uri.js";

afterEach(cleanup);

const key: PageKey = { page: "acme/page", params: {}, streamId: null, threadId: null };
const wait = (ms: number) => act(() => new Promise<void>((r) => setTimeout(r, ms)));

test("every page offers Show definition with the page and the run's actual params", async () => {
  const opened: string[] = [];
  const { runs } = fakePageRuns({ ...tablePage(), params: { q: "recent" } });
  const view = render(<PageHost pageKey={key} runs={runs} onOpenPage={(ref) => opened.push(ref.id)} />);
  fireEvent.click(await view.findByTestId("action-page-definition"));
  expect(paramOf(opened[0], "of")).toBe("acme/page");
  expect(paramOf(opened[0], "params")).toBe('{"q":"recent"}');
});

test("an inline view offers saves without requiring a single SQL viz", async () => {
  const yaml = "title: Snapshot\ndata: { items: { rows: [{ title: Alpha }] } }\nview: { viz: table, from: items }\n";
  const kept: string[] = [];
  const { runs } = fakePageRuns(gridPage());
  const view = render(<PageHost pageKey={{ ...key, page: undefined, specYaml: yaml }} runs={runs} onSpecYaml={(source) => kept.push(source)} />);
  await view.findByTestId("action-view-save-page");
  expect(view.getByTestId("action-view-save-wiki").textContent).toContain("Save to wiki");
  expect(kept).toContain(yaml);
});

test("the title is the page's label over its params and its first loader's first row", async () => {
  const { runs } = fakePageRuns(tablePage());
  const { findByText } = render(<PageHost pageKey={key} runs={runs} />);
  expect((await findByText("Items: Alpha")).tagName).toBe("H1");
});

test("a whole-page action presses on the run, asking first when its command does", async () => {
  const calls: unknown[][] = [];
  const press: RunPageAction = async (...args) => {
    calls.push(args);
    if (!args[6]) throw new IpcCallError("Archive needs confirmation", "NEEDS_CONFIRMATION", null, []);
    return { result: null, audit_id: null, event_id: null, inverse: null } as never;
  };
  const { runs } = fakePageRuns(tablePage({ actions: [fakeAction({ id: "archive", label: "Archive" })] }));
  const { findByTestId, queryByTestId } = render(
    <PageActionContext.Provider value={press}>
      <PageHost pageKey={key} runs={runs} />
    </PageActionContext.Provider>,
  );
  fireEvent.click(await findByTestId("action-page-action-archive"));
  await waitFor(() => expect(calls).toEqual([["run-1", null, "archive", null, null, null, false]]));
  fireEvent.click(await findByTestId("page-action-confirm-run"));
  await waitFor(() => expect(calls.at(-1)).toEqual(["run-1", null, "archive", null, null, null, true]));
  await waitFor(() => expect(queryByTestId("page-action-confirm")).toBeNull());
});

test("a whole-page form action opens its form from page_action_form and submits it as a press", async () => {
  const calls: unknown[][] = [];
  const press: RunPageAction = async (...args) => {
    calls.push(["press", ...args]);
    return { result: null, audit_id: null, event_id: null, inverse: null } as never;
  };
  const command = {
    id: "acme.add",
    input_schema: { type: "object", properties: { title: { type: "string" }, notes: { type: "string" } } },
  } as unknown as CommandSpec;
  const forms: PageForms = {
    start: async () => Promise.reject(new Error("no form node")),
    submit: async () => Promise.reject(new Error("no form node")),
    actionStart: async (...args) => {
      calls.push(["actionStart", ...args]);
      return { command, values: { title: "" }, fixed: [], fields: null, choices: {} } as unknown as FormStart;
    },
  };
  const { runs } = fakePageRuns(tablePage({ actions: [fakeAction({ id: "add", label: "Add…", form: true })] }));
  const { findByTestId, getByTestId, queryByTestId } = render(
    <PageActionContext.Provider value={press}>
      <PageFormsContext.Provider value={forms}>
        <PageHost pageKey={key} runs={runs} />
      </PageFormsContext.Provider>
    </PageActionContext.Provider>,
  );
  fireEvent.click(await findByTestId("action-page-action-add"));
  const title = (await findByTestId("lens-action-form-fields-title")) as HTMLInputElement;
  expect(calls[0]).toEqual(["actionStart", "run-1", null, "add", null]);
  expect(queryByTestId("page-action-form-unsupported")).toBeNull();
  fireEvent.change(title, { target: { value: "Gamma" } });
  fireEvent.click(getByTestId("lens-action-form-fields-submit"));
  await waitFor(() => expect(calls.at(-1)).toEqual(["press", "run-1", null, "add", null, { title: "Gamma" }, null, false]));
});

test("Refresh is offered only when the page asks for it, and re-runs the run", async () => {
  const plain = fakePageRuns(tablePage());
  const first = render(<PageHost pageKey={key} runs={plain.runs} />);
  await first.findByText("Items: Alpha");
  expect(first.queryByTestId("action-page-refresh")).toBeNull();
  cleanup();
  const run = tablePage();
  const refreshing = fakePageRuns({ ...run, page: { ...run.page, refresh: true } });
  const view = render(<PageHost pageKey={key} runs={refreshing.runs} />);
  fireEvent.click(await view.findByTestId("action-page-refresh"));
  await waitFor(() => expect(refreshing.calls.rerun).toBe(1));
});

test("a filter changed in the rail re-runs the page with it; the old rows stay until the new run answers", async () => {
  let answer: (run: PageRun) => void = () => {};
  const base = tablePage({ params: [{ name: "q", label: "Query", role: "filter", type: "text", default: null }] });
  const { runs, calls } = fakePageRuns(base, () => new Promise<PageRun>((r) => (answer = r)));
  const view = render(<PageHost pageKey={key} runs={runs} />);
  fireEvent.click(await view.findByTestId("lens-param-q-value"));
  const input = view.getByTestId("lens-param-q-edit") as HTMLInputElement;
  fireEvent.change(input, { target: { value: "be" } });
  fireEvent.keyDown(input, { key: "Enter" });
  await waitFor(() => expect(calls.setParams).toEqual([{ q: "be" } as Record<string, SqlCell>]));
  await wait(200);
  // Stale: still the old rows, no Loading.
  expect(view.getByTestId("lens-row-0").textContent).toContain("Alpha");
  expect(view.container.querySelector(".ox-loading")).toBeNull();
  expect(view.getByTestId("page-file").getAttribute("data-stale")).toBe("true");
  await act(async () => answer({ ...base, params: { q: "be" }, loaders: { items: loader(["id", "title"], [["b", "Beta"]]) } }));
  await waitFor(() => expect(view.getByTestId("lens-row-0").textContent).toContain("Beta"));
  expect(view.getByTestId("page-file").hasAttribute("data-stale")).toBe(false);
});

test("Loading shows only after 150 ms without a run; an open that fails says why", async () => {
  const runs = createPageRuns({
    open: () => new Promise(() => {}),
    close: async () => {},
    fetch: async () => ({}),
    setParams: async () => Promise.reject(new Error("no")),
    rerun: async () => Promise.reject(new Error("no")),
    onChange: () => () => {},
    onReconnect: () => () => {},
  });
  const view = render(<PageHost pageKey={key} runs={runs} />);
  expect(view.container.querySelector(".ox-loading")).toBeNull();
  await wait(200);
  expect(view.container.querySelector(".ox-loading")).not.toBeNull();
  cleanup();
  const failing = createPageRuns({
    open: async () => Promise.reject(new Error("no page acme/page")),
    close: async () => {},
    fetch: async () => ({}),
    setParams: async () => Promise.reject(new Error("no")),
    rerun: async () => Promise.reject(new Error("no")),
    onChange: () => () => {},
    onReconnect: () => () => {},
  });
  const failed = render(<PageHost pageKey={key} runs={failing} />);
  await waitFor(() => expect(failed.container.querySelector(".ox-error")?.textContent).toContain("no page acme/page"));
});

test("a grid page takes the full width; a table reads as a document", async () => {
  const grid = render(<PageHost pageKey={key} runs={fakePageRuns(gridPage()).runs} />);
  await waitFor(() => expect(grid.container.querySelector('[data-node="tiles"]')).not.toBeNull());
  expect(grid.container.querySelector(".oxplow-reading-column")).toBeNull();
  cleanup();
  const table = render(<PageHost pageKey={key} runs={fakePageRuns(tablePage()).runs} />);
  await table.findByText("Items: Alpha");
  expect(table.container.querySelector(".oxplow-reading-column")).not.toBeNull();
});

test("`focus` lands on its node once the run draws it", async () => {
  const view = render(<PageHost pageKey={key} runs={fakePageRuns(tablePage()).runs} focus="items" />);
  await waitFor(() => expect(view.container.querySelector('[data-node="items"]')?.getAttribute("data-focused")).toBe("true"));
});

// A page whose view is a switch is one page of sections, as a view
// document was: its rail links each section (the one in view marked), a
// link sets the switch's param, and the section in view titles the body.
test("a switch page's sections are links in its rail; the one in view titles the body", async () => {
  const { fakePage, fakeRun, node, decl } = await import("./testing/fakeRun.js");
  const page = fakePage(
    node.switch("areas", "area", [
      node.stack("overview", [node.viz("a", "items", { columns: [{ key: "id" }] }, { title: "All" })], { title: "Every Setting" }),
      node.stack("agents", [node.viz("b", "items", { columns: [{ key: "id" }] }, { title: "Agents list" })], { title: "Agents" }),
    ]),
    { title: "Settings", params: [{ name: "area", role: "filter", default: "overview", type: null, options: [] } as never], loaders: [decl("items")] },
  );
  const run = fakeRun(page, { params: { area: "overview" }, loaders: { items: loader(["id"], [["x"]]) }, pick: { areas: "overview" } });
  const { runs } = fakePageRuns(run);
  const changes: Record<string, SqlCell>[] = [];
  const view = render(<PageHost pageKey={key} runs={runs} onChange={(p) => changes.push(p)} />);
  expect((await view.findByRole("heading", { level: 1 })).textContent).toBe("Every Setting");
  expect(view.getByTestId("page-section-overview").getAttribute("aria-current")).toBe("page");
  // The switch's param is the sections, not a filter.
  expect(view.queryByTestId("lens-param-area")).toBeNull();
  fireEvent.click(view.getByTestId("page-section-agents"));
  expect(changes).toEqual([{ area: "agents" }]);
});
