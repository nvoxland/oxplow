import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render as rtlRender, waitFor } from "@testing-library/react";
import type { ReactElement, ReactNode } from "react";
import type { VizConfig, VizRun } from "../tauri-bridge/generated/bindings.js";
import { IpcCallError } from "../ipc-error.js";
import { VizActionRunnerContext, type VizActionRunner } from "./vizActionRunner.js";
import { personSpec, recordRefOffers } from "../components/refCommandsTestSupport.js";

const realApi = await import("../api.js");
const pristineApi = { ...realApi };
const realQuerySql = realApi.querySql;
// Bun's `mock.module` is process-wide: everything not faked here delegates
// to the real module so other test files keep working.
const calls: boolean[] = [];
const commandRuns: Array<[string, unknown]> = [];
// What the daemon answers (`ref_commands`) for a task: the kind's
// commands, less those whose `about_when` fails on its row (`notNow`).
const notNow = new Set<string>();
mock.module("../api.js", () => ({
  ...realApi,
  refCommands: async (ref: string) =>
    ref.startsWith("oxplow:oxplow/task") ? ["tracker.item.flag", "tracker.item.reopen"].filter((id) => !notNow.has(id)) : [],
  getCommand: async () => Promise.reject(new Error("no bridge in tests")),
  listPersonCommands: async () => [
    personSpec("tracker.item.flag", { label: "Flag It", group: "Tracker", about: "oxplow/task", input: { uri: "{{uri}}" } }),
    personSpec("tracker.item.reopen", { label: "Reopen It", group: "Tracker", about: "oxplow/task", about_when: "closed", input: { uri: "{{uri}}" } }),
  ],
  listExtensions: async () => [
    {
      name: "tracker",
      enabled: true, pages: [],
      ui: {
        slots: [],
        decorators: [
          { id: "tracker/0", extension: "tracker", view: "v_tracker_flags", resource: "oxplow/task", placement: "row-badge", label: "label", color: null },
        ],
      },
    },
  ],
  querySql: async (sql: string, ...rest: unknown[]) => {
    if (!sql.includes("FROM v_tracker_flags")) return (realQuerySql as (...a: unknown[]) => unknown)(sql, ...rest);
    return {
      columns: ["uri", "label"],
      rows: [["oxplow:oxplow/task?id=tsk1&provider=oxtask", "flaky"]],
      truncated: false,
      reads: { datasets: ["v_tracker_flags"], tables: [], measures: [] },
      freshness: {},
    };
  },
  runCommand: async (name: string, input: unknown) => {
    commandRuns.push([name, input]);
    return { result: null, audit_id: 1, event_id: null, inverse: null };
  },
}));

const { VizView } = await import("./VizView.js");

// A viz presses through the page that draws it: a runner stands in for it.
const press: VizActionRunner = async (_id, _action, _params, _row, _streamId, confirmed) => {
  calls.push(confirmed);
  if (!confirmed) {
    throw new IpcCallError("Finish needs confirmation", "NEEDS_CONFIRMATION", null, [
      "process — runs programs with your authority; oxplow does not confine them",
    ]);
  }
  return { result: null, audit_id: 1, event_id: null, inverse: null };
};
const wrapper = ({ children }: { children: ReactNode }) => <VizActionRunnerContext.Provider value={press}>{children}</VizActionRunnerContext.Provider>;
const render = (ui: ReactElement) => rtlRender(ui, { wrapper });

afterEach(cleanup);

const viz: VizConfig = {
  id: "x/l",
  extension: "x",
  slug: "l",
  title: "L",
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
  actions: [{ id: "finish", label: "Finish", command: "oxplow.task.transition", input: {}, row: true }],
  alert: null,
  origin: { extension: "", source: null, running: "", deployment: null, bundled: false },
};
const run: VizRun = {
  config: viz,
  params: {},
  result: { columns: ["uri"], rows: [["oxplow:oxplow/task?id=tsk1&provider=oxtask"]], truncated: false },
};

// A row action that asks for confirmation must ask wherever the row is —
// an answer in the strip, a grid child, a dashboard tile — not only where
// the toolbar is shown.
test("a row action's confirmation shows wherever the row is", async () => {
  const { getByTestId, queryByTestId } = render(<VizView run={run} onOpenPage={() => {}} />);
  fireEvent.contextMenu(getByTestId("viz-row-0"));
  fireEvent.click(getByTestId("menu-item-viz-action-finish"));
  await waitFor(() => expect(calls).toEqual([false]));
  await waitFor(() => expect(queryByTestId("viz-action-confirm")).not.toBeNull());
  // What the command asks about beyond itself (a deploy's added
  // permissions) is in the confirmation.
  expect(getByTestId("viz-action-confirm-details").textContent).toContain("process — runs programs with your authority");
  fireEvent.click(getByTestId("viz-action-confirm-run"));
  await waitFor(() => expect(calls).toEqual([false, true]));
  await waitFor(() => expect(queryByTestId("viz-action-confirm") === null).toBe(true));
});

// Every row of every row component has the row menu (Ask About This and
// the viz's row actions), from the mouse and from the keyboard — Menu
// key or Shift+F10 on the focused row.
test("rows are focusable and open their menu from the keyboard, in every row component", () => {
  const { getByTestId, queryByTestId, unmount } = render(<VizView run={run} onOpenPage={() => {}} />);
  const row = getByTestId("viz-row-0");
  expect(row.tabIndex).toBe(0);
  fireEvent.keyDown(row, { key: "F10", shiftKey: true });
  expect(queryByTestId("menu-item-ask-about-row")).not.toBeNull();
  expect(queryByTestId("menu-item-viz-action-finish")).not.toBeNull();
  unmount();

  const steps: VizRun = {
    config: { ...viz, viz: "steps", steps: { label: "uri", status: null } },
    params: {},
    result: run.result,
  };
  const view = render(<VizView run={steps} onOpenPage={() => {}} />);
  fireEvent.contextMenu(view.getByTestId("viz-step-0"));
  expect(view.queryByTestId("menu-item-viz-action-finish")).not.toBeNull();
});

// A row that links to a ref offers the commands about that kind of ref
// (their `ui.about`), after Ask About This and the viz's row actions.
test("a row's linked ref gets its kind's commands", async () => {
  commandRuns.length = 0; // an earlier test's action reports its failure here
  recordRefOffers(commandRuns);
  const linked: VizRun = {
    config: { ...viz, columns: [{ key: "uri", label: null, link: { kind: "page", from: null, line: null, base: null, head: null } }] },
    params: {},
    result: run.result,
  };
  const view = render(<VizView run={linked} onOpenPage={() => {}} />);
  await new Promise((r) => setTimeout(r, 20));
  fireEvent.contextMenu(view.getByTestId("viz-row-0"));
  fireEvent.click(await waitFor(() => view.getByTestId("menu-item-ref-commands-Tracker")));
  fireEvent.click(view.getByTestId("menu-item-ref-command-tracker.item.flag"));
  await waitFor(() => expect(commandRuns).toEqual([["tracker.item.flag", { uri: "oxplow:oxplow/task?id=tsk1&provider=oxtask" }]]));
});

// A command whose `about_when` fails on the row's ref (the daemon leaves it
// out of its answer) is not in the row's menu at all.
test("a row's menu leaves out a command whose about_when fails on its ref", async () => {
  recordRefOffers(commandRuns);
  const linked: VizRun = {
    config: { ...viz, columns: [{ key: "uri", label: null, link: { kind: "page", from: null, line: null, base: null, head: null } }] },
    params: {},
    result: run.result,
  };
  const view = render(<VizView run={linked} onOpenPage={() => {}} />);
  await new Promise((r) => setTimeout(r, 20));
  fireEvent.contextMenu(view.getByTestId("viz-row-0"));
  fireEvent.click(await waitFor(() => view.getByTestId("menu-item-ref-commands-Tracker")));
  expect(view.queryByTestId("menu-item-ref-command-tracker.item.reopen")).not.toBeNull();
  cleanup();
  notNow.add("tracker.item.reopen");
  const again = render(<VizView run={linked} onOpenPage={() => {}} />);
  await new Promise((r) => setTimeout(r, 20));
  fireEvent.contextMenu(again.getByTestId("viz-row-0"));
  fireEvent.click(await waitFor(() => again.getByTestId("menu-item-ref-commands-Tracker")));
  expect(again.queryByTestId("menu-item-ref-command-tracker.item.flag")).not.toBeNull();
  expect(again.queryByTestId("menu-item-ref-command-tracker.item.reopen")).toBeNull();
  notNow.clear();
});

// P6b.C5: a decorator's label follows a cell that links to its ref.
test("a linked cell gets its ref's badges", async () => {
  const linked: VizRun = {
    config: { ...viz, columns: [{ key: "uri", label: null, link: { kind: "page", from: null, line: null, base: null, head: null } }] },
    params: {},
    result: run.result,
  };
  const view = render(<VizView run={linked} onOpenPage={() => {}} />);
  await waitFor(() => expect(view.getByTestId("viz-row-0").textContent).toContain("flaky"));
});

// `mock.module` is process-wide: put the real module back so the files that
// run after this one don't see this file's extensions.
afterAll(() => {
  mock.module("../api.js", () => pristineApi);
});

// An action with options is a submenu in the row's menu: one entry per
// option, the row's own (`current`) checked and not offered, and the one
// picked is what the press sends.
test("an action with options offers each option, the row's own checked", async () => {
  const options: Array<string | null | undefined> = [];
  const pressing: VizActionRunner = async (_id, _action, _params, _row, _streamId, _confirmed, _input, _drop, option) => {
    options.push(option);
    return { result: null, audit_id: 1, event_id: null, inverse: null };
  };
  const marked: VizRun = {
    ...run,
    config: {
      ...viz,
      actions: [{ id: "mark", label: "Mark", command: "oxplow.task.transition", input: {}, row: true, optionsFrom: "stages", current: "stage" }],
    },
    result: { columns: ["uri", "stage"], rows: [["oxplow:oxplow/task?id=tsk1&provider=oxtask", "ready"]], truncated: false },
  };
  const actionOptions = {
    mark: [
      { value: "ready", label: "Ready", group: null },
      { value: "done", label: "Done", group: null },
    ],
  };
  const { getByTestId } = rtlRender(
    <VizActionRunnerContext.Provider value={pressing}>
      <VizView run={marked} onOpenPage={() => {}} actionOptions={actionOptions} />
    </VizActionRunnerContext.Provider>,
  );
  fireEvent.contextMenu(getByTestId("viz-row-0"));
  fireEvent.click(getByTestId("menu-item-viz-action-mark"));
  const own = getByTestId("menu-item-viz-action-mark-ready") as HTMLButtonElement;
  expect(own.textContent).toContain("✓");
  expect(own.disabled).toBe(true);
  fireEvent.click(getByTestId("menu-item-viz-action-mark-done"));
  await waitFor(() => expect(options).toEqual(["done"]));
});
