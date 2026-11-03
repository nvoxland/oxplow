import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render as rtlRender, waitFor } from "@testing-library/react";
import { useState, type ReactElement, type ReactNode } from "react";
import type { ActionThen, CommandSpec, VizConfig, VizRun } from "../tauri-bridge/generated/bindings.js";
import { IpcCallError } from "../ipc-error.js";
import { VizActionRunnerContext, type VizActionRunner } from "./vizActionRunner.js";
import { FormSourceContext, type FormSource } from "./formSource.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";

// A viz action with `form: true` opens its command's form in a slideover,
// its fields starting from the action's bound input and its fixed fields
// shown but not editable; submitting runs it with what the person filled
// in, an invalid field's message lands under that field, and success
// closes it. A form that is one text field is a prompt strip at the top of
// the viz instead. A form viz shows the same per-field errors, and after
// a submit goes where its command says, as its `then` says.

const realApi = await import("../api.js");
const runs: Array<{ action: string; row: unknown; input: unknown; confirmed: boolean }> = [];
let invalidField: string | null = null;
const submits: unknown[] = [];
// What the item's title is in the datasets: a form viz starts from it.
let storedTitle = "Old";
let formStarts = 0;
// When set, a form viz always starts from these values (not what was stored).
let fixedStart: Record<string, unknown> | null = null;
// The page the command names after a run (`ui.open_after`), if any.
let openAfterPage: string | null = null;
// What else an action's form starts with (`fields`, `choices`, other values).
let actionExtras: Record<string, unknown> = {};

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
    ...actionExtras,
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
const press: VizActionRunner = async (_id, action, _params, row, _streamId, confirmed, input) => {
  runs.push({ action, row, input, confirmed });
  if (invalidField) throw new IpcCallError("a title can't be empty", "INVALID", invalidField);
  return { result: null, audit_id: 1, event_id: null, inverse: null };
};
const wrapper = ({ children }: { children: ReactNode }) => (
  <VizActionRunnerContext.Provider value={press}>
    <FormSourceContext.Provider value={formSource}>{children}</FormSourceContext.Provider>
  </VizActionRunnerContext.Provider>
);
const render = (ui: ReactElement) => rtlRender(ui, { wrapper });

const { VizView } = await import("./VizView.js");

beforeEach(() => {
  runs.length = 0;
  submits.length = 0;
  invalidField = null;
  storedTitle = "Old";
  formStarts = 0;
  fixedStart = null;
  openAfterPage = null;
  actionCommand = UPDATE;
  actionExtras = {};
});
afterEach(cleanup);

const viz: VizConfig = {
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
} as unknown as VizConfig;
const run: VizRun = {
  config: viz,
  params: {},
  result: { columns: ["uri", "title"], rows: [["oxplow:oxplow/work_item?id=tsk1&provider=oxplow", "Old"]], truncated: false },
} as unknown as VizRun;

test("a form action opens its form in a slideover and runs what the person filled in", async () => {
  actionCommand = UPDATE_WIDE;
  const view = render(<VizView run={run} onOpenPage={() => {}} />);
  fireEvent.contextMenu(view.getByTestId("viz-row-0"));
  fireEvent.click(view.getByTestId("menu-item-viz-action-retitle"));
  // It opened a form instead of running: a slideover, its form being two fields.
  const slideover = await waitFor(() => view.getByTestId("viz-action-form"));
  expect(slideover.getAttribute("role")).toBe("dialog");
  expect(runs).toEqual([]);
  // The fixed uri is the host's: not drawn, not even as text; the title starts from the row.
  expect(view.queryByTestId("viz-action-form-fields-uri-fixed")).toBeNull();
  expect(slideover.textContent).not.toContain("oxplow:oxplow/work_item?id=tsk1");
  const title = view.getByTestId("viz-action-form-fields-title") as HTMLInputElement;
  expect(title.value).toBe("Old");
  fireEvent.change(title, { target: { value: "New" } });
  fireEvent.submit(view.getByTestId("viz-action-form-fields"));
  await waitFor(() => expect(runs).toHaveLength(1));
  expect(runs[0].input).toEqual({ uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: "New" });
  expect(runs[0].row).toEqual({ uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: "Old" });
  // Success closes it.
  await waitFor(() => expect(view.queryByTestId("viz-action-form")).toBeNull());
});

test("an invalid field's message shows under it, and the form stays open", async () => {
  invalidField = "/title";
  actionCommand = UPDATE_WIDE;
  const view = render(<VizView run={run} onOpenPage={() => {}} />);
  fireEvent.contextMenu(view.getByTestId("viz-row-0"));
  fireEvent.click(view.getByTestId("menu-item-viz-action-retitle"));
  await waitFor(() => view.getByTestId("viz-action-form-fields-title"));
  fireEvent.change(view.getByTestId("viz-action-form-fields-title"), { target: { value: "" } });
  fireEvent.submit(view.getByTestId("viz-action-form-fields"));
  const error = await waitFor(() => view.getByTestId("viz-action-form-fields-title-error"));
  expect(error.textContent).toContain("can't be empty");
  expect(view.getByTestId("viz-action-form")).toBeTruthy();
});

/** Press the row's Retitle… and wait for whatever form shape opens. */
async function pressRetitle(view: ReturnType<typeof render>) {
  fireEvent.contextMenu(view.getByTestId("viz-row-0"));
  fireEvent.click(view.getByTestId("menu-item-viz-action-retitle"));
  return waitFor(() => view.getByTestId("viz-action-form"));
}

// A one-field form is never a slideover: the Project panel's Rename… is a
// strip at the top of the panel, and a viz's Rename… is the same.
test("a form action whose form is one text field opens a prompt strip, not a slideover", async () => {
  const view = render(<VizView run={run} onOpenPage={() => {}} />);
  const strip = await pressRetitle(view);
  expect(strip.getAttribute("data-shape")).toBe("prompt");
  expect(view.queryByRole("dialog")).toBeNull();
  expect(runs).toEqual([]);
  // The strip's message is the field's label (its help, when the page gives
  // one), never the schema's agent-facing description; the field starts
  // from the row, focused and selected.
  expect(strip.textContent).toContain("Title");
  expect(strip.textContent).not.toContain("What to call it");
  const title = view.getByTestId("viz-action-form-fields-title") as HTMLInputElement;
  expect(title.value).toBe("Old");
  expect(document.activeElement).toBe(title);
  expect(title.selectionStart).toBe(0);
  expect(title.selectionEnd).toBe(3);
  expect(view.getByTestId("viz-action-form-fields-submit").textContent).toBe("Retitle");
  fireEvent.change(title, { target: { value: "New" } });
  fireEvent.submit(title.form!);
  await waitFor(() => expect(runs).toHaveLength(1));
  expect(runs[0].input).toEqual({ title: "New" });
  expect(runs[0].row).toEqual({ uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: "Old" });
  await waitFor(() => expect(view.queryByTestId("viz-action-form")).toBeNull());
});

test("Escape closes the strip and runs nothing", async () => {
  const view = render(<VizView run={run} onOpenPage={() => {}} />);
  await pressRetitle(view);
  fireEvent.keyDown(view.getByTestId("viz-action-form-fields-title"), { key: "Escape" });
  await waitFor(() => expect(view.queryByTestId("viz-action-form")).toBeNull());
  expect(runs).toEqual([]);
});

test("a refusal naming the field shows under the strip, which stays open", async () => {
  invalidField = "/title";
  const view = render(<VizView run={run} onOpenPage={() => {}} />);
  await pressRetitle(view);
  const title = view.getByTestId("viz-action-form-fields-title") as HTMLInputElement;
  fireEvent.change(title, { target: { value: "Other" } });
  fireEvent.submit(title.form!);
  const error = await waitFor(() => view.getByTestId("viz-action-form-fields-error"));
  expect(error.textContent).toContain("can't be empty");
  expect(view.getByTestId("viz-action-form").getAttribute("data-shape")).toBe("prompt");
  expect((view.getByTestId("viz-action-form-fields-title") as HTMLInputElement).value).toBe("Other");
});

test("a form whose one field is a choice still opens the slideover", async () => {
  actionCommand = DECIDE;
  const view = render(<VizView run={run} onOpenPage={() => {}} />);
  const form = await pressRetitle(view);
  expect(form.getAttribute("role")).toBe("dialog");
  expect((view.getByTestId("viz-action-form-fields-title") as HTMLSelectElement).tagName).toBe("SELECT");
});

test("a form viz over a loader with no rows draws its empty line instead of its fields", () => {
  const form: VizRun = {
    config: { ...viz, viz: "form", empty: "Nothing to commit.", form: { command: "oxplow.work_item.update", defaults: null, fixed: ["uri"], fields: ["title"] }, actions: [] },
    params: {},
    result: { columns: [], rows: [], truncated: false },
  } as unknown as VizRun;
  const view = render(<VizView run={form} onOpenPage={() => {}} />);
  expect(view.getByTestId("viz-empty").textContent).toBe("Nothing to commit.");
  expect(view.queryByTestId("viz-form-fields")).toBeNull();
});

test("a form viz shows only its fields, keeps its fixed ones, and puts errors under the field", async () => {
  invalidField = "/title";
  const form: VizRun = {
    config: { ...viz, viz: "form", form: { command: "oxplow.work_item.update", defaults: null, fixed: ["uri"], fields: ["title"] }, actions: [] },
    params: {},
    result: { columns: [], rows: [], truncated: false },
  } as unknown as VizRun;
  const view = render(<VizView run={form} onOpenPage={() => {}} />);
  const title = await waitFor(() => view.getByTestId("viz-form-fields-title") as HTMLInputElement);
  expect(view.queryByTestId("viz-form-fields-ref")).toBeNull();
  fireEvent.change(title, { target: { value: "" } });
  fireEvent.submit(view.getByTestId("viz-form-fields"));
  // An emptied text field is left out; the fixed ref is always sent.
  await waitFor(() => expect(submits).toEqual([{ uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow" }]));
  const error = await waitFor(() => view.getByTestId("viz-form-fields-title-error"));
  expect(error.textContent).toContain("can't be empty");
});

// An edit form shows what it saved: after a successful submit it starts
// again from the datasets, not from the values it first opened with.
test("a form viz starts again from the current values after it saves", async () => {
  const form: VizRun = {
    config: { ...viz, viz: "form", form: { command: "oxplow.work_item.update", defaults: null, fixed: ["uri"], fields: ["title"] }, actions: [] },
    params: {},
    result: { columns: [], rows: [], truncated: false },
  } as unknown as VizRun;
  const view = render(<VizView run={form} onOpenPage={() => {}} />);
  const title = await waitFor(() => view.getByTestId("viz-form-fields-title") as HTMLInputElement);
  fireEvent.change(title, { target: { value: "New" } });
  fireEvent.submit(view.getByTestId("viz-form-fields"));
  await waitFor(() => expect(submits).toEqual([{ uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: "New" }]));
  await waitFor(() => expect(formStarts).toBe(2));
  await waitFor(() => expect((view.getByTestId("viz-form-fields-title") as HTMLInputElement).value).toBe("New"));
});

// A page run re-read after a save (the server re-ran it, or the window
// did) brings the saved row: the form starts again from it, even when the
// first start after the save raced the re-run.
test("a form starts again when its run's rows change", async () => {
  const at = (title: string): VizRun =>
    ({
      config: { ...viz, viz: "form", form: { command: "oxplow.work_item.update", defaults: null, fixed: ["uri"], fields: ["title"] }, actions: [] },
      params: {},
      result: { columns: ["title"], rows: [[title]], truncated: false },
    }) as unknown as VizRun;
  const view = render(<VizView run={at("Old")} onOpenPage={() => {}} />);
  await waitFor(() => expect(view.getByTestId("viz-form-fields-title")).toBeTruthy());
  const before = formStarts;
  storedTitle = "Saved elsewhere";
  view.rerender(<VizView run={at("Saved elsewhere")} onOpenPage={() => {}} />);
  await waitFor(() => expect(formStarts).toBe(before + 1));
  await waitFor(() => expect((view.getByTestId("viz-form-fields-title") as HTMLInputElement).value).toBe("Saved elsewhere"));
});

function formViz(then: ActionThen): VizRun {
  return {
    config: { ...viz, viz: "form", form: { command: "oxplow.work_item.update", defaults: null, fixed: ["uri"], fields: ["title"], then }, actions: [] },
    params: {},
    result: { columns: [], rows: [], truncated: false },
  } as unknown as VizRun;
}

/** Submit a form viz on a page; what it asked the page to open. */
async function submitOnPage(then: ActionThen): Promise<Array<[string, unknown]>> {
  openAfterPage = "oxplow:oxplow/work_item?id=tsk1&provider=oxplow";
  const opened: Array<[string, unknown]> = [];
  const nav = { openRef: (ref: string, how: unknown) => opened.push([ref, how]) } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <VizView run={formViz(then)} onOpenPage={() => {}} />
    </PageNavigationContext.Provider>,
  );
  const title = await waitFor(() => view.getByTestId("viz-form-fields-title") as HTMLInputElement);
  fireEvent.change(title, { target: { value: "New" } });
  fireEvent.submit(view.getByTestId("viz-form-fields"));
  await waitFor(() => expect(submits).toHaveLength(1));
  await waitFor(() => expect(formStarts).toBe(2));
  return opened;
}

test("a form viz goes where its command says after a submit", async () => {
  expect(await submitOnPage("open")).toEqual([["oxplow:oxplow/work_item?id=tsk1&provider=oxplow", { replace: false }]]);
});

test("`then: replace` sends the form's page out of history", async () => {
  expect(await submitOnPage("replace")).toEqual([["oxplow:oxplow/work_item?id=tsk1&provider=oxplow", { replace: true }]]);
});

test("`then: stay` keeps the form for the next entry", async () => {
  expect(await submitOnPage("stay")).toEqual([]);
});

/** A form viz with a second submit that keeps `keep`'s values. */
function againViz(keep: string[]): VizRun {
  return {
    config: {
      ...viz,
      viz: "form",
      form: { command: "oxplow.work_item.update", defaults: null, fixed: ["uri"], fields: ["title"], then: "open", again: { label: "Save and Another", keep } },
      actions: [],
    },
    params: {},
    result: { columns: [], rows: [], truncated: false },
  } as unknown as VizRun;
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
      <VizView run={againViz(["title"])} onOpenPage={() => {}} />
    </PageNavigationContext.Provider>,
  );
  const title = await waitFor(() => view.getByTestId("viz-form-fields-title") as HTMLInputElement);
  fireEvent.change(title, { target: { value: "One" } });
  fireEvent.click(view.getByTestId("viz-form-fields-again"));
  await waitFor(() => expect(submits).toEqual([{ uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: "One" }]));
  await waitFor(() => expect(formStarts).toBe(2));
  await waitFor(() => expect((view.getByTestId("viz-form-fields-title") as HTMLInputElement).value).toBe("One"));
  expect(opened).toEqual([]);
});

test("a field not kept starts again from the form's own start", async () => {
  fixedStart = { uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: "Start" };
  const view = render(<VizView run={againViz([])} onOpenPage={() => {}} />);
  const title = await waitFor(() => view.getByTestId("viz-form-fields-title") as HTMLInputElement);
  fireEvent.change(title, { target: { value: "One" } });
  fireEvent.click(view.getByTestId("viz-form-fields-again"));
  await waitFor(() => expect(formStarts).toBe(2));
  await waitFor(() => expect((view.getByTestId("viz-form-fields-title") as HTMLInputElement).value).toBe("Start"));
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
  const edit: VizRun = {
    config: { ...viz, viz: "form", form: { command: "oxplow.work_item.update", defaults: null, fixed: ["uri"], fields: ["title"], mode: "edit" }, actions: [] },
    params: {},
    result: { columns: [], rows: [], truncated: false },
  } as unknown as VizRun;
  const view = render(
    <Tab state={state}>
      <VizView run={edit} onOpenPage={() => {}} />
    </Tab>,
  );
  const title = await waitFor(() => view.getByTestId("viz-form-fields-title") as HTMLInputElement);
  expect((view.getByTestId("viz-form-fields-submit") as HTMLButtonElement).disabled).toBe(true);
  fireEvent.change(title, { target: { value: "Half" } });
  expect(state["form:x/l"]).toEqual({ uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: "Half" });
  cleanup();

  // The page again, from its kept step: the draft is there.
  const again = render(
    <Tab state={state}>
      <VizView run={edit} onOpenPage={() => {}} />
    </Tab>,
  );
  const kept = await waitFor(() => again.getByTestId("viz-form-fields-title") as HTMLInputElement);
  await waitFor(() => expect(kept.value).toBe("Half"));
  fireEvent.click(again.getByTestId("viz-form-fields-discard"));
  expect(kept.value).toBe("Old");
  expect(state["form:x/l"]).toBe(null);
  fireEvent.change(kept, { target: { value: "New" } });
  fireEvent.submit(again.getByTestId("viz-form-fields"));
  await waitFor(() => expect(submits).toEqual([{ uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: "New" }]));
  await waitFor(() => expect(state["form:x/l"]).toBe(null));
});

// A form for a person: the heading and the button say the action without
// the menu's ellipsis, the host's fixed values are not drawn, the fields
// read as the page says (else the humanized name, never the agent-facing
// schema description), the choices show their labels, and the submit is a
// primary button beside a Cancel.
const AGENT: CommandSpec = {
  id: "oxplow.work_item.update",
  input_schema: {
    type: "object",
    required: ["uri", "title"],
    properties: {
      uri: { type: "string", description: "The item's ref." },
      title: { type: "string", description: "Agent-facing words about the title." },
      native_rename: { type: "boolean", description: "Only where the harness declares it." },
      conversation: { type: "string", description: "The saved conversation to fork (its id)." },
    },
  },
} as unknown as CommandSpec;

test("the form's heading and submit say the action without its ellipsis, in a primary button beside Cancel", async () => {
  actionCommand = AGENT;
  const view = render(<VizView run={run} onOpenPage={() => {}} />);
  await pressRetitle(view);
  expect(view.getByTestId("viz-action-form-title").textContent).toBe("Retitle");
  const submit = view.getByTestId("viz-action-form-fields-submit");
  expect(submit.textContent).toBe("Retitle");
  expect(submit.className).toContain("ox-primary");
  expect(view.getByTestId("viz-action-form-fields-cancel")).toBeTruthy();
});

test("fields read as the page says, else the humanized name with no schema description", async () => {
  actionCommand = AGENT;
  actionExtras = { fields: [{ name: "title", label: "Name", help: "What the tab shows." }, "native_rename"] };
  const view = render(<VizView run={run} onOpenPage={() => {}} />);
  const form = await pressRetitle(view);
  expect(view.getByText("Name")).toBeTruthy();
  expect(view.getByText("What the tab shows.")).toBeTruthy();
  expect(view.getByText("Native rename")).toBeTruthy();
  expect(form.textContent).not.toContain("Agent-facing");
  expect(form.textContent).not.toContain("harness declares");
});

test("a choice shows its labels, submits its value, and starts on the action's own value", async () => {
  actionCommand = AGENT;
  actionExtras = {
    values: { uri: "u", title: "Old", conversation: "c2" },
    fields: ["title", "conversation"],
    choices: { conversation: [{ value: "c1", label: "Fix the build", group: null }, { value: "c2", label: "Add forms — this session's", group: null }] },
  };
  const view = render(<VizView run={run} onOpenPage={() => {}} />);
  await pressRetitle(view);
  const select = view.getByTestId("viz-action-form-fields-conversation") as HTMLSelectElement;
  expect(select.value).toBe("c2");
  expect(Array.from(select.options).map((o) => o.textContent)).toContain("Fix the build");
  fireEvent.change(select, { target: { value: "c1" } });
  fireEvent.submit(view.getByTestId("viz-action-form-fields"));
  await waitFor(() => expect(runs).toHaveLength(1));
  expect(runs[0].input).toMatchObject({ conversation: "c1", title: "Old" });
});

test("Cancel and Escape close the form and run nothing", async () => {
  actionCommand = AGENT;
  actionExtras = { fields: ["title", "native_rename"] };
  const view = render(<VizView run={run} onOpenPage={() => {}} />);
  await pressRetitle(view);
  fireEvent.click(view.getByTestId("viz-action-form-fields-cancel"));
  await waitFor(() => expect(view.queryByTestId("viz-action-form")).toBeNull());
  await pressRetitle(view);
  fireEvent.keyDown(view.getByTestId("viz-action-form-fields-title"), { key: "Escape" });
  await waitFor(() => expect(view.queryByTestId("viz-action-form")).toBeNull());
  expect(runs).toEqual([]);
});

test("the submit is off while a required field is empty, and its hover says which", async () => {
  actionCommand = AGENT;
  actionExtras = { values: { uri: "u" }, fields: [{ name: "title", label: "Name" }, "native_rename"] };
  const view = render(<VizView run={run} onOpenPage={() => {}} />);
  await pressRetitle(view);
  const submit = view.getByTestId("viz-action-form-fields-submit") as HTMLButtonElement;
  expect(submit.disabled).toBe(true);
  expect(submit.title).toBe("Fill in Name first");
});

test("a page action's form names what it is about under its heading, from the page's own label", async () => {
  actionCommand = AGENT;
  actionExtras = { fields: ["title", "native_rename"] };
  const { ActionForm } = await import("./VizView.js");
  const action = viz.actions[0]!;
  const view = render(
    <ActionForm run={run as never} streamId={null} pressed={{ action, row: null }} subject="Fix the build" onClose={() => {}} />,
  );
  const subtitle = await waitFor(() => view.getByTestId("viz-action-form-subtitle"));
  expect(subtitle.textContent).toBe("Fix the build");
});

test("a row action's form names its row under the heading by the action's `about` column", async () => {
  actionCommand = AGENT;
  actionExtras = { fields: ["title", "native_rename"] };
  const { ActionForm } = await import("./VizView.js");
  const action = { ...viz.actions[0]!, about: "title" };
  const view = render(
    <ActionForm run={run as never} streamId={null} pressed={{ action, row: { uri: "u", title: "Fix the build" } }} subject="The page" onClose={() => {}} />,
  );
  const subtitle = await waitFor(() => view.getByTestId("viz-action-form-subtitle"));
  expect(subtitle.textContent).toBe("Fix the build");
});

test("a row action without `about` has no subtitle, whatever page it is on", async () => {
  actionCommand = AGENT;
  actionExtras = { fields: ["title", "native_rename"] };
  const { ActionForm } = await import("./VizView.js");
  const view = render(
    <ActionForm run={run as never} streamId={null} pressed={{ action: viz.actions[0]!, row: { uri: "u", title: "T" } }} subject="The page" onClose={() => {}} />,
  );
  await waitFor(() => view.getByTestId("viz-action-form"));
  expect(view.queryByTestId("viz-action-form-subtitle")).toBeNull();
});
