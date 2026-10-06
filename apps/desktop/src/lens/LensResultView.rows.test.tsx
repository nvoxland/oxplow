import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Lens, LensLink, LensRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { CONTEXT_REF_MIME } from "../dragMimes.js";

// Lens rows can be grouped under headings (with actions there),
// styled from their own columns, and dragged into the agent's context.

const realApi = await import("../api.js");
const actionRuns: { action: string; row: unknown }[] = [];
mock.module("../api.js", () => ({
  ...realApi,
  runLensAction: async (_id: string, action: string, _params: unknown, row: unknown) => {
    actionRuns.push({ action, row });
    return { result: null, auditId: 1, eventId: null, inverse: null };
  },
  listExtensions: async () => [],
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} }),
}));
const { LensResultView } = await import("./LensResultView.js");

afterEach(cleanup);

const link = (kind: LensLink["kind"], from: string | null = null): LensLink => ({ kind, from, line: null, base: null, head: null });

const base: Lens = {
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
  timeline: null,
  steps: null,
  hunks: null,
  form: null,
  custom: null,
  group: null,
  emphasis: null,
  depth: null,
  children: [],
  launcherCategory: null,
  hidden: false,
  actions: [],
  alert: null,
  path: "",
};
const columns = ["bucket", "bucket_ref", "id", "title", "status", "hue", "on", "d"];
const rows: SqlCell[][] = [
  ["In progress", "page:tasks", 1, "Epic", "in_progress", null, 0, 0],
  ["In progress", "page:tasks", 2, "Child", "done", "warning", 1, 1],
  ["Ready", null, 3, "Next", "ready", null, 0, 0],
];
const runOf = (lens: Partial<Lens>): LensRun =>
  ({ lens: { ...base, ...lens }, params: {}, result: { columns, rows, truncated: false } }) as unknown as LensRun;
const grouped: Partial<Lens> = {
  group: { by: "bucket", link: link("page", "bucket_ref") },
  emphasis: "on",
  depth: "d",
  columns: [{ key: "title", label: null, link: link("task", "id"), unit: null, icon: "status", tone: "hue" }],
  actions: [{ id: "add", label: "+", command: "work_item.create", input: {}, row: false, group: "Ready" }],
};

test("a grouped list puts its rows under a heading per value, in order, and the heading links", () => {
  const view = render(<LensResultView run={runOf(grouped)} compact onOpenPage={() => {}} />);
  const headings = view.getAllByTestId("lens-group-heading");
  expect(headings.map((h) => h.getAttribute("data-group"))).toEqual(["In progress", "Ready"]);
  expect(headings[0]!.querySelector("button")?.textContent).toBe("In progress");
  // "Ready" has no ref: plain text, not a link.
  expect(headings[1]!.querySelector("[data-testid=lens-group-link]")).toBeNull();
  // The group column isn't a cell.
  expect(view.getByTestId("lens-row-0").textContent).not.toContain("In progress");
  expect(view.getByTestId("lens-row-2").textContent).toContain("Next");
});

test("a group's action is a button in its heading, even compact, and runs without a row", async () => {
  actionRuns.length = 0;
  const view = render(<LensResultView run={runOf(grouped)} compact onOpenPage={() => {}} />);
  expect(view.queryByTestId("lens-actions")).toBeNull();
  const button = view.getByTestId("lens-group-action-add");
  expect(button.closest("[data-group]")?.getAttribute("data-group")).toBe("Ready");
  fireEvent.click(button);
  await waitFor(() => expect(actionRuns).toEqual([{ action: "add", row: null }]));
});

test("a group action stays out of the toolbar", () => {
  const view = render(<LensResultView run={runOf(grouped)} onOpenPage={() => {}} />);
  expect(view.getByTestId("lens-actions").querySelector("[data-testid=lens-action-add]")).toBeNull();
  expect(view.getAllByTestId("lens-group-action-add")).toHaveLength(1);
});

test("cells draw their icon and tone; rows their emphasis and depth", () => {
  const view = render(<LensResultView run={runOf(grouped)} compact onOpenPage={() => {}} />);
  const child = view.getByTestId("lens-row-1");
  expect(child.querySelector("[data-testid=lens-icon]")?.getAttribute("aria-label")).toBe("Done");
  expect(child.querySelector("[data-tone=warning]")).not.toBeNull();
  expect(child.getAttribute("data-emphasis")).toBe("true");
  expect(view.getByTestId("lens-row-0").getAttribute("data-emphasis")).toBeNull();
  expect(child.style.paddingLeft).not.toBe(view.getByTestId("lens-row-0").style.paddingLeft);
});

test("a grouped table puts heading rows between its rows", () => {
  const view = render(<LensResultView run={runOf({ ...grouped, viz: "table" })} onOpenPage={() => {}} />);
  const table = view.getByTestId("lens-table");
  expect([...table.querySelectorAll("[data-testid=lens-group-heading]")].map((h) => h.getAttribute("data-group"))).toEqual([
    "In progress",
    "Ready",
  ]);
  expect([...table.querySelectorAll("th")].map((th) => th.textContent)).toContain("title");
  expect([...table.querySelectorAll("thead th")].map((th) => th.textContent)).not.toContain("bucket");
});

function dragData(el: HTMLElement): Record<string, string> {
  const data: Record<string, string> = {};
  const dataTransfer = { setData: (k: string, v: string) => (data[k] = v), effectAllowed: "" };
  fireEvent.dragStart(el, { dataTransfer });
  return data;
}

test("a row that links somewhere drags into the agent's context; one that doesn't isn't draggable", () => {
  const view = render(<LensResultView run={runOf(grouped)} compact onOpenPage={() => {}} />);
  const row = view.getByTestId("lens-row-2");
  expect(row.getAttribute("draggable")).toBe("true");
  expect(JSON.parse(dragData(row)[CONTEXT_REF_MIME]!)).toEqual({ kind: "ref", ref: "work_item:oxplow:tsk3" });
  cleanup();
  const plain = render(<LensResultView run={runOf({})} onOpenPage={() => {}} />);
  expect(plain.getByTestId("lens-row-0").getAttribute("draggable")).toBeNull();
});

test("a tree's labels follow their column's link, and its rows drag", () => {
  // `d` names the parent: Child (d = 1) nests under Epic (id 1).
  const run = runOf({
    viz: "tree",
    tree: { id: "id", parent: "d", label: "title" },
    emphasis: "on",
    columns: [{ key: "title", label: null, link: link("task", "id"), unit: null, icon: "status", tone: null }],
  });
  const view = render(<LensResultView run={run} onOpenPage={() => {}} />);
  const nodes = view.getAllByTestId("lens-tree-row");
  // Its first button is the branch toggle; the label is the link.
  expect(nodes[0]!.querySelector("button:not([aria-expanded])")?.textContent).toContain("Epic");
  expect(nodes[0]!.querySelector("[data-testid=lens-icon]")).not.toBeNull();
  expect(JSON.parse(dragData(nodes[0]!)[CONTEXT_REF_MIME]!)).toEqual({ kind: "ref", ref: "work_item:oxplow:tsk1" });
  expect(nodes.some((n) => n.getAttribute("data-emphasis") === "true")).toBe(true);
});
