import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { PageTree } from "./PageTree.js";
import { PageActionContext, PageFormsContext, type PageForms, type RunPageAction } from "./useNodeActions.js";
import { decl, fakeAction, fakePage, fakeRun, loader, node } from "./testing/fakeRun.js";
import { TabModalHostContext } from "../components/TabModal.js";
import type { CommandSpec, FormStart } from "../tauri-bridge/generated/bindings.js";

// A node's form action ("Link…") is open and the person has typed into it:
// whatever arrives after — the page read again, another node's rows
// changing, the form's host being set — must leave the same field standing
// with what was typed. (A browser run once found the field empty a few
// milliseconds after it was filled.)

afterEach(cleanup);

const LINK = {
  id: "oxplow.work_item.link",
  input_schema: {
    type: "object",
    required: ["uri", "target", "link_type"],
    properties: { uri: { type: "string" }, target: { type: "string" }, link_type: { type: "string" } },
  },
} as unknown as CommandSpec;

const start = (): FormStart =>
  ({
    command: LINK,
    values: { uri: "u1", link_type: "relates_to" },
    fixed: ["uri"],
    fields: null,
    choices: {},
    reads: { datasets: [], tables: [] },
  }) as unknown as FormStart;

const forms: PageForms = {
  start: async () => start(),
  submit: async () => ({ result: null, auditId: 1, eventId: null, inverse: null }) as never,
  actionStart: async () => start(),
};
const press: RunPageAction = async () => ({ result: null, auditId: 1, eventId: null, inverse: null }) as never;

/** The page of a work item: its links (with the Link… form action) over
 *  its comments, which change as the page is read again. */
function page(comments: string[]) {
  const view = node.stack("page", [
    node.viz("links", "links", {
      columns: [{ key: "item" }],
      actions: [fakeAction({ id: "link", label: "Link…", form: true })],
    }),
    node.viz("comments", "comments", { columns: [{ key: "body" }] }),
  ]);
  return fakeRun(fakePage(view, { loaders: [decl("links"), decl("comments")] }), {
    loaders: { links: loader(["item"], []), comments: loader(["body"], comments.map((c) => [c])) },
  });
}

const tree = (run: ReturnType<typeof page>, host: HTMLElement | null) => (
  <TabModalHostContext.Provider value={host}>
    <PageActionContext.Provider value={press}>
      <PageFormsContext.Provider value={forms}>
        <PageTree run={run} runId="run-1" />
      </PageFormsContext.Provider>
    </PageActionContext.Provider>
  </TabModalHostContext.Provider>
);

async function typed() {
  const host = document.createElement("div");
  document.body.append(host);
  const view = render(tree(page(["first"]), host));
  fireEvent.click(view.getByTestId("viz-action-link"));
  const field = (await view.findByTestId("viz-action-form-fields-target")) as HTMLInputElement;
  fireEvent.change(field, { target: { value: "oxplow:oxplow/work_item?id=tsk3" } });
  expect(field.value).toBe("oxplow:oxplow/work_item?id=tsk3");
  return { view, field, host };
}

const standing = (view: ReturnType<typeof render>, field: HTMLInputElement) => {
  const now = view.getByTestId("viz-action-form-fields-target") as HTMLInputElement;
  expect(now).toBe(field);
  expect(now.value).toBe("oxplow:oxplow/work_item?id=tsk3");
};

test("the page read again with the same content leaves the typed field standing", async () => {
  const { view, field, host } = await typed();
  view.rerender(tree(page(["first"]), host));
  standing(view, field);
});

test("another node's rows changing leaves the typed field standing", async () => {
  const { view, field, host } = await typed();
  view.rerender(tree(page(["first", "Noted by the suite's effect."]), host));
  standing(view, field);
});
