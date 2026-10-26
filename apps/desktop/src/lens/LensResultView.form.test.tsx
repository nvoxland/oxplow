import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render as rtlRender, waitFor } from "@testing-library/react";
import { useState, type ReactElement, type ReactNode } from "react";
import type { ActionThen, CommandSpec, Lens, LensRun } from "../tauri-bridge/generated/bindings.js";
import { IpcCallError } from "../ipc-error.js";
import { LensActionRunnerContext, type LensActionRunner } from "./lensActionRunner.js";
import { FormSourceContext, type FormSource } from "./formSource.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";

// A lens action with `form: true` opens its command's form in a slideover,
// its fields starting from the action's bound input and its fixed fields
// shown but not editable; submitting runs it with what the person filled
// in, an invalid field's message lands under that field, and success
// closes it. A form that is one text field is a prompt strip at the top of
// the lens instead. A form lens shows the same per-field errors, and after
// a submit goes where its command says, as its `then` says.

const realApi = await import("../api.js");
const runs: Array<{ action: string; row: unknown; input: unknown; confirmed: boolean }> = [];
let invalidField: string | null = null;
const submits: unknown[] = [];
// What the item's title is in the models: a form lens starts from it.
let storedTitle = "Old";
let formStarts = 0;
// When set, a form lens always starts from these values (not what was stored).
let fixedStart: Record<string, unknown> | null = null;
// The page the command names after a run (`ui.open_after`), if any.
let openAfterPage: string | null = null;

const UPDATE: CommandSpec = {
  id: "oxplow.work_item.update",
  input_schema: {
    type: "object",
    required: ["uri"],
    properties: { uri: { type: "string" }, title: { type: "string", description: "What to call it" } },
  },
} as unknown as CommandSpec;
// The same command with a second editable field.
const UPDATE_WIDE: CommandSpec = {
  id: "oxplow.work_item.update",
  input_schema: {
    type: "object",
    required: ["uri"],
    properties: { uri: { type: "string" }, title: { type: "string" }, notes: { type: "string" } },
  },
} as unknown as CommandSpec;
// A command whose one editable field is a choice.
const DECIDE: CommandSpec = {
  id: "oxplow.work_item.decide",
  input_schema: {
    type: "object",
    required: ["uri"],
    properties: { uri: { type: "string" }, title: { type: "string", enum: ["Old", "New"] } },
  },
} as unknown as CommandSpec;
// The command a form action's form is of.
let actionCommand: CommandSpec = UPDATE;

mock.module("../api.js", () => ({
  ...realApi,
  commandOpenAfter: async () => openAfterPage,
}));

// A viz starts and submits its forms, and presses its actions, through the
// page that draws it: these stand in for it.
const formSource: FormSource = {
  actionStart: async (_target, _action, row) => ({
    command: actionCommand,
    values: { uri: row?.uri, title: row?.title },
    fixed: ["uri"],
    fields: null,
  }) as never,
  start: async () => {
    formStarts += 1;
    return {
      command: UPDATE,
      values: fixedStart ?? { uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: storedTitle },
      fixed: ["uri"],
      fields: ["title"],
    } as never;
  },
  submit: async (_target, input) => {
    submits.push(input);
    if (!invalidField) storedTitle = (input as { title?: string }).title ?? storedTitle;
    if (invalidField) throw new IpcCallError("a title can't be empty", "INVALID", invalidField);
    return { result: null, audit_id: 1, event_id: null, inverse: null } as never;
  },
};
const press: LensActionRunner = async (_id, action, _params, row, _streamId, confirmed, input) => {
  runs.push({ action, row, input, confirmed });
  if (invalidField) throw new IpcCallError("a title can't be empty", "INVALID", invalidField);
  return { result: null, audit_id: 1, event_id: null, inverse: null };
};
const wrapper = ({ children }: { children: ReactNode }) => (
  <LensActionRunnerContext.Provider value={press}>
    <FormSourceContext.Provider value={formSource}>{children}</FormSourceContext.Provider>
  </LensActionRunnerContext.Provider>
);
const render = (ui: ReactElement) => rtlRender(ui, { wrapper });

const { LensResultView } = await import("./LensResultView.js");

beforeEach(() => {
  runs.length = 0;
  submits.length = 0;
  invalidField = null;
  storedTitle = "Old";
  formStarts = 0;
  fixedStart = null;
  openAfterPage = null;
  actionCommand = UPDATE;
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
  tile: null,
  timeline: null,
  steps: null,
  hunks: null,
  form: null,
  parts: [],
  launcherCategory: null,
  hidden: false,
  actions: [
    {
      id: "retitle",
      label: "Retitle…",
      command: "oxplow.work_item.update",
      input: { uri: "{{row.uri}}", title: "{{row.title}}" },
      row: true,
      group: null,
      form: true,
      fixed: ["uri"],
    },
  ],
  alert: null,
  origin: { extension: "", source: null, running: "", deployment: null, bundled: false },
} as unknown as Lens;
const run: LensRun = {
  lens,
  params: {},
  result: { columns: ["uri", "title"], rows: [["oxplow:oxplow/work_item?id=tsk1&provider=oxplow", "Old"]], truncated: false },
} as unknown as LensRun;

test("a form action opens its form in a slideover and runs what the person filled in", async () => {
  actionCommand = UPDATE_WIDE;
  const view = render(<LensResultView run={run} onOpenPage={() => {}} />);
  fireEvent.contextMenu(view.getByTestId("lens-row-0"));
  fireEvent.click(view.getByTestId("menu-item-lens-action-retitle"));
  // It opened a form instead of running: a slideover, its form being two fields.
  const slideover = await waitFor(() => view.getByTestId("lens-action-form"));
  expect(slideover.getAttribute("role")).toBe("dialog");
  expect(runs).toEqual([]);
  // The fixed uri is shown, not an input; the title starts from the row.
  expect(view.getByTestId("lens-action-form-fields-uri-fixed").textContent).toContain("oxplow:oxplow/work_item?id=tsk1&provider=oxplow");
  const title = view.getByTestId("lens-action-form-fields-title") as HTMLInputElement;
  expect(title.value).toBe("Old");
  fireEvent.change(title, { target: { value: "New" } });
  fireEvent.submit(view.getByTestId("lens-action-form-fields"));
  await waitFor(() => expect(runs).toHaveLength(1));
  expect(runs[0].input).toEqual({ uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: "New" });
  expect(runs[0].row).toEqual({ uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: "Old" });
  // Success closes it.
  await waitFor(() => expect(view.queryByTestId("lens-action-form")).toBeNull());
});

test("an invalid field's message shows under it, and the form stays open", async () => {
  invalidField = "/title";
  actionCommand = UPDATE_WIDE;
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

/** Press the row's Retitle… and wait for whatever form shape opens. */
async function pressRetitle(view: ReturnType<typeof render>) {
  fireEvent.contextMenu(view.getByTestId("lens-row-0"));
  fireEvent.click(view.getByTestId("menu-item-lens-action-retitle"));
  return waitFor(() => view.getByTestId("lens-action-form"));
}

// A one-field form is never a slideover: the Project panel's Rename… is a
// strip at the top of the panel, and a lens's Rename… is the same.
test("a form action whose form is one text field opens a prompt strip, not a slideover", async () => {
  const view = render(<LensResultView run={run} onOpenPage={() => {}} />);
  const strip = await pressRetitle(view);
  expect(strip.getAttribute("data-shape")).toBe("prompt");
  expect(view.queryByRole("dialog")).toBeNull();
  expect(runs).toEqual([]);
  // The field's description is the strip's message; the field starts
  // from the row, focused and selected.
  expect(strip.textContent).toContain("What to call it");
  const title = view.getByTestId("lens-action-form-fields-title") as HTMLInputElement;
  expect(title.value).toBe("Old");
  expect(document.activeElement).toBe(title);
  expect(title.selectionStart).toBe(0);
  expect(title.selectionEnd).toBe(3);
  expect(view.getByTestId("lens-action-form-fields-submit").textContent).toBe("Retitle");
  fireEvent.change(title, { target: { value: "New" } });
  fireEvent.submit(title.form!);
  await waitFor(() => expect(runs).toHaveLength(1));
  expect(runs[0].input).toEqual({ title: "New" });
  expect(runs[0].row).toEqual({ uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: "Old" });
  await waitFor(() => expect(view.queryByTestId("lens-action-form")).toBeNull());
});

test("Escape closes the strip and runs nothing", async () => {
  const view = render(<LensResultView run={run} onOpenPage={() => {}} />);
  await pressRetitle(view);
  fireEvent.keyDown(view.getByTestId("lens-action-form-fields-title"), { key: "Escape" });
  await waitFor(() => expect(view.queryByTestId("lens-action-form")).toBeNull());
  expect(runs).toEqual([]);
});

test("a refusal naming the field shows under the strip, which stays open", async () => {
  invalidField = "/title";
  const view = render(<LensResultView run={run} onOpenPage={() => {}} />);
  await pressRetitle(view);
  const title = view.getByTestId("lens-action-form-fields-title") as HTMLInputElement;
  fireEvent.change(title, { target: { value: "Other" } });
  fireEvent.submit(title.form!);
  const error = await waitFor(() => view.getByTestId("lens-action-form-fields-error"));
  expect(error.textContent).toContain("can't be empty");
  expect(view.getByTestId("lens-action-form").getAttribute("data-shape")).toBe("prompt");
  expect((view.getByTestId("lens-action-form-fields-title") as HTMLInputElement).value).toBe("Other");
});

test("a form whose one field is a choice still opens the slideover", async () => {
  actionCommand = DECIDE;
  const view = render(<LensResultView run={run} onOpenPage={() => {}} />);
  const form = await pressRetitle(view);
  expect(form.getAttribute("role")).toBe("dialog");
  expect((view.getByTestId("lens-action-form-fields-title") as HTMLSelectElement).tagName).toBe("SELECT");
});

test("a form lens shows only its fields, keeps its fixed ones, and puts errors under the field", async () => {
  invalidField = "/title";
  const form: LensRun = {
    lens: { ...lens, viz: "form", form: { command: "oxplow.work_item.update", defaults: null, fixed: ["uri"], fields: ["title"] }, actions: [] },
    params: {},
    result: { columns: [], rows: [], truncated: false },
  } as unknown as LensRun;
  const view = render(<LensResultView run={form} onOpenPage={() => {}} />);
  const title = await waitFor(() => view.getByTestId("lens-form-fields-title") as HTMLInputElement);
  expect(view.queryByTestId("lens-form-fields-ref")).toBeNull();
  fireEvent.change(title, { target: { value: "" } });
  fireEvent.submit(view.getByTestId("lens-form-fields"));
  // An emptied text field is left out; the fixed ref is always sent.
  await waitFor(() => expect(submits).toEqual([{ uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow" }]));
  const error = await waitFor(() => view.getByTestId("lens-form-fields-title-error"));
  expect(error.textContent).toContain("can't be empty");
});

// An edit form shows what it saved: after a successful submit it starts
// again from the models, not from the values it first opened with.
test("a form lens starts again from the current values after it saves", async () => {
  const form: LensRun = {
    lens: { ...lens, viz: "form", form: { command: "oxplow.work_item.update", defaults: null, fixed: ["uri"], fields: ["title"] }, actions: [] },
    params: {},
    result: { columns: [], rows: [], truncated: false },
  } as unknown as LensRun;
  const view = render(<LensResultView run={form} onOpenPage={() => {}} />);
  const title = await waitFor(() => view.getByTestId("lens-form-fields-title") as HTMLInputElement);
  fireEvent.change(title, { target: { value: "New" } });
  fireEvent.submit(view.getByTestId("lens-form-fields"));
  await waitFor(() => expect(submits).toEqual([{ uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: "New" }]));
  await waitFor(() => expect(formStarts).toBe(2));
  await waitFor(() => expect((view.getByTestId("lens-form-fields-title") as HTMLInputElement).value).toBe("New"));
});

// A page run re-read after a save (the server re-ran it, or the window
// did) brings the saved row: the form starts again from it, even when the
// first start after the save raced the re-run.
test("a form starts again when its run's rows change", async () => {
  const at = (title: string): LensRun =>
    ({
      lens: { ...lens, viz: "form", form: { command: "oxplow.work_item.update", defaults: null, fixed: ["uri"], fields: ["title"] }, actions: [] },
      params: {},
      result: { columns: ["title"], rows: [[title]], truncated: false },
    }) as unknown as LensRun;
  const view = render(<LensResultView run={at("Old")} onOpenPage={() => {}} />);
  await waitFor(() => expect(view.getByTestId("lens-form-fields-title")).toBeTruthy());
  const before = formStarts;
  storedTitle = "Saved elsewhere";
  view.rerender(<LensResultView run={at("Saved elsewhere")} onOpenPage={() => {}} />);
  await waitFor(() => expect(formStarts).toBe(before + 1));
  await waitFor(() => expect((view.getByTestId("lens-form-fields-title") as HTMLInputElement).value).toBe("Saved elsewhere"));
});

function formLens(then: ActionThen): LensRun {
  return {
    lens: { ...lens, viz: "form", form: { command: "oxplow.work_item.update", defaults: null, fixed: ["uri"], fields: ["title"], then }, actions: [] },
    params: {},
    result: { columns: [], rows: [], truncated: false },
  } as unknown as LensRun;
}

/** Submit a form lens on a page; what it asked the page to open. */
async function submitOnPage(then: ActionThen): Promise<Array<[string, unknown]>> {
  openAfterPage = "oxplow:oxplow/work_item?id=tsk1&provider=oxplow";
  const opened: Array<[string, unknown]> = [];
  const nav = { openRef: (ref: string, how: unknown) => opened.push([ref, how]) } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <LensResultView run={formLens(then)} onOpenPage={() => {}} />
    </PageNavigationContext.Provider>,
  );
  const title = await waitFor(() => view.getByTestId("lens-form-fields-title") as HTMLInputElement);
  fireEvent.change(title, { target: { value: "New" } });
  fireEvent.submit(view.getByTestId("lens-form-fields"));
  await waitFor(() => expect(submits).toHaveLength(1));
  await waitFor(() => expect(formStarts).toBe(2));
  return opened;
}

test("a form lens goes where its command says after a submit", async () => {
  expect(await submitOnPage("open")).toEqual([["oxplow:oxplow/work_item?id=tsk1&provider=oxplow", { replace: false }]]);
});

test("`then: replace` sends the form's page out of history", async () => {
  expect(await submitOnPage("replace")).toEqual([["oxplow:oxplow/work_item?id=tsk1&provider=oxplow", { replace: true }]]);
});

test("`then: stay` keeps the form for the next entry", async () => {
  expect(await submitOnPage("stay")).toEqual([]);
});

/** A form lens with a second submit that keeps `keep`'s values. */
function againLens(keep: string[]): LensRun {
  return {
    lens: {
      ...lens,
      viz: "form",
      form: { command: "oxplow.work_item.update", defaults: null, fixed: ["uri"], fields: ["title"], then: "open", again: { label: "Save and Another", keep } },
      actions: [],
    },
    params: {},
    result: { columns: [], rows: [], truncated: false },
  } as unknown as LensRun;
}

// The second submit runs the command, stays on the page (whatever `then`
// says), and starts the form again with the `keep` fields' values carried
// over and the rest from the form's own start.
test("a form's second submit starts again with the kept fields carried over", async () => {
  fixedStart = { uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: "Start" };
  openAfterPage = "oxplow:oxplow/work_item?id=tsk1&provider=oxplow";
  const opened: unknown[] = [];
  const nav = { openRef: (ref: string) => opened.push(ref) } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <LensResultView run={againLens(["title"])} onOpenPage={() => {}} />
    </PageNavigationContext.Provider>,
  );
  const title = await waitFor(() => view.getByTestId("lens-form-fields-title") as HTMLInputElement);
  fireEvent.change(title, { target: { value: "One" } });
  fireEvent.click(view.getByTestId("lens-form-fields-again"));
  await waitFor(() => expect(submits).toEqual([{ uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: "One" }]));
  await waitFor(() => expect(formStarts).toBe(2));
  await waitFor(() => expect((view.getByTestId("lens-form-fields-title") as HTMLInputElement).value).toBe("One"));
  expect(opened).toEqual([]);
});

test("a field not kept starts again from the form's own start", async () => {
  fixedStart = { uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: "Start" };
  const view = render(<LensResultView run={againLens([])} onOpenPage={() => {}} />);
  const title = await waitFor(() => view.getByTestId("lens-form-fields-title") as HTMLInputElement);
  fireEvent.change(title, { target: { value: "One" } });
  fireEvent.click(view.getByTestId("lens-form-fields-again"));
  await waitFor(() => expect(formStarts).toBe(2));
  await waitFor(() => expect((view.getByTestId("lens-form-fields-title") as HTMLInputElement).value).toBe("Start"));
});

/** A tab whose history state the form keeps its draft in: `state` is
 *  what it holds, kept across the tab's renders. */
function Tab({ state, children }: { state: Record<string, unknown>; children: ReactNode }) {
  const [held, setHeld] = useState(state);
  const nav = {
    historyState: held,
    setHistoryState: (patch: Record<string, unknown>) => {
      Object.assign(state, patch);
      setHeld({ ...state });
    },
  } as unknown as PageNavigation;
  return <PageNavigationContext.Provider value={nav}>{children}</PageNavigationContext.Provider>;
}

// What the person typed is the tab's (history state): Back, Forward and a
// restart give it back, and an edit form saves only when changed.
test("an edit form keeps its draft in the tab and saves only when changed", async () => {
  const state: Record<string, unknown> = {};
  const edit: LensRun = {
    lens: { ...lens, viz: "form", form: { command: "oxplow.work_item.update", defaults: null, fixed: ["uri"], fields: ["title"], mode: "edit" }, actions: [] },
    params: {},
    result: { columns: [], rows: [], truncated: false },
  } as unknown as LensRun;
  const view = render(
    <Tab state={state}>
      <LensResultView run={edit} onOpenPage={() => {}} />
    </Tab>,
  );
  const title = await waitFor(() => view.getByTestId("lens-form-fields-title") as HTMLInputElement);
  expect((view.getByTestId("lens-form-fields-submit") as HTMLButtonElement).disabled).toBe(true);
  fireEvent.change(title, { target: { value: "Half" } });
  expect(state["form:x/l"]).toEqual({ uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: "Half" });
  cleanup();

  // The page again, from its kept step: the draft is there.
  const again = render(
    <Tab state={state}>
      <LensResultView run={edit} onOpenPage={() => {}} />
    </Tab>,
  );
  const kept = await waitFor(() => again.getByTestId("lens-form-fields-title") as HTMLInputElement);
  await waitFor(() => expect(kept.value).toBe("Half"));
  fireEvent.click(again.getByTestId("lens-form-fields-discard"));
  expect(kept.value).toBe("Old");
  expect(state["form:x/l"]).toBe(null);
  fireEvent.change(kept, { target: { value: "New" } });
  fireEvent.submit(again.getByTestId("lens-form-fields"));
  await waitFor(() => expect(submits).toEqual([{ uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: "New" }]));
  await waitFor(() => expect(state["form:x/l"]).toBe(null));
});
