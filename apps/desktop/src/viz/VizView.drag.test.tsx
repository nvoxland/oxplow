import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, createEvent, fireEvent, render as rtlRender, waitFor } from "@testing-library/react";

import type { ReactElement, ReactNode } from "react";
import type { VizConfig, PageAction, VizRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { VIZ_ROW_DRAG_MIME } from "../dragMimes.js";
import { VizActionRunnerContext, type VizActionRunner } from "./vizActionRunner.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";

// A viz's rows drag to where a drag action runs, mark for a bulk action,
// and run a row action from its key; a board is columns of them.

const realApi = await import("../api.js");
const ran: { action: string; row: unknown; drop: unknown; confirmed: boolean }[] = [];
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [],
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }),
}));
const { VizView } = await import("./VizView.js");

// A viz presses through the page that draws it: a runner stands in for it.
const press: VizActionRunner = async (_id, action, _params, row, _streamId, confirmed, _input, drop) => {
  ran.push({ action, row, drop, confirmed });
  return { result: null, audit_id: 1, event_id: null, inverse: null };
};
const wrapper = ({ children }: { children: ReactNode }) => <VizActionRunnerContext.Provider value={press}>{children}</VizActionRunnerContext.Provider>;
const render = (ui: ReactElement) => rtlRender(ui, { wrapper });

afterEach(() => {
  cleanup();
  ran.length = 0;
});

const action = (over: Partial<PageAction>): PageAction =>
  ({ id: "x", label: "X", command: "oxplow.work_item.transition", input: {}, row: true, group: null, form: false, fixed: [], choices: {}, then: "open", when: null, primary: false, drag: false, bulk: false, key: null, ...over }) as PageAction;

const move = action({ id: "move", label: "Move", drag: true, input: { uri: "{{row.uri}}", to: "{{group.value}}" } });
const reorder = action({ id: "reorder", label: "Reorder", command: "oxplow.work_item.reorder", drag: true, input: { uri: "{{row.uri}}", before: "{{before.uri}}", after: "{{after.uri}}" } });
const finish = action({ id: "finish", label: "Finish", bulk: true, key: "f", input: { uri: "{{row.uri}}", to: "done" } });

const base = {
  id: "x/work",
  extension: "x",
  slug: "work",
  title: "Work",
  description: "",
  query: "",
  viz: "list",
  params: [],
  columns: [{ key: "title", label: null, link: null, unit: null, icon: null, tone: null }],
  empty: null,
  chart: null,
  tree: null,
  tile: null,
  timeline: null,
  steps: null,
  hunks: null,
  code: null,
  diff: null,
  markdown: null,
  form: null,
  custom: null,
  group: null,
  emphasis: null,
  depth: null,
  parts: [],
  launcherCategory: null,
  hidden: false,
  actions: [],
  alert: null,
  needs: [],
  origin: { extension: "", source: null, running: "", deployment: null, bundled: false },
};
const columns = ["uri", "title", "state"];
const rows: SqlCell[][] = [
  ["u1", "One", "todo"],
  ["u2", "Two", "todo"],
  ["u3", "Three", "done"],
];
const runOf = (viz: Partial<VizConfig>): VizRun =>
  ({ config: { ...base, ...viz }, params: {}, result: { columns, rows, truncated: false, reads: { datasets: [], tables: [], measures: [] } } }) as unknown as VizRun;
const record = (i: number) => ({ uri: rows[i]![0], title: rows[i]![1], state: rows[i]![2] });

/** A drag that carries what each `setData` set, for the drop to read. */
function drag(from: HTMLElement) {
  const data: Record<string, string> = {};
  const dataTransfer = {
    setData: (k: string, v: string) => (data[k] = v),
    getData: (k: string) => data[k] ?? "",
    get types() {
      return Object.keys(data);
    },
    effectAllowed: "",
    dropEffect: "",
  };
  fireEvent.dragStart(from, { dataTransfer });
  return {
    data,
    dropOn(to: HTMLElement, y = 0) {
      // The DOM here keeps no `clientY` on a drag event: set it.
      for (const make of [createEvent.dragOver, createEvent.drop]) {
        const ev = make(to, { dataTransfer });
        Object.defineProperty(ev, "clientY", { value: y });
        fireEvent(to, ev);
      }
    },
  };
}

/** `el` as a 30px-tall row at the top, so a drop's height picks its zone. */
function tall(el: HTMLElement): HTMLElement {
  el.getBoundingClientRect = () => ({ top: 0, height: 30, bottom: 30, left: 0, right: 100, width: 100, x: 0, y: 0, toJSON: () => ({}) });
  return el;
}

test("a row with a drag action drags; dropped beside another, the action runs with its neighbours, a missing one left out", async () => {
  const view = render(<VizView run={runOf({ actions: [reorder] })} onOpenPage={() => {}} />);
  const first = view.getByTestId("viz-row-0");
  expect(first.getAttribute("draggable")).toBe("true");
  const d = drag(view.getByTestId("viz-row-2"));
  expect(JSON.parse(d.data[VIZ_ROW_DRAG_MIME]!).keys).toHaveLength(1);
  // The top half of the first row: before it, nothing above.
  d.dropOn(tall(first), 5);
  await waitFor(() => expect(ran).toHaveLength(1));
  expect(ran[0]).toEqual({ action: "reorder", row: record(2), drop: { before: record(0), after: null, onto: null, group: null, order: [record(2), record(0), record(1)] }, confirmed: false });
  // The bottom half of the second row: after it, before the one below (the dragged row left out).
  ran.length = 0;
  drag(view.getByTestId("viz-row-0")).dropOn(tall(view.getByTestId("viz-row-1")), 25);
  await waitFor(() => expect(ran).toHaveLength(1));
  expect(ran[0]!.drop).toEqual({ before: record(2), after: record(1), onto: null, group: null, order: [record(1), record(0), record(2)] });
});

test("a drag action that binds the order gets every row in its new order", async () => {
  const order = action({ id: "order", label: "Reorder", command: "oxplow.dashboard.reorder_items", drag: true, input: { order: "{{order.uri}}" } });
  const view = render(<VizView run={runOf({ actions: [order] })} onOpenPage={() => {}} />);
  drag(view.getByTestId("viz-row-0")).dropOn(tall(view.getByTestId("viz-row-2")), 25);
  await waitFor(() => expect(ran).toHaveLength(1));
  expect(ran[0]).toMatchObject({ action: "order", drop: { order: [record(1), record(2), record(0)] } });
});

test("a row without a drag action isn't draggable, and a drop on it runs nothing", async () => {
  const view = render(<VizView run={runOf({ actions: [finish] })} onOpenPage={() => {}} />);
  expect(view.getByTestId("viz-row-0").getAttribute("draggable")).toBeNull();
});

test("a drop on a group's heading runs the action that binds the group, at its end", async () => {
  const view = render(<VizView run={runOf({ group: { by: "state", link: null, values: [] }, actions: [move] })} onOpenPage={() => {}} />);
  const headings = view.getAllByTestId("viz-group-heading");
  drag(view.getByTestId("viz-row-0")).dropOn(headings[1]!.parentElement!);
  await waitFor(() => expect(ran).toHaveLength(1));
  expect(ran[0]).toMatchObject({ action: "move", row: record(0), drop: { group: "done", after: record(2) } });
});

test("Cmd+click and Shift+click mark rows; the marks bar runs a bulk action once per row, in order; Clear clears", async () => {
  const view = render(<VizView run={runOf({ actions: [finish] })} onOpenPage={() => {}} />);
  fireEvent.click(view.getByTestId("viz-row-0"), { metaKey: true });
  fireEvent.click(view.getByTestId("viz-row-2"), { shiftKey: true });
  expect(["viz-row-0", "viz-row-1", "viz-row-2"].map((id) => view.getByTestId(id).getAttribute("data-marked"))).toEqual(["true", "true", "true"]);
  expect(view.getByTestId("viz-marks-count").textContent).toBe("3 selected");
  fireEvent.click(view.getByTestId("viz-bulk-finish"));
  await waitFor(() => expect(ran.map((r) => r.row)).toEqual([record(0), record(1), record(2)]));
  fireEvent.click(view.getByTestId("viz-marks-clear"));
  expect(view.queryByTestId("viz-marks")).toBeNull();
  expect(view.getByTestId("viz-row-0").getAttribute("data-marked")).toBeNull();
});

test("dragging a marked row carries every marked row, and they land in order", async () => {
  const view = render(<VizView run={runOf({ actions: [reorder] })} onOpenPage={() => {}} />);
  fireEvent.click(view.getByTestId("viz-row-1"), { metaKey: true });
  fireEvent.click(view.getByTestId("viz-row-2"), { metaKey: true });
  const d = drag(view.getByTestId("viz-row-1"));
  expect(JSON.parse(d.data[VIZ_ROW_DRAG_MIME]!).keys).toHaveLength(2);
  d.dropOn(tall(view.getByTestId("viz-row-0")), 5);
  await waitFor(() => expect(ran).toHaveLength(2));
  expect(ran.map((r) => [r.row, r.drop])).toEqual([
    [record(1), { before: record(0), after: null, onto: null, group: null, order: [record(1), record(2), record(0)] }],
    [record(2), { before: record(0), after: record(1), onto: null, group: null, order: [record(1), record(2), record(0)] }],
  ]);
});

test("a row action's key runs it on the cursor row; Space marks it, Escape clears the marks", async () => {
  const view = render(<VizView run={runOf({ actions: [finish] })} onOpenPage={() => {}} />);
  const list = view.getByTestId("viz-list");
  fireEvent.keyDown(list, { key: "ArrowDown" });
  fireEvent.keyDown(list, { key: "f" });
  await waitFor(() => expect(ran.map((r) => r.row)).toEqual([record(1)]));
  fireEvent.keyDown(list, { key: " " });
  expect(view.getByTestId("viz-row-1").getAttribute("data-marked")).toBe("true");
  fireEvent.keyDown(list, { key: "Escape" });
  expect(view.getByTestId("viz-row-1").getAttribute("data-marked")).toBeNull();
});

test("a board is a column per group, its values first and empty ones too; a drop on an empty column binds its value", async () => {
  const view = render(
    <VizView run={runOf({ viz: "board", group: { by: "state", link: null, values: ["todo", "doing", "done"] }, actions: [move] })} onOpenPage={() => {}} />,
  );
  const columnsShown = [...view.getByTestId("viz-board").querySelectorAll("section")].map((c) => c.getAttribute("data-testid"));
  expect(columnsShown).toEqual(["viz-board-column-todo", "viz-board-column-doing", "viz-board-column-done"]);
  expect(view.getByTestId("viz-board-column-todo").querySelectorAll(".ox-card")).toHaveLength(2);
  drag(view.getByTestId("viz-row-0")).dropOn(view.getByTestId("viz-board-column-doing"));
  await waitFor(() => expect(ran).toHaveLength(1));
  expect(ran[0]).toMatchObject({ action: "move", row: record(0), drop: { group: "doing", after: null } });
});

// A card's title is a link, and a link's own right-click opens its page in a
// new tab; on a board the right-click on a card is the card's menu.
test("a right-click on a board card's title link opens the card's menu, not its page", () => {
  const opened: string[] = [];
  const run = runOf({
    viz: "board",
    group: { by: "state", link: null, values: ["todo", "done"] },
    columns: [{ key: "title", label: null, link: { kind: "page", from: "uri", line: null, base: null, head: null }, unit: null, icon: null, tone: null }] as never,
    actions: [finish],
  });
  const withUris = {
    ...run,
    result: { ...run.result, rows: rows.map(([, title, state], i) => [`oxplow:oxplow/work_item?id=tsk${i + 1}&provider=oxplow`, title, state]) },
  } as VizRun;
  const nav = { navigate: () => {}, openRef: (ref: string) => opened.push(ref) } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <VizView run={withUris} onOpenPage={(r) => opened.push(r.id)} />
    </PageNavigationContext.Provider>,
  );
  const title = view.getByTestId("viz-row-0").querySelector("button, a") as HTMLElement;
  fireEvent.contextMenu(title);
  expect(view.getByTestId("menu-item-viz-action-finish")).toBeTruthy();
  expect(opened).toEqual([]);
});
