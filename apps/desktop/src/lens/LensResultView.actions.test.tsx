import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import type { Lens, LensRun } from "../tauri-bridge/generated/bindings.js";
import { IpcCallError } from "../ipc-error.js";
import { personSpec, recordRefOffers } from "../components/refCommandsTestSupport.js";

const realApi = await import("../api.js");
const realQuerySql = realApi.querySql;
// Bun's `mock.module` is process-wide: everything not faked here delegates
// to the real module so other test files keep working.
const calls: boolean[] = [];
const commandRuns: Array<[string, unknown]> = [];
mock.module("../api.js", () => ({
  ...realApi,
  runLensAction: async (
    _id: string,
    _action: string,
    _params: unknown,
    _row: unknown,
    _streamId: unknown,
    confirmed: boolean,
  ) => {
    calls.push(confirmed);
    if (!confirmed) throw new IpcCallError("Finish needs confirmation", "NEEDS_CONFIRMATION");
    return { result: null, auditId: 1, eventId: null, inverse: null };
  },
  getCommand: async () => Promise.reject(new Error("no bridge in tests")),
  listPersonCommands: async () => [
    personSpec("tracker.item.flag", { label: "Flag It", group: "Tracker", about: "work_item", input: { ref: "{{ref}}" } }),
  ],
  listExtensions: async () => [
    {
      name: "tracker",
      enabled: true,
      ui: {
        slots: [],
        decorators: [
          { id: "tracker/0", extension: "tracker", view: "v_tracker_flags", kind: "work_item", placement: "row-badge", label: "label", color: null },
        ],
      },
    },
  ],
  querySql: async (sql: string, ...rest: unknown[]) => {
    if (!sql.includes("FROM v_tracker_flags")) return (realQuerySql as (...a: unknown[]) => unknown)(sql, ...rest);
    return {
      columns: ["ref", "label"],
      rows: [["work_item:oxplow:tsk1", "flaky"]],
      truncated: false,
      reads: { models: ["v_tracker_flags"], tables: [], measures: [] },
      freshness: {},
    };
  },
  runCommand: async (name: string, input: unknown) => {
    commandRuns.push([name, input]);
    return { result: null, audit_id: 1, event_id: null, inverse: null };
  },
}));

const { LensResultView } = await import("./LensResultView.js");

afterEach(cleanup);

const lens: Lens = {
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
  timeline: null,
  steps: null,
  hunks: null,
  form: null,
  children: [],
  launcherCategory: null,
  hidden: false,
  actions: [{ id: "finish", label: "Finish", command: "oxplow.work_item.transition", input: {}, row: true }],
  alert: null,
  path: "",
};
const run: LensRun = {
  lens,
  params: {},
  result: { columns: ["ref"], rows: [["work_item:oxplow:tsk1"]], truncated: false },
};

// A row action that asks for confirmation must ask wherever the row is —
// an answer in the strip, a grid child, a dashboard tile — not only where
// the toolbar is shown.
test("a row action's confirmation shows when the toolbar is hidden", async () => {
  const { getByTestId, queryByTestId } = render(<LensResultView run={run} toolbar={false} onOpenPage={() => {}} />);
  fireEvent.contextMenu(getByTestId("lens-row-0"));
  fireEvent.click(getByTestId("menu-item-lens-action-finish"));
  await waitFor(() => expect(calls).toEqual([false]));
  await waitFor(() => expect(queryByTestId("lens-action-confirm")).not.toBeNull());
  fireEvent.click(getByTestId("lens-action-confirm-run"));
  await waitFor(() => expect(calls).toEqual([false, true]));
  await waitFor(() => expect(queryByTestId("lens-action-confirm") === null).toBe(true));
});

// Every row of every row component has the row menu (Ask About This and
// the lens's row actions), from the mouse and from the keyboard — Menu
// key or Shift+F10 on the focused row.
test("rows are focusable and open their menu from the keyboard, in every row component", () => {
  const { getByTestId, queryByTestId, unmount } = render(<LensResultView run={run} onOpenPage={() => {}} />);
  const row = getByTestId("lens-row-0");
  expect(row.tabIndex).toBe(0);
  fireEvent.keyDown(row, { key: "F10", shiftKey: true });
  expect(queryByTestId("menu-item-ask-about-row")).not.toBeNull();
  expect(queryByTestId("menu-item-lens-action-finish")).not.toBeNull();
  unmount();

  const steps: LensRun = {
    lens: { ...lens, viz: "steps", steps: { label: "ref", status: null } },
    params: {},
    result: run.result,
  };
  const view = render(<LensResultView run={steps} onOpenPage={() => {}} />);
  fireEvent.contextMenu(view.getByTestId("lens-step-0"));
  expect(view.queryByTestId("menu-item-lens-action-finish")).not.toBeNull();
});

// A row that links to a ref offers the commands about that kind of ref
// (their `ui.about`), after Ask About This and the lens's row actions.
test("a row's linked ref gets its kind's commands", async () => {
  recordRefOffers(commandRuns);
  const linked: LensRun = {
    lens: { ...lens, columns: [{ key: "ref", label: null, link: { kind: "page", from: null, line: null, base: null, head: null } }] },
    params: {},
    result: run.result,
  };
  const view = render(<LensResultView run={linked} onOpenPage={() => {}} />);
  await new Promise((r) => setTimeout(r, 20));
  fireEvent.contextMenu(view.getByTestId("lens-row-0"));
  fireEvent.click(await waitFor(() => view.getByTestId("menu-item-ref-commands-Tracker")));
  fireEvent.click(view.getByTestId("menu-item-ref-command-tracker.item.flag"));
  await waitFor(() => expect(commandRuns).toEqual([["tracker.item.flag", { ref: "work_item:oxplow:tsk1" }]]));
});

// P6b.C5: a decorator's label follows a cell that links to its ref.
test("a linked cell gets its ref's badges", async () => {
  const linked: LensRun = {
    lens: { ...lens, columns: [{ key: "ref", label: null, link: { kind: "page", from: null, line: null, base: null, head: null } }] },
    params: {},
    result: run.result,
  };
  const view = render(<LensResultView run={linked} onOpenPage={() => {}} />);
  await waitFor(() => expect(view.getByTestId("lens-row-0").textContent).toContain("flaky"));
});
