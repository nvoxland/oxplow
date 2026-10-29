import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render as rtlRender, waitFor } from "@testing-library/react";

import type { ReactElement, ReactNode } from "react";
import type { VizConfig, ColumnLink, VizRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { VizActionRunnerContext, type VizActionRunner } from "./vizActionRunner.js";
import { CONTEXT_REF_MIME } from "../dragMimes.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";

// A viz node's rows can be grouped under headings (with actions there),
// styled from their own columns, and dragged into the agent's context.

const realApi = await import("../api.js");
const actionRuns: { action: string; row: unknown }[] = [];
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [],
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }),
}));
const { VizView } = await import("./VizView.js");

// A viz presses through the page that draws it: a runner stands in for it.
const press: VizActionRunner = async (_id, action, _params, row) => {
  actionRuns.push({ action, row });
  return { result: null, audit_id: 1, event_id: null, inverse: null };
};
const wrapper = ({ children }: { children: ReactNode }) => <VizActionRunnerContext.Provider value={press}>{children}</VizActionRunnerContext.Provider>;
const render = (ui: ReactElement) => rtlRender(ui, { wrapper });

afterEach(cleanup);

const link = (kind: ColumnLink["kind"], from: string | null = null): ColumnLink => ({ kind, from, line: null, base: null, head: null });

const base: VizConfig = {
  id: "x/work",
  extension: "x",
  slug: "work",
  title: "Work",
  description: "",
  query: "",
  viz: "list",
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
const columns = ["bucket", "bucket_ref", "id", "title", "status", "hue", "on", "d", "uri"];
const rows: SqlCell[][] = [
  ["In progress", "oxplow:oxplow/page?page=tasks", 1, "Epic", "in_progress", null, 0, 0, "oxplow:oxplow/work_item?id=tsk1&provider=oxplow"],
  ["In progress", "oxplow:oxplow/page?page=tasks", 2, "Child", "done", "warning", 1, 1, "oxplow:oxplow/work_item?id=tsk2&provider=oxplow"],
  ["Ready", null, 3, "Next", "todo", null, 0, 0, "oxplow:oxplow/work_item?id=tsk3&provider=oxplow"],
];
const runOf = (viz: Partial<VizConfig>): VizRun =>
  ({ config: { ...base, ...viz }, params: {}, result: { columns, rows, truncated: false } }) as unknown as VizRun;
const grouped: Partial<VizConfig> = {
  group: { by: "bucket", link: link("page", "bucket_ref") },
  emphasis: "on",
  depth: "d",
  columns: [{ key: "title", label: null, link: link("page", "uri"), unit: null, icon: "status", tone: "hue" }],
  actions: [{ id: "add", label: "+", command: "oxplow.work_item.create", input: {}, row: false, group: "Ready" }],
};

test("a grouped list puts its rows under a heading per value, in order, and the heading has an arrow to its page", () => {
  const view = render(<VizView run={runOf(grouped)} compact onOpenPage={() => {}} />);
  const headings = view.getAllByTestId("viz-group-heading");
  expect(headings.map((h) => h.getAttribute("data-group"))).toEqual(["In progress", "Ready"]);
  // The heading text is plain; a group with a ref has the arrow after it.
  expect(headings[0]!.querySelector("[data-testid=open-page-link]")?.getAttribute("title")).toBe("Open In progress");
  expect(headings[0]!.querySelector("button[data-testid=open-page-link]")?.textContent).toBe("");
  expect(headings[0]!.textContent).toContain("In progress");
  // "Ready" has no ref: no arrow.
  expect(headings[1]!.querySelector("[data-testid=open-page-link]")).toBeNull();
  // The group column isn't a cell.
  expect(view.getByTestId("viz-row-0").textContent).not.toContain("In progress");
  expect(view.getByTestId("viz-row-2").textContent).toContain("Next");
});

test("a group's action is a button in its heading, even compact, and runs without a row", async () => {
  actionRuns.length = 0;
  const view = render(<VizView run={runOf(grouped)} compact onOpenPage={() => {}} />);
  expect(view.queryByTestId("viz-actions")).toBeNull();
  const button = view.getByTestId("viz-group-action-add");
  expect(button.closest("[data-group]")?.getAttribute("data-group")).toBe("Ready");
  fireEvent.click(button);
  await waitFor(() => expect(actionRuns).toEqual([{ action: "add", row: null }]));
});

// A group's heading says how many rows it holds and folds them away.
test("a group's heading shows its count and folds its rows", () => {
  const view = render(<VizView run={runOf(grouped)} compact onOpenPage={() => {}} />);
  const headings = view.getAllByTestId("viz-group-heading");
  expect(headings.map((h) => h.querySelector("[data-testid=viz-group-count]")?.textContent)).toEqual(["2", "1"]);
  const toggle = headings[0]!.querySelector("[aria-expanded]") as HTMLElement;
  expect(toggle.getAttribute("aria-expanded")).toBe("true");
  fireEvent.click(toggle);
  expect(view.queryByTestId("viz-row-0")).toBeNull();
  expect(view.queryByTestId("viz-row-1")).toBeNull();
  // The other group's rows stay, and a folded heading says so.
  expect(view.getByTestId("viz-row-2")).toBeTruthy();
  expect((view.getAllByTestId("viz-group-heading")[0]!.querySelector("[aria-expanded]") as HTMLElement).getAttribute("aria-expanded")).toBe("false");
  fireEvent.click(view.getAllByTestId("viz-group-heading")[0]!.querySelector("[aria-expanded]") as HTMLElement);
  expect(view.getByTestId("viz-row-0")).toBeTruthy();
});

// A cell's format can draw a mark: a series as a small line with its
// latest value, a short value as a pill in the column's tone.
test("sparkline and badge formats draw a line and a pill", () => {
  const run = {
    config: {
      ...base,
      viz: "table",
      columns: [
        { key: "title", label: null, link: null, unit: null, icon: null, tone: null },
        { key: "trend", label: null, link: null, unit: null, icon: null, tone: null, format: "sparkline" },
        { key: "level", label: null, link: null, unit: null, icon: null, tone: "hue", format: "badge" },
      ],
    },
    params: {},
    result: { columns: ["title", "trend", "level", "hue"], rows: [["a", "[1,3,2,4]", "high", "danger"], ["b", "n/a", "low", null]], truncated: false },
  } as unknown as VizRun;
  const view = render(<VizView run={run} onOpenPage={() => {}} />);
  const first = view.getByTestId("viz-row-0");
  expect(first.querySelector("[data-testid=viz-sparkline] svg")).not.toBeNull();
  expect(first.querySelector("[data-testid=viz-sparkline]")!.textContent).toContain("4");
  expect(first.querySelector("[data-testid=viz-badge]")!.textContent).toBe("high");
  // A cell that isn't a series stays plain text.
  const second = view.getByTestId("viz-row-1");
  expect(second.querySelector("[data-testid=viz-sparkline]")).toBeNull();
  expect(second.textContent).toContain("n/a");
  expect(second.querySelector("[data-testid=viz-badge]")!.textContent).toBe("low");
});

// A line chart's targets column draws a target line on its series.
test("a line's targets column draws each series' target line", () => {
  const run = {
    config: { ...base, viz: "line", chart: { x: "at", y: "v", series: null, label: null, size: null, group: null, targets: "goal" } },
    params: {},
    result: { columns: ["at", "v", "goal"], rows: [[1, 1, 5], [2, 3, 5], [3, 2, 5]], truncated: false },
  } as unknown as VizRun;
  const view = render(<VizView run={run} onOpenPage={() => {}} />);
  expect(view.getByTestId("viz-line").querySelectorAll("[data-testid=trend-target]")).toHaveLength(1);
  cleanup();
  const none = render(<VizView run={{ ...run, config: { ...run.config, chart: { ...run.config.chart!, targets: null } } }} onOpenPage={() => {}} />);
  expect(none.getByTestId("viz-line").querySelector("[data-testid=trend-target]")).toBeNull();
});

test("a group action stays out of the whole-viz buttons", () => {
  const view = render(<VizView run={runOf(grouped)} partButtons onOpenPage={() => {}} />);
  expect(view.queryByTestId("viz-action-add")).toBeNull();
  expect(view.getAllByTestId("viz-group-action-add")).toHaveLength(1);
});

test("cells draw their icon and tone; rows their emphasis and depth", () => {
  const view = render(<VizView run={runOf(grouped)} compact onOpenPage={() => {}} />);
  const child = view.getByTestId("viz-row-1");
  expect(child.querySelector("[data-testid=viz-icon]")?.getAttribute("aria-label")).toBe("Done");
  expect(child.querySelector("[data-tone=warning]")).not.toBeNull();
  expect(child.getAttribute("data-emphasis")).toBe("true");
  expect(view.getByTestId("viz-row-0").getAttribute("data-emphasis")).toBeNull();
  expect(child.style.paddingLeft).not.toBe(view.getByTestId("viz-row-0").style.paddingLeft);
});

test("a grouped table puts heading rows between its rows", () => {
  const view = render(<VizView run={runOf({ ...grouped, viz: "table" })} onOpenPage={() => {}} />);
  const table = view.getByTestId("viz-table");
  expect([...table.querySelectorAll("[data-testid=viz-group-heading]")].map((h) => h.getAttribute("data-group"))).toEqual([
    "In progress",
    "Ready",
  ]);
  expect([...table.querySelectorAll("th")].map((th) => th.textContent)).toContain("Title");
  expect([...table.querySelectorAll("thead th")].map((th) => th.textContent)).not.toContain("bucket");
});

function dragData(el: HTMLElement): Record<string, string> {
  const data: Record<string, string> = {};
  const dataTransfer = { setData: (k: string, v: string) => (data[k] = v), effectAllowed: "" };
  fireEvent.dragStart(el, { dataTransfer });
  return data;
}

test("a row that links somewhere drags into the agent's context; one that doesn't isn't draggable", () => {
  const view = render(<VizView run={runOf(grouped)} compact onOpenPage={() => {}} />);
  const row = view.getByTestId("viz-row-2");
  expect(row.getAttribute("draggable")).toBe("true");
  expect(JSON.parse(dragData(row)[CONTEXT_REF_MIME]!)).toEqual({ kind: "ref", ref: "oxplow:oxplow/work_item?id=tsk3&provider=oxplow" });
  cleanup();
  const plain = render(<VizView run={runOf({})} onOpenPage={() => {}} />);
  expect(plain.getByTestId("viz-row-0").getAttribute("draggable")).toBeNull();
});

test("a tree's labels follow their column's link, and its rows drag", () => {
  // `d` names the parent: Child (d = 1) nests under Epic (id 1).
  const run = runOf({
    viz: "tree",
    tree: { id: "id", parent: "d", label: "title" },
    emphasis: "on",
    columns: [{ key: "title", label: null, link: link("page", "uri"), unit: null, icon: "status", tone: null }],
  });
  const view = render(<VizView run={run} onOpenPage={() => {}} />);
  const nodes = view.getAllByTestId("viz-tree-row");
  // Its first button is the branch toggle; the label is the link.
  expect(nodes[0]!.querySelector("button:not([aria-expanded])")?.textContent).toContain("Epic");
  expect(nodes[0]!.querySelector("[data-testid=viz-icon]")).not.toBeNull();
  expect(JSON.parse(dragData(nodes[0]!)[CONTEXT_REF_MIME]!)).toEqual({ kind: "ref", ref: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow" });
  expect(nodes.some((n) => n.getAttribute("data-emphasis") === "true")).toBe(true);
});

// A tree's row picks like a table's: a click sets the param its column
// selects, the picked row is marked, and a link in it still opens.
test("a tree's row click sets the param its column selects and marks the picked row", () => {
  const picked: [string, SqlCell][] = [];
  const run = {
    ...runOf({
      viz: "tree",
      tree: { id: "id", parent: "d", label: "title" },
      params: [{ name: "pick", label: null, default: null, options: [], type: "text", role: "selection", presets: [] }],
      columns: [{ key: "title", label: null, link: null, unit: null, icon: null, tone: null, select: "pick" }] as never,
    }),
    params: { pick: "Child" },
  } as VizRun;
  const view = render(<VizView run={run} onOpenPage={() => {}} onSetParam={(n, v) => picked.push([n, v])} />);
  const nodes = view.getAllByTestId("viz-tree-row");
  fireEvent.click(nodes[0]!);
  expect(picked).toEqual([["pick", "Epic"]]);
  expect(nodes[1]!.getAttribute("aria-selected")).toBe("true");
  expect(nodes[1]!.className).toContain("ox-selected");
  expect(nodes[0]!.getAttribute("aria-selected")).toBe("false");
  expect(nodes[0]!.className).toContain("ox-clickable");
  fireEvent.click(view.getAllByRole("button", { name: "Collapse" })[0]!);
  expect(picked).toHaveLength(1);
});

// A cell or heading that links to a ref opens it where its kind's page
// says (`openRef`), whatever the kind: a handoff, a note, an extension's.
test("a link to a ref of any kind opens through the page's openRef", () => {
  const opened: Array<[string, boolean | undefined]> = [];
  const nav = {
    navigate: () => {},
    openRef: (ref: string, how?: { newTab?: boolean }) => opened.push([ref, how?.newTab]),
  } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <VizView run={runOf(grouped)} compact />
    </PageNavigationContext.Provider>,
  );
  fireEvent.click(view.getByTestId("viz-row-2").querySelector("button")!);
  fireEvent.click(view.getAllByTestId("open-page-link")[0]!, { metaKey: true });
  expect(opened).toEqual([
    ["oxplow:oxplow/work_item?id=tsk3&provider=oxplow", false],
    ["oxplow:oxplow/page?page=tasks", true],
  ]);
});

test("a highlighted column marks its param's value in the cell, as elements", () => {
  const run = {
    ...runOf({ columns: [{ key: "title", label: null, link: null, unit: null, highlight: "filter" }], params: [{ name: "filter", label: null, default: null, options: [] }] }),
    params: { filter: "ild" },
  } as unknown as VizRun;
  const view = render(<VizView run={run} compact onOpenPage={() => {}} />);
  const marks = view.getByTestId("viz-row-1").querySelectorAll("mark.ox-mark");
  expect(Array.from(marks).map((m) => m.textContent)).toEqual(["ild"]);
  expect(view.getByTestId("viz-row-1").textContent).toContain("Child");
  // No value: plain text.
  cleanup();
  const plain = render(<VizView run={{ ...run, params: { filter: "" } } as unknown as VizRun} compact onOpenPage={() => {}} />);
  expect(plain.container.querySelector("mark")).toBeNull();
});

/** A column's `select` makes a row a pick: clicking it (or Enter) sets
 *  that param to the row's value; a link inside it still only navigates,
 *  and the row holding the current value is drawn selected. */
test("a row click sets the param its column selects; a link in it doesn't", () => {
  const picked: [string, SqlCell][] = [];
  const run = {
    ...runOf({
      viz: "table",
      params: [{ name: "pick", label: null, default: null, options: [], type: "text", role: "selection", presets: [] }],
      columns: [
        { key: "title", label: null, link: null, unit: null, icon: null, tone: null, select: "pick" },
        { key: "uri", label: null, link: link("page", "uri"), unit: null, icon: null, tone: null },
      ] as never,
    }),
    params: { pick: "Child" },
  } as VizRun;
  const view = render(<VizView run={run} onOpenPage={() => {}} onSetParam={(n, v) => picked.push([n, v])} />);
  fireEvent.click(view.getByTestId("viz-row-0"));
  fireEvent.keyDown(view.getByTestId("viz-row-2"), { key: "Enter" });
  const linkCell = view.getByTestId("viz-row-0").querySelectorAll("td")[1]!.firstElementChild as HTMLElement;
  fireEvent.click(linkCell);
  expect(picked).toEqual([
    ["pick", "Epic"],
    ["pick", "Next"],
  ]);
  expect(view.getByTestId("viz-row-1").getAttribute("aria-selected")).toBe("true");
  expect(view.getByTestId("viz-row-0").getAttribute("aria-selected")).toBe("false");
  cleanup();
  // With nowhere to set it, a row is no pick.
  const plain = render(<VizView run={run} onOpenPage={() => {}} />);
  expect(plain.getByTestId("viz-row-0").getAttribute("aria-selected")).toBeNull();
});

// The rail shows what was picked: the row as its columns label it.
test("a pick passes the row it came from, labelled as its columns are", () => {
  const rowsPicked: unknown[] = [];
  const run = {
    ...runOf({
      viz: "table",
      params: [{ name: "pick", label: null, default: null, options: [], type: "text", role: "selection", presets: [] }],
      columns: [
        { key: "title", label: null, link: null, unit: null, icon: null, tone: null, select: "pick" },
        { key: "status", label: "State", link: null, unit: null, icon: null, tone: null },
      ] as never,
    }),
  } as VizRun;
  const view = render(<VizView run={run} onOpenPage={() => {}} onSetParam={(_n, _v, row) => rowsPicked.push(row)} />);
  fireEvent.click(view.getByTestId("viz-row-1"));
  expect(rowsPicked).toEqual([
    {
      value: "Child",
      facts: [
        { label: "Title", text: "Child" },
        { label: "State", text: "done" },
      ],
    },
  ]);
});

// A column with no label reads as words, not as its key.
test("a column with no label is headed by its key in words", () => {
  const view = render(<VizView run={runOf({ ...grouped, viz: "table" })} onOpenPage={() => {}} />);
  expect([...view.getByTestId("viz-table").querySelectorAll("thead th")].map((th) => th.textContent)).toContain("Title");
});

// A table's links are one collection: the page a row opens has Up and Down
// through the rows' pages (`Origin`).
test("a link opens with its column's pages as the origin", () => {
  const opened: Array<[string, unknown]> = [];
  const nav = {
    navigate: () => {},
    openRef: (ref: string, how?: { origin?: unknown }) => opened.push([ref, how?.origin]),
  } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <VizView
        run={runOf({ viz: "table", columns: [{ key: "title", label: null, link: link("page", "uri"), unit: null }] })}
      />
    </PageNavigationContext.Provider>,
  );
  fireEvent.click(view.getByText("Child"));
  const [ref, origin] = opened[0]!;
  expect(ref).toBe("oxplow:oxplow/work_item?id=tsk2&provider=oxplow");
  // The collection is the viz's link column, run again for its list.
  expect(origin).toMatchObject({ items: [], live: { node: "x/work", column: "title" } });
});

// A viz table is keyboard-driven as every list is (`useListNav`): ↑/↓ move
// the cursor, Enter opens the row's link, or runs its first row action when
// it has none.
test("↑/↓ and Enter open the row under the cursor", () => {
  const opened: Array<[string, unknown]> = [];
  const nav = {
    navigate: () => {},
    openRef: (ref: string, how?: { origin?: unknown }) => opened.push([ref, how?.origin ? "origin" : "none"]),
  } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <VizView
        run={runOf({ viz: "table", columns: [{ key: "title", label: null, link: link("page", "uri"), unit: null }] })}
      />
    </PageNavigationContext.Provider>,
  );
  const table = view.getByTestId("viz-table");
  fireEvent.keyDown(table, { key: "ArrowDown" });
  fireEvent.keyDown(table, { key: "ArrowDown" });
  fireEvent.keyDown(table, { key: "ArrowUp" });
  fireEvent.keyDown(table, { key: "Enter" });
  expect(opened).toEqual([["oxplow:oxplow/work_item?id=tsk2&provider=oxplow", "origin"]]);
  expect(view.getByTestId("viz-row-1").getAttribute("data-cursor")).toBe("true");
});

test("Enter runs the first row action of a row with no link", async () => {
  actionRuns.length = 0;
  const view = render(
    <VizView
      run={runOf({
        viz: "table",
        columns: [{ key: "title", label: null, link: null, unit: null }],
        actions: [{ id: "open", label: "Open", command: "oxplow.work_item.update", input: {}, row: true }],
      })}
      onOpenPage={() => {}}
    />,
  );
  const table = view.getByTestId("viz-table");
  fireEvent.keyDown(table, { key: "ArrowDown" });
  fireEvent.keyDown(table, { key: "Enter" });
  await waitFor(() => expect(actionRuns.map((r) => r.action)).toEqual(["open"]));
});

// A row action marked `primary` is what a click on the row runs (and Enter,
// before a link): the pointer's way to take a row.
test("a click on a row runs the primary action, but not from a link in it", async () => {
  actionRuns.length = 0;
  const view = render(
    <VizView
      run={runOf({
        viz: "table",
        columns: [{ key: "title", label: null, link: null, unit: null }],
        actions: [
          { id: "other", label: "Other", command: "oxplow.work_item.update", input: {}, row: true },
          { id: "take", label: "Take", command: "oxplow.work_item.update", input: {}, row: true, primary: true },
        ],
      })}
      onOpenPage={() => {}}
    />,
  );
  fireEvent.click(view.getByTestId("viz-row-1"));
  await waitFor(() => expect(actionRuns.map((r) => r.action)).toEqual(["take"]));
  actionRuns.length = 0;
  const table = view.getByTestId("viz-table");
  fireEvent.keyDown(table, { key: "Enter" });
  await waitFor(() => expect(actionRuns.map((r) => r.action)).toEqual(["take"]));
});
