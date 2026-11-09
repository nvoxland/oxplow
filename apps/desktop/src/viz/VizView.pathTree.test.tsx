import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render as rtlRender, within } from "@testing-library/react";

import type { ReactElement, ReactNode } from "react";
import type { VizConfig, VizRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { VizActionRunnerContext, type VizActionRunner } from "./vizActionRunner.js";

// A tree over paths (`tree.path`) draws its made folders, each name with
// its status marks at the row's end, a long name cut with its path on
// hover; compact, it shows a few visible rows and says how many more.

const realApi = { ...(await import("../api.js")) };
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [],
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }),
}));
const { VizView } = await import("./VizView.js");

const press: VizActionRunner = async () => ({ result: null, audit_id: 1, event_id: null, inverse: null });
const wrapper = ({ children }: { children: ReactNode }) => <VizActionRunnerContext.Provider value={press}>{children}</VizActionRunnerContext.Provider>;
const render = (ui: ReactElement) => rtlRender(ui, { wrapper });
afterEach(cleanup);
afterAll(() => {
  mock.module("../api.js", () => realApi);
});

const base = {
  id: "x/files",
  extension: "x",
  slug: "files",
  title: "Files",
  description: "",
  query: "",
  viz: "tree",
  params: [],
  columns: [{ key: "path", label: "File", link: { kind: "file", from: "file_path", line: null, base: null, head: null }, unit: null, icon: null, tone: null }],
  empty: null,
  chart: null,
  tree: { path: "path", status: "status" },
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
} as unknown as VizConfig;

const runOf = (rows: [string, string | null][], config: Partial<VizConfig> = {}): VizRun =>
  ({
    config: { ...base, ...config },
    params: {},
    result: { columns: ["path", "status", "file_path"], rows: rows.map(([p, s]) => [p, s, s === "deleted" ? null : p] as SqlCell[]), truncated: false },
  }) as unknown as VizRun;

const labels = (view: ReturnType<typeof render>) => view.getAllByTestId("viz-tree-row").map((r) => r.querySelector("[data-testid=viz-tree-label]")?.textContent);
const marks = (row: HTMLElement) => Array.from(row.querySelectorAll("[data-testid=viz-mark]")).map((m) => [m.textContent, m.getAttribute("data-tone"), m.getAttribute("title")]);

test("a path tree draws its made folders, each name with its marks at the end", () => {
  const view = render(<VizView run={runOf([["src/ui/a.rs", "modified"], ["src/new.rs", "added"], ["old.rs", "deleted"]])} onOpenPage={() => {}} />);
  expect(labels(view)).toEqual(["src", "ui", "a.rs", "new.rs", "old.rs"]);
  const rows = view.getAllByTestId("viz-tree-row");
  expect(marks(rows[0]!)).toEqual([
    ["A", "success", "added"],
    ["M", "warning", "modified"],
  ]);
  expect(marks(rows[4]!)).toEqual([["D", "danger", "deleted"]]);
  // A file's name links by its own column; a deleted one has nothing to open.
  // (A link is a button: `RouteLink`.)
  const link = (row: HTMLElement) => row.querySelector("[data-testid=viz-tree-label] button");
  expect(link(rows[2]!)).not.toBeNull();
  expect(link(rows[4]!)).toBeNull();
  // A made folder is no row of the result: no link, nothing to act on.
  expect(link(rows[0]!)).toBeNull();
});

test("a long name is cut short, its whole path on hover", () => {
  const view = render(<VizView run={runOf([["src/a-very-long-file-name-that-will-not-fit.rs", "added"]])} onOpenPage={() => {}} />);
  const label = view.getAllByTestId("viz-tree-label")[1]!;
  expect(label.getAttribute("title")).toBe("src/a-very-long-file-name-that-will-not-fit.rs");
  expect(label.style.textOverflow).toBe("ellipsis");
  expect(label.style.overflow).toBe("hidden");
});

test("compact, a tree shows its first visible rows and how many more, and folding frees rows", () => {
  const files: [string, string][] = Array.from({ length: 10 }, (_, i) => [`dir/f${i}.rs`, "added"]);
  let asked = 0;
  const view = render(<VizView run={runOf([...files, ["top.rs", "modified"]])} compact maxRows={8} onMore={() => asked++} onOpenPage={() => {}} />);
  // `dir` and seven of its files; `top.rs` and three files are past the cap.
  expect(view.getAllByTestId("viz-tree-row")).toHaveLength(8);
  const moreRow = view.getByTestId("viz-more");
  expect(moreRow.textContent).toBe("+4 more");
  // It asks for the rest (a rail panel grows to show them).
  fireEvent.click(within(moreRow).getByRole("button"));
  expect(asked).toBe(1);
  // Folded, `dir` is one row: everything fits.
  fireEvent.click(within(view.getAllByTestId("viz-tree-row")[0]!).getByRole("button", { name: "Collapse" }));
  expect(labels(view)).toEqual(["dir", "top.rs"]);
  expect(view.queryByTestId("viz-more")).toBeNull();
});

test("compact, a tree's \"+N more\" counts the files it doesn't show, not its folder rows", () => {
  const files: [string, string][] = [
    ...Array.from({ length: 7 }, (_, i): [string, string] => [`a/f${i}.rs`, "added"]),
    ["b/g.rs", "modified"],
    ["c/h.rs", "modified"],
  ];
  const view = render(<VizView run={runOf(files)} compact maxRows={8} />);
  // `a` and its seven files fill the cap; `b`, `c` and their files are past
  // it: four rows, but only two files.
  expect(view.getByTestId("viz-more").textContent).toBe("+2 more");
  // Folded, `a` holds its files: shown, they count as seen.
  fireEvent.click(within(view.getAllByTestId("viz-tree-row")[0]!).getByRole("button", { name: "Collapse" }));
  expect(view.queryByTestId("viz-more")).toBeNull();
});

test("compact, a list past its cap says how many more", () => {
  const list = { ...runOf([]), config: { ...base, viz: "list", tree: null } } as unknown as VizRun;
  const run = { ...list, result: { columns: ["path"], rows: Array.from({ length: 11 }, (_, i) => [`f${i}`]), truncated: false } } as unknown as VizRun;
  const view = render(<VizView run={run} compact maxRows={8} onOpenPage={() => {}} />);
  expect(view.getByTestId("viz-more").textContent).toBe("+3 more");
  // With no one to ask, it is text.
  expect(within(view.getByTestId("viz-more")).queryByRole("button")).toBeNull();
});

test("a marks column draws each mark in its tone, its hint on hover", () => {
  const run = {
    config: { ...base, viz: "list", tree: null, columns: [{ key: "line", label: null, link: null, unit: null, icon: null, tone: null, format: "marks" }] },
    params: {},
    result: { columns: ["line"], rows: [[JSON.stringify([{ text: "4A", tone: "success", hint: "added" }, { text: "−3", tone: "danger" }, "plain"])]], truncated: false },
  } as unknown as VizRun;
  const view = render(<VizView run={run} compact onOpenPage={() => {}} />);
  expect(marks(view.container as HTMLElement)).toEqual([
    ["4A", "success", "added"],
    ["−3", "danger", null],
    ["plain", null, null],
  ]);
});

test("a mark with `gap` starts a new group: a dot before it", () => {
  const run = {
    config: { ...base, viz: "list", tree: null, columns: [{ key: "line", label: null, link: null, unit: null, icon: null, tone: null, format: "marks" }] },
    params: {},
    result: { columns: ["line"], rows: [[JSON.stringify([{ text: "4A" }, { text: "+2", gap: true }])]], truncated: false },
  } as unknown as VizRun;
  const view = render(<VizView run={run} compact onOpenPage={() => {}} />);
  expect(Array.from(view.container.querySelectorAll("[data-testid=viz-mark]")).map((m) => m.textContent)).toEqual(["4A", "+2"]);
  const dots = view.container.querySelectorAll("[data-testid=viz-mark-gap]");
  expect(dots.length).toBe(1);
  expect(dots[0]!.textContent).toBe("·");
  expect(dots[0]!.nextElementSibling?.textContent).toBe("+2");
});
