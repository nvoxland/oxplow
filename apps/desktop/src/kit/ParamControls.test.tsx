import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { SqlCell, SqlQueryResult } from "../api.js";
import type { PageParam } from "../tauri-bridge/generated/bindings.js";
import { ParamList, ParamRow } from "./ParamControls.js";

afterEach(cleanup);

function param(over: Partial<PageParam> & { name: string }): PageParam {
  return { label: null, default: null, options: [], presets: [], role: "filter", ...over } as PageParam;
}

type Query = (sql: string) => Promise<SqlQueryResult>;

/** What a select shows: its chosen option's text. */
const picked = (el: HTMLElement) => (el as HTMLSelectElement).selectedOptions[0]?.textContent ?? null;

function setup(p: PageParam, value: SqlCell, query?: Query) {
  const applied: [string, SqlCell][] = [];
  const view = render(<ParamRow param={p} value={value} onApply={(n, v) => applied.push([n, v])} query={query} />);
  const valueButton = () => view.getByTestId(`page-param-${p.name}-value`);
  const edit = () => view.getByTestId(`page-param-${p.name}-edit`);
  return { applied, view, valueButton, edit };
}

// At rest a row is its label beside its value; nothing to edit is drawn.
test("a row shows its label and its value, and Any when it has none", () => {
  const { view, valueButton } = setup(param({ name: "author_name", type: "text" }), null);
  expect(view.getByTestId("page-param-author_name").textContent).toContain("Author name");
  expect(valueButton().textContent).toBe("Any");
  expect(view.queryByTestId("page-param-author_name-edit")).toBeNull();
});

test("clicking the value edits text in place: Enter applies and closes, focus back on the value", () => {
  const { applied, valueButton, edit, view } = setup(param({ name: "q", label: "Query", type: "text" }), "a");
  fireEvent.click(valueButton());
  const input = edit() as HTMLInputElement;
  expect(document.activeElement).toBe(input);
  fireEvent.change(input, { target: { value: "42" } });
  fireEvent.keyDown(input, { key: "Enter" });
  expect(applied).toEqual([["q", 42]]);
  expect(view.queryByTestId("page-param-q-edit")).toBeNull();
  expect(valueButton().textContent).toBe("42");
  expect(document.activeElement).toBe(valueButton());
});

test("Escape closes an edit and keeps the value from before it opened", () => {
  const { applied, valueButton, edit, view } = setup(param({ name: "q", type: "text" }), "a");
  fireEvent.click(valueButton());
  fireEvent.change(edit(), { target: { value: "zzz" } });
  fireEvent.keyDown(edit(), { key: "Escape" });
  expect(applied).toEqual([]);
  expect(view.queryByTestId("page-param-q-edit")).toBeNull();
  expect(valueButton().textContent).toBe("a");
});

// A value picked from a list is its select at rest: one click opens it (a
// button swapped for a select on click only focused it).
test("a choice is its select at rest, showing its option's label, and applies the one picked", () => {
  const p = param({ name: "mode", type: "choice", default: "a", options: [{ value: "a", label: "Recent" }, { value: "b", label: "Most visited" }] });
  const { applied, edit, view } = setup(p, "a");
  expect(view.queryByTestId("page-param-mode-value")).toBeNull();
  expect(picked(edit())).toBe("Recent");
  expect(document.activeElement).not.toBe(edit());
  expect([...(edit() as HTMLSelectElement).options].map((o) => o.textContent)).toEqual(["Recent", "Most visited"]);
  fireEvent.change(edit(), { target: { value: "b" } });
  expect(applied).toEqual([["mode", "b"]]);
  expect((edit() as HTMLSelectElement).value).toBe("b");
});

// A choice with `options_from` asks the daemon for the param's options
// with the page's values (the daemon keeps the ones the source declares),
// shows the option's label, groups them, and follows what was read.
test("a dynamic choice loads its options for its page and param with the page's values, grouped", async () => {
  const runs: [string, string, Record<string, SqlCell>, string | null][] = [];
  const source = {
    list: async (page: string, param: string, values: Record<string, SqlCell>, streamId: string | null) => {
      runs.push([page, param, values, streamId]);
      return {
        columns: ["value", "label", "group"],
        rows: [
          ["snap:1", "First", "Snapshots"],
          ["snap:2", "Second", "Snapshots"],
          ["working", "Working tree", null],
        ],
        reads: { datasets: ["v_snapshot"], tables: [], measures: [] },
      } as unknown as SqlQueryResult;
    },
  };
  const p = param({ name: "end", type: "choice", options_from: "x/ends" });
  const applied: [string, SqlCell][] = [];
  const view = render(<ParamRow param={p} value="snap:2" values={{ start: "snap:1", end: "snap:2", other: 1 }} streamId="str1" onApply={(n, v) => applied.push([n, v])} optionsSource={source} page="x/changes" />);
  const edit = view.getByTestId("page-param-end-edit") as HTMLSelectElement;
  await waitFor(() => expect(picked(edit)).toBe("Second"));
  expect(runs).toEqual([["x/changes", "end", { start: "snap:1", end: "snap:2", other: 1 }, "str1"]]);
  expect([...edit.querySelectorAll("optgroup")].map((g) => g.label)).toEqual(["Snapshots"]);
  expect([...edit.options].map((o) => o.textContent)).toEqual(["Working tree", "First", "Second"]);
  fireEvent.change(edit, { target: { value: "working" } });
  expect(applied).toEqual([["end", "working"]]);
});

test("a text param named like an id param picks what exists, shown by its title", async () => {
  const query: Query = async () => ({ columns: ["value", "label"], rows: [["t1", "First"], ["t2", "Second"]] }) as unknown as SqlQueryResult;
  const { applied, edit } = setup(param({ name: "thread_id", type: "text" }), "t2", query);
  await waitFor(() => expect(picked(edit())).toBe("Second"));
  fireEvent.change(edit(), { target: { value: "0" } });
  expect(applied).toEqual([["thread_id", "t1"]]);
});

// A bool has nothing to open: its value is the checkbox.
test("a bool is a checkbox in its row: checked when truthy, applies true and false", () => {
  const on = setup(param({ name: "merges", label: "Merges only", type: "bool", default: false }), 1);
  const box = on.edit() as HTMLInputElement;
  expect(box.checked).toBe(true);
  expect(box.getAttribute("aria-label")).toBe("Merges only");
  fireEvent.click(box);
  expect(on.applied).toEqual([["merges", false]]);
  cleanup();
  const off = setup(param({ name: "merges", type: "bool" }), 0);
  fireEvent.click(off.edit());
  expect(off.applied).toEqual([["merges", true]]);
});

test("a search applies as you type, after a pause, and once", async () => {
  const { applied, valueButton, edit } = setup(param({ name: "q", type: "search" }), null);
  fireEvent.click(valueButton());
  fireEvent.change(edit(), { target: { value: "n" } });
  fireEvent.change(edit(), { target: { value: "numbat" } });
  expect(applied).toEqual([]);
  await waitFor(() => expect(applied).toEqual([["q", "numbat"]]));
});

test("a search's Escape puts back the value it had when it opened", async () => {
  const { applied, valueButton, edit, view } = setup(param({ name: "q", type: "search" }), "old");
  fireEvent.click(valueButton());
  fireEvent.change(edit(), { target: { value: "new" } });
  fireEvent.keyDown(edit(), { key: "Enter" });
  expect(applied).toEqual([["q", "new"]]);
  // Enter closed it; open again, type, Escape: back to "new".
  fireEvent.click(valueButton());
  fireEvent.change(edit(), { target: { value: "newer" } });
  fireEvent.keyDown(edit(), { key: "Escape" });
  expect(view.queryByTestId("page-param-q-edit")).toBeNull();
  await act(async () => new Promise((r) => setTimeout(r, 300)));
  expect(applied).toEqual([["q", "new"]]);
});

test("a branch shows All branches for none and lists the local branches", async () => {
  const query: Query = async () => ({ columns: ["name"], rows: [["main"], ["topic"]] }) as unknown as SqlQueryResult;
  const { applied, edit } = setup(param({ name: "br", type: "branch" }), null, query);
  expect(picked(edit())).toBe("All branches");
  await waitFor(() => expect([...(edit() as HTMLSelectElement).options].map((o) => o.textContent)).toEqual(["All branches", "main", "topic"]));
  fireEvent.change(edit(), { target: { value: "topic" } });
  expect(applied).toEqual([["br", "topic"]]);
});

test("a date_range shows its preset's name and applies the preset picked", () => {
  const p = param({ name: "range", label: "When", type: "date_range", default: "30d", presets: ["7d", "30d", "all"] });
  const { applied, edit } = setup(p, "30d");
  expect(picked(edit())).toBe("Last 30 days");
  expect([...(edit() as HTMLSelectElement).options].map((o) => o.textContent)).toEqual(["Last 7 days", "Last 30 days", "All time", "Custom…"]);
  fireEvent.change(edit(), { target: { value: "7d" } });
  expect(applied).toEqual([["range", "7d"]]);
});

// Two dates don't fit a 320px row: Custom… opens them over the page.
test("Custom… opens a popover with two dates; Escape closes it with focus on the select", () => {
  const p = param({ name: "range", label: "When", type: "date_range", default: "30d" });
  const { applied, edit, view } = setup(p, "30d");
  fireEvent.change(edit(), { target: { value: "__custom__" } });
  const pop = view.getByTestId("page-param-range-custom");
  expect(pop.getAttribute("role")).toBe("dialog");
  fireEvent.change(view.getByTestId("page-param-range-from"), { target: { value: "2026-10-01" } });
  expect(applied).toEqual([["range", "2026-10-01.."]]);
  fireEvent.keyDown(view.getByTestId("page-param-range-from"), { key: "Escape" });
  expect(view.queryByTestId("page-param-range-custom")).toBeNull();
  expect(document.activeElement).toBe(edit());
});

test("a custom range shows its two ends", () => {
  const { edit } = setup(param({ name: "range", type: "date_range" }), "2026-10-01..2026-10-20");
  expect(picked(edit())).toBe("2026-10-01 – 2026-10-20");
});

test("a row whose value isn't its default is marked changed", () => {
  const p = param({ name: "range", type: "date_range", default: "30d" });
  expect(setup(p, "30d").view.getByTestId("page-param-range").className).not.toContain("ox-param-changed");
  cleanup();
  expect(setup(p, "7d").view.getByTestId("page-param-range").className).toContain("ox-param-changed");
  cleanup();
  // A bool's echo is 0/1 against a default of false.
  expect(setup(param({ name: "m", type: "bool", default: false }), 0).view.getByTestId("page-param-m").className).not.toContain("ox-param-changed");
});

test("a row shows the value it applied before the run echoes it", () => {
  const { valueButton, edit } = setup(param({ name: "q", type: "text" }), "a");
  fireEvent.click(valueButton());
  fireEvent.change(edit(), { target: { value: "b" } });
  fireEvent.keyDown(edit(), { key: "Enter" });
  expect(valueButton().textContent).toBe("b");
});

// Six rows at most; the rest wait behind More, unless one is set.
test("a ParamList shows six rows, the rest behind More (N), and a changed one always", () => {
  const params = Array.from({ length: 9 }, (_, i) => param({ name: `p${i}`, type: "text" }));
  const view = render(<ParamList params={params} values={{ p8: "set" }} onApply={() => {}} />);
  const shown = () => params.filter((p) => view.queryByTestId(`page-param-${p.name}`) !== null).map((p) => p.name);
  expect(shown()).toEqual(["p0", "p1", "p2", "p3", "p4", "p5", "p8"]);
  const more = view.getByTestId("page-params-more");
  expect(more.textContent).toBe("More (2)");
  fireEvent.click(more);
  expect(shown()).toEqual(params.map((p) => p.name));
});

// A value you type (text, search) reads as a text box at rest: no caret, a
// search empty says "Search…"; a value you pick is its select.
test("a typed value has no caret and an empty search says Search…; a choice is its select", () => {
  const search = setup(param({ name: "q", type: "search" }), null);
  expect(search.valueButton().className).toContain("ox-param-typed");
  expect(search.valueButton().textContent).toBe("Search…");
  cleanup();
  const text = setup(param({ name: "author_name", type: "text" }), "ana");
  expect(text.valueButton().className).toContain("ox-param-typed");
  cleanup();
  const choice = setup(param({ name: "mode", type: "choice", default: "a", options: [{ value: "a", label: "A", icon: null }] }), "a");
  expect(choice.edit().className).not.toContain("ox-param-typed");
});
