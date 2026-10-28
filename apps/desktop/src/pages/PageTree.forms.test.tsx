import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { PageTree } from "./PageTree.js";
import { PageActionContext, PageFormsContext, type PageForms, type RunPageAction } from "./useNodeActions.js";
import { decl, fakeAction, fakePage, fakeRun, loader, node } from "./testing/fakeRun.js";
import type { CommandSpec, FormStart } from "../tauri-bridge/generated/bindings.js";

// A page's form node starts from `page_form` on its open run and submits
// with `submit_page_form`; a form action (a node's row action, or the
// page's own) opens from `page_action_form` and submits through
// `run_page_action` with the input. The page viz components draw both.

afterEach(cleanup);

const UPDATE = {
  id: "oxplow.work_item.update",
  input_schema: { type: "object", required: ["uri"], properties: { uri: { type: "string" }, title: { type: "string" } } },
} as unknown as CommandSpec;

const start = (values: Record<string, unknown>): FormStart =>
  ({ command: UPDATE, values, fixed: ["uri"], fields: ["title"], choices: {}, reads: { datasets: [], tables: [] } }) as unknown as FormStart;

function fakes() {
  const calls: unknown[][] = [];
  const forms: PageForms = {
    start: async (runId, nodeId) => {
      calls.push(["start", runId, nodeId]);
      return start({ uri: "u1", title: "Old" });
    },
    submit: async (runId, nodeId, input, confirmed) => {
      calls.push(["submit", runId, nodeId, input, confirmed]);
      return { result: null, auditId: 1, eventId: null, inverse: null } as never;
    },
    actionStart: async (runId, nodeId, action, row) => {
      calls.push(["actionStart", runId, nodeId, action, row]);
      return start({ uri: row?.id ?? "page", title: "Typed" });
    },
  };
  const press: RunPageAction = async (...args) => {
    calls.push(["press", ...args]);
    return { result: null, auditId: 1, eventId: null, inverse: null } as never;
  };
  return { calls, forms, press };
}

function formPage() {
  const page = fakePage(node.viz("edit", "item", { viz: "form", title: "Save", form: { mode: "edit", then: "stay", again: null } as never }), {
    loaders: [decl("item")],
  });
  return fakeRun(page, { loaders: { item: loader(["uri", "title"], [["u1", "Old"]]) } });
}

test("a form node starts from page_form on the open run and submits with submit_page_form", async () => {
  const { calls, forms, press } = fakes();
  const { findByTestId, getByTestId } = render(
    <PageActionContext.Provider value={press}>
      <PageFormsContext.Provider value={forms}>
        <PageTree run={formPage()} runId="run-1" />
      </PageFormsContext.Provider>
    </PageActionContext.Provider>,
  );
  const field = (await findByTestId("viz-form-fields-title")) as HTMLInputElement;
  expect(field.value).toBe("Old");
  expect(calls[0]).toEqual(["start", "run-1", "edit"]);
  fireEvent.change(field, { target: { value: "New" } });
  fireEvent.click(getByTestId("viz-form-fields-submit"));
  await waitFor(() => expect(calls.some((c) => c[0] === "submit")).toBe(true));
  expect(calls.find((c) => c[0] === "submit")).toEqual(["submit", "run-1", "edit", { uri: "u1", title: "New" }, false]);
});

test("a form node's draft survives the run being read again", async () => {
  const { forms, press } = fakes();
  const ui = (run: ReturnType<typeof formPage>) => (
    <PageActionContext.Provider value={press}>
      <PageFormsContext.Provider value={forms}>
        <PageTree run={run} runId="run-1" />
      </PageFormsContext.Provider>
    </PageActionContext.Provider>
  );
  const view = render(ui(formPage()));
  const field = (await view.findByTestId("viz-form-fields-title")) as HTMLInputElement;
  fireEvent.change(field, { target: { value: "Half typed" } });
  // The daemon re-ran the page: a fresh run object, the same content.
  view.rerender(ui(formPage()));
  expect((view.getByTestId("viz-form-fields-title") as HTMLInputElement).value).toBe("Half typed");
});

test("a node's form action opens from page_action_form and submits through run_page_action", async () => {
  const { calls, forms, press } = fakes();
  const page = fakePage(
    node.viz("items", "items", {
      columns: [{ key: "id" }, { key: "title" }],
      actions: [fakeAction({ id: "retitle", label: "Retitle…", row: true, form: true })],
    }),
    { loaders: [decl("items")] },
  );
  const run = fakeRun(page, { loaders: { items: loader(["id", "title"], [["a", "Alpha"]]) } });
  const { getByTestId, findByTestId } = render(
    <PageActionContext.Provider value={press}>
      <PageFormsContext.Provider value={forms}>
        <PageTree run={run} runId="run-1" />
      </PageFormsContext.Provider>
    </PageActionContext.Provider>,
  );
  fireEvent.contextMenu(getByTestId("viz-row-0"));
  fireEvent.click(getByTestId("menu-item-viz-action-retitle"));
  const field = (await findByTestId("viz-action-form-fields-title")) as HTMLInputElement;
  expect(calls[0]).toEqual(["actionStart", "run-1", "items", "retitle", { id: "a", title: "Alpha" }]);
  fireEvent.change(field, { target: { value: "Renamed" } });
  fireEvent.click(getByTestId("viz-action-form-fields-submit"));
  await waitFor(() => expect(calls.some((c) => c[0] === "press")).toBe(true));
  expect(calls.find((c) => c[0] === "press")).toEqual([
    "press",
    "run-1",
    "items",
    "retitle",
    { id: "a", title: "Alpha" },
    { title: "Renamed" },
    null,
    false,
  ]);
});
