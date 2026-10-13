import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import type { CommandSpec, Lens, LensRun } from "../tauri-bridge/generated/bindings.js";
import { IpcCallError } from "../ipc-error.js";

// A lens action with `form: true` opens its command's form in a slideover,
// its fields starting from the action's bound input and its fixed fields
// shown but not editable; submitting runs it with what the person filled
// in, an invalid field's message lands under that field, and success
// closes it. A form lens shows the same per-field errors.

const realApi = await import("../api.js");
const runs: Array<{ action: string; row: unknown; input: unknown; confirmed: boolean }> = [];
let invalidField: string | null = null;
const submits: unknown[] = [];
// What the item's title is in the models: a form lens starts from it.
let storedTitle = "Old";
let formStarts = 0;

const UPDATE: CommandSpec = {
  id: "oxplow.work_item.update",
  input_schema: {
    type: "object",
    required: ["ref"],
    properties: { ref: { type: "string" }, title: { type: "string" } },
  },
} as unknown as CommandSpec;

mock.module("../api.js", () => ({
  ...realApi,
  lensActionForm: async (_id: string, _action: string, _params: unknown, row: Record<string, unknown> | null) => ({
    command: UPDATE,
    values: { ref: row?.ref, title: row?.title },
    fixed: ["ref"],
    fields: null,
  }),
  runLensAction: async (
    _id: string,
    action: string,
    _params: unknown,
    row: unknown,
    _streamId: unknown,
    confirmed: boolean,
    input: unknown,
  ) => {
    runs.push({ action, row, input, confirmed });
    if (invalidField) throw new IpcCallError("a title can't be empty", "INVALID", invalidField);
    return { result: null, auditId: 1, eventId: null, inverse: null };
  },
  lensForm: async () => {
    formStarts += 1;
    return {
    command: UPDATE,
    values: { ref: "work_item:oxplow:tsk1", title: storedTitle },
    fixed: ["ref"],
    fields: ["title"],
    };
  },
  submitLensForm: async (_id: string, input: unknown) => {
    submits.push(input);
    if (!invalidField) storedTitle = (input as { title?: string }).title ?? storedTitle;
    if (invalidField) throw new IpcCallError("a title can't be empty", "INVALID", invalidField);
    return { result: null, auditId: 1, eventId: null, inverse: null };
  },
}));

const { LensResultView } = await import("./LensResultView.js");

beforeEach(() => {
  runs.length = 0;
  submits.length = 0;
  invalidField = null;
  storedTitle = "Old";
  formStarts = 0;
});
afterEach(cleanup);

const lens: Lens = {
  id: "x/l",
  extension: "x",
  slug: "l",
  title: "Edit",
  description: "",
  query: "",
  viz: "table",
  params: [],
  columns: [],
  empty: null,
  chart: null,
  tree: null,
  timeline: null,
  steps: null,
  hunks: null,
  form: null,
  children: [],
  launcherCategory: null,
  hidden: false,
  actions: [
    {
      id: "retitle",
      label: "Retitle…",
      command: "oxplow.work_item.update",
      input: { ref: "{{row.ref}}", title: "{{row.title}}" },
      row: true,
      group: null,
      form: true,
      fixed: ["ref"],
    },
  ],
  alert: null,
  path: "",
} as unknown as Lens;
const run: LensRun = {
  lens,
  params: {},
  result: { columns: ["ref", "title"], rows: [["work_item:oxplow:tsk1", "Old"]], truncated: false },
} as unknown as LensRun;

test("a form action opens its form in a slideover and runs what the person filled in", async () => {
  const view = render(<LensResultView run={run} onOpenPage={() => {}} />);
  fireEvent.contextMenu(view.getByTestId("lens-row-0"));
  fireEvent.click(view.getByTestId("menu-item-lens-action-retitle"));
  // It opened a form instead of running.
  await waitFor(() => view.getByTestId("lens-action-form"));
  expect(runs).toEqual([]);
  // The fixed ref is shown, not an input; the title starts from the row.
  expect(view.getByTestId("lens-action-form-fields-ref-fixed").textContent).toContain("work_item:oxplow:tsk1");
  const title = view.getByTestId("lens-action-form-fields-title") as HTMLInputElement;
  expect(title.value).toBe("Old");
  fireEvent.change(title, { target: { value: "New" } });
  fireEvent.submit(view.getByTestId("lens-action-form-fields"));
  await waitFor(() => expect(runs).toHaveLength(1));
  expect(runs[0].input).toEqual({ ref: "work_item:oxplow:tsk1", title: "New" });
  expect(runs[0].row).toEqual({ ref: "work_item:oxplow:tsk1", title: "Old" });
  // Success closes it.
  await waitFor(() => expect(view.queryByTestId("lens-action-form")).toBeNull());
});

test("an invalid field's message shows under it, and the form stays open", async () => {
  invalidField = "/title";
  const view = render(<LensResultView run={run} onOpenPage={() => {}} />);
  fireEvent.contextMenu(view.getByTestId("lens-row-0"));
  fireEvent.click(view.getByTestId("menu-item-lens-action-retitle"));
  await waitFor(() => view.getByTestId("lens-action-form-fields-title"));
  fireEvent.change(view.getByTestId("lens-action-form-fields-title"), { target: { value: "" } });
  fireEvent.submit(view.getByTestId("lens-action-form-fields"));
  const error = await waitFor(() => view.getByTestId("lens-action-form-fields-title-error"));
  expect(error.textContent).toContain("can't be empty");
  expect(view.getByTestId("lens-action-form")).toBeTruthy();
});

test("a form lens shows only its fields, keeps its fixed ones, and puts errors under the field", async () => {
  invalidField = "/title";
  const form: LensRun = {
    lens: { ...lens, viz: "form", form: { command: "oxplow.work_item.update", defaults: null, fixed: ["ref"], fields: ["title"] }, actions: [] },
    params: {},
    result: { columns: [], rows: [], truncated: false },
  } as unknown as LensRun;
  const view = render(<LensResultView run={form} onOpenPage={() => {}} />);
  const title = await waitFor(() => view.getByTestId("lens-form-fields-title") as HTMLInputElement);
  expect(view.queryByTestId("lens-form-fields-ref")).toBeNull();
  fireEvent.change(title, { target: { value: "" } });
  fireEvent.submit(view.getByTestId("lens-form-fields"));
  // An emptied text field is left out; the fixed ref is always sent.
  await waitFor(() => expect(submits).toEqual([{ ref: "work_item:oxplow:tsk1" }]));
  const error = await waitFor(() => view.getByTestId("lens-form-fields-title-error"));
  expect(error.textContent).toContain("can't be empty");
});

// An edit form shows what it saved: after a successful submit it starts
// again from the models, not from the values it first opened with.
test("a form lens starts again from the current values after it saves", async () => {
  const form: LensRun = {
    lens: { ...lens, viz: "form", form: { command: "oxplow.work_item.update", defaults: null, fixed: ["ref"], fields: ["title"] }, actions: [] },
    params: {},
    result: { columns: [], rows: [], truncated: false },
  } as unknown as LensRun;
  const view = render(<LensResultView run={form} onOpenPage={() => {}} />);
  const title = await waitFor(() => view.getByTestId("lens-form-fields-title") as HTMLInputElement);
  fireEvent.change(title, { target: { value: "New" } });
  fireEvent.submit(view.getByTestId("lens-form-fields"));
  await waitFor(() => expect(submits).toEqual([{ ref: "work_item:oxplow:tsk1", title: "New" }]));
  await waitFor(() => expect(formStarts).toBe(2));
  await waitFor(() => expect((view.getByTestId("lens-form-fields-title") as HTMLInputElement).value).toBe("New"));
});
