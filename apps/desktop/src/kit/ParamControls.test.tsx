import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { SqlCell, SqlQueryResult } from "../api.js";
import type { Lens, LensParam, LensRun } from "../tauri-bridge/generated/bindings.js";
import { ParamList, ParamRow } from "./ParamControls.js";

afterEach(cleanup);

function param(over: Partial<LensParam> & { name: string }): LensParam {
  return { label: null, default: null, options: [], presets: [], role: "filter", ...over } as LensParam;
}

type Query = (sql: string) => Promise<SqlQueryResult>;

function setup(p: LensParam, value: SqlCell, query?: Query) {
  const applied: [string, SqlCell][] = [];
  const view = render(<ParamRow param={p} value={value} onApply={(n, v) => applied.push([n, v])} query={query} />);
  const valueButton = () => view.getByTestId(`lens-param-${p.name}-value`);
  const edit = () => view.getByTestId(`lens-param-${p.name}-edit`);
  return { applied, view, valueButton, edit };
}

// At rest a row is its label beside its value; nothing to edit is drawn.
test("a row shows its label and its value, and Any when it has none", () => {
  const { view, valueButton } = setup(param({ name: "author_name", type: "text" }), null);
  expect(view.getByTestId("lens-param-author_name").textContent).toContain("Author name");
  expect(valueButton().textContent).toBe("Any");
  expect(view.queryByTestId("lens-param-author_name-edit")).toBeNull();
});

test("clicking the value edits text in place: Enter applies and closes, focus back on the value", () => {
  const { applied, valueButton, edit, view } = setup(param({ name: "q", label: "Query", type: "text" }), "a");
  fireEvent.click(valueButton());
  const input = edit() as HTMLInputElement;
  expect(document.activeElement).toBe(input);
  fireEvent.change(input, { target: { value: "42" } });
  fireEvent.keyDown(input, { key: "Enter" });
  expect(applied).toEqual([["q", 42]]);
  expect(view.queryByTestId("lens-param-q-edit")).toBeNull();
  expect(valueButton().textContent).toBe("42");
  expect(document.activeElement).toBe(valueButton());
});

test("Escape closes an edit and keeps the value from before it opened", () => {
  const { applied, valueButton, edit, view } = setup(param({ name: "q", type: "text" }), "a");
  fireEvent.click(valueButton());
  fireEvent.change(edit(), { target: { value: "zzz" } });
  fireEvent.keyDown(edit(), { key: "Escape" });
  expect(applied).toEqual([]);
  expect(view.queryByTestId("lens-param-q-edit")).toBeNull();
  expect(valueButton().textContent).toBe("a");
});

test("a choice shows its option's label and applies the one picked", () => {
  const p = param({ name: "mode", type: "choice", default: "a", options: [{ value: "a", label: "Recent" }, { value: "b", label: "Most visited" }] });
  const { applied, valueButton, edit, view } = setup(p, "a");
  expect(valueButton().textContent).toBe("Recent");
  fireEvent.click(valueButton());
  expect([...(edit() as HTMLSelectElement).options].map((o) => o.textContent)).toEqual(["Recent", "Most visited"]);
  fireEvent.change(edit(), { target: { value: "b" } });
  expect(applied).toEqual([["mode", "b"]]);
  expect(view.queryByTestId("lens-param-mode-edit")).toBeNull();
  expect(valueButton().textContent).toBe("Most visited");
});

// A choice whose options a lens lists runs that lens with the page's
// values it declares, shows the option's label, groups them, and follows
// the lens's reads.
test("a dynamic choice loads its options from its lens with the page's params, grouped", async () => {
  const runs: [string, Record<string, SqlCell>, string | null][] = [];
  const source = {
    lens: async (id: string) => ({ id, params: [{ name: "start" }, { name: "stream_id" }] }) as unknown as Lens,
    run: async (id: string, params: Record<string, SqlCell>, streamId: string | null) => {
      runs.push([id, params, streamId]);
      return {
        result: {
          columns: ["value", "label", "group"],
          rows: [
            ["snap:1", "First", "Snapshots"],
            ["snap:2", "Second", "Snapshots"],
            ["working", "Working tree", null],
          ],
          reads: { models: ["v_snapshot"], tables: [], measures: [] },
        },
      } as unknown as LensRun;
    },
  };
  const p = param({ name: "end", type: "choice", options_from: "x/ends" });
  const applied: [string, SqlCell][] = [];
  const view = render(<ParamRow param={p} value="snap:2" values={{ start: "snap:1", end: "snap:2", other: 1 }} streamId="str1" onApply={(n, v) => applied.push([n, v])} optionsSource={source} />);
  const valueButton = () => view.getByTestId("lens-param-end-value");
  await waitFor(() => expect(valueButton().textContent).toBe("Second"));
  expect(runs).toEqual([["x/ends", { start: "snap:1" }, "str1"]]);
  fireEvent.click(valueButton());
  const edit = view.getByTestId("lens-param-end-edit") as HTMLSelectElement;
  expect([...edit.querySelectorAll("optgroup")].map((g) => g.label)).toEqual(["Snapshots"]);
  expect([...edit.options].map((o) => o.textContent)).toEqual(["Working tree", "First", "Second"]);
  fireEvent.change(edit, { target: { value: "working" } });
  expect(applied).toEqual([["end", "working"]]);
});

test("a text param named like an id param picks what exists, shown by its title", async () => {
  const query: Query = async () => ({ columns: ["value", "label"], rows: [["t1", "First"], ["t2", "Second"]] }) as unknown as SqlQueryResult;
  const { applied, valueButton, edit } = setup(param({ name: "thread_id", type: "text" }), "t2", query);
  await waitFor(() => expect(valueButton().textContent).toBe("Second"));
  fireEvent.click(valueButton());
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
  expect(view.queryByTestId("lens-param-q-edit")).toBeNull();
  await act(async () => new Promise((r) => setTimeout(r, 300)));
  expect(applied).toEqual([["q", "new"]]);
});

test("a branch shows All branches for none and lists the local branches", async () => {
  const query: Query = async () => ({ columns: ["name"], rows: [["main"], ["topic"]] }) as unknown as SqlQueryResult;
  const { applied, valueButton, edit } = setup(param({ name: "br", type: "branch" }), null, query);
  expect(valueButton().textContent).toBe("All branches");
  fireEvent.click(valueButton());
  await waitFor(() => expect([...(edit() as HTMLSelectElement).options].map((o) => o.textContent)).toEqual(["All branches", "main", "topic"]));
  fireEvent.change(edit(), { target: { value: "topic" } });
  expect(applied).toEqual([["br", "topic"]]);
});

test("a date_range shows its preset's name and applies the preset picked", () => {
  const p = param({ name: "range", label: "When", type: "date_range", default: "30d", presets: ["7d", "30d", "all"] });
  const { applied, valueButton, edit } = setup(p, "30d");
  expect(valueButton().textContent).toBe("Last 30 days");
  fireEvent.click(valueButton());
  expect([...(edit() as HTMLSelectElement).options].map((o) => o.textContent)).toEqual(["Last 7 days", "Last 30 days", "All time", "Custom…"]);
  fireEvent.change(edit(), { target: { value: "7d" } });
  expect(applied).toEqual([["range", "7d"]]);
});

// Two dates don't fit a 320px row: Custom… opens them over the page.
test("Custom… opens a popover with two dates; Escape closes it with focus on the value", () => {
  const p = param({ name: "range", label: "When", type: "date_range", default: "30d" });
  const { applied, valueButton, edit, view } = setup(p, "30d");
  fireEvent.click(valueButton());
  fireEvent.change(edit(), { target: { value: "__custom__" } });
  const pop = view.getByTestId("lens-param-range-custom");
  expect(pop.getAttribute("role")).toBe("dialog");
  fireEvent.change(view.getByTestId("lens-param-range-from"), { target: { value: "2026-10-01" } });
  expect(applied).toEqual([["range", "2026-10-01.."]]);
  fireEvent.keyDown(view.getByTestId("lens-param-range-from"), { key: "Escape" });
  expect(view.queryByTestId("lens-param-range-custom")).toBeNull();
  expect(view.queryByTestId("lens-param-range-edit")).toBeNull();
  expect(document.activeElement).toBe(valueButton());
});

test("a custom range shows its two ends", () => {
  const { valueButton } = setup(param({ name: "range", type: "date_range" }), "2026-10-01..2026-10-20");
  expect(valueButton().textContent).toBe("2026-10-01 – 2026-10-20");
});

test("a row whose value isn't its default is marked changed", () => {
  const p = param({ name: "range", type: "date_range", default: "30d" });
  expect(setup(p, "30d").view.getByTestId("lens-param-range").className).not.toContain("ox-param-changed");
  cleanup();
  expect(setup(p, "7d").view.getByTestId("lens-param-range").className).toContain("ox-param-changed");
  cleanup();
  // A bool's echo is 0/1 against a default of false.
  expect(setup(param({ name: "m", type: "bool", default: false }), 0).view.getByTestId("lens-param-m").className).not.toContain("ox-param-changed");
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
  const shown = () => params.filter((p) => view.queryByTestId(`lens-param-${p.name}`) !== null).map((p) => p.name);
  expect(shown()).toEqual(["p0", "p1", "p2", "p3", "p4", "p5", "p8"]);
  const more = view.getByTestId("lens-params-more");
  expect(more.textContent).toBe("More (2)");
  fireEvent.click(more);
  expect(shown()).toEqual(params.map((p) => p.name));
});
