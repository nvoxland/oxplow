import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { SqlCell, SqlQueryResult } from "../api.js";
import type { LensParam } from "../tauri-bridge/generated/bindings.js";
import { ParamControl } from "./ParamControls.js";

afterEach(cleanup);

function param(over: Partial<LensParam> & { name: string }): LensParam {
  return { presets: [], ...over } as LensParam;
}

function setup(p: LensParam, value: SqlCell, extra: { compact?: boolean; query?: (sql: string) => Promise<SqlQueryResult> } = {}) {
  const applied: [string, SqlCell][] = [];
  const view = render(<ParamControl param={p} value={value} onApply={(n, v) => applied.push([n, v])} {...extra} />);
  return { applied, view, el: view.getByTestId(`lens-param-${p.name}`) };
}

test("text applies on Enter and on blur, and Escape reverts", () => {
  const { applied, el } = setup(param({ name: "q", type: "text" }), "a");
  const input = el as HTMLInputElement;
  fireEvent.change(input, { target: { value: "42" } });
  fireEvent.keyDown(input, { key: "Enter" });
  fireEvent.change(input, { target: { value: "zzz" } });
  fireEvent.keyDown(input, { key: "Escape" });
  // Escape goes back to what was last applied.
  expect(input.value).toBe("42");
  fireEvent.change(input, { target: { value: "b" } });
  fireEvent.blur(input);
  expect(applied).toEqual([["q", 42], ["q", "b"]]);
});

test("choice is a select of its options", () => {
  const { applied, el } = setup(param({ name: "mode", type: "choice", options: [{ value: "a", label: "A" }, { value: "b", label: "B" }] }), "a");
  expect([...(el as HTMLSelectElement).options].map((o) => o.textContent)).toEqual(["A", "B"]);
  fireEvent.change(el, { target: { value: "b" } });
  expect(applied).toEqual([["mode", "b"]]);
});

test("a text param named like an id param draws a picker over what exists", async () => {
  const query = async () => ({ columns: ["value", "label"], rows: [["t1", "First"], ["t2", "Second"]] }) as unknown as SqlQueryResult;
  const { applied, el } = setup(param({ name: "thread_id", type: "text" }), "t2", { query });
  await waitFor(() => expect([...(el as HTMLSelectElement).options].map((o) => o.textContent)).toEqual(["First", "Second"]));
  fireEvent.change(el, { target: { value: "0" } });
  expect(applied).toEqual([["thread_id", "t1"]]);
});

test("bool is a checkbox: checked when truthy, applies true and false", () => {
  const on = setup(param({ name: "all", type: "bool" }), 1);
  expect((on.el as HTMLInputElement).checked).toBe(true);
  fireEvent.click(on.el);
  expect(on.applied).toEqual([["all", false]]);
  cleanup();
  const off = setup(param({ name: "all", type: "bool" }), 0);
  expect((off.el as HTMLInputElement).checked).toBe(false);
  fireEvent.click(off.el);
  expect(off.applied).toEqual([["all", true]]);
});

test("search applies only after the pause, and once", async () => {
  const { applied, el } = setup(param({ name: "q", type: "search" }), null);
  fireEvent.change(el, { target: { value: "fo" } });
  fireEvent.change(el, { target: { value: "foo" } });
  expect(applied).toEqual([]);
  await waitFor(() => expect(applied).toEqual([["q", "foo"]]), { timeout: 1000 });
});

test("search applies at once on Enter; Escape clears and applies null", async () => {
  const { applied, el } = setup(param({ name: "q", type: "search" }), null);
  fireEvent.change(el, { target: { value: "bar" } });
  fireEvent.keyDown(el, { key: "Enter" });
  expect(applied).toEqual([["q", "bar"]]);
  fireEvent.keyDown(el, { key: "Escape" });
  expect((el as HTMLInputElement).value).toBe("");
  expect(applied).toEqual([["q", "bar"], ["q", null]]);
  // The pause that follows adds nothing: both were applied already.
  await act(() => new Promise((r) => setTimeout(r, 300)));
  expect(applied.length).toBe(2);
});

test("branch lists local branches after All branches, which applies null", async () => {
  let asked = "";
  const query = async (sql: string) => {
    asked = sql;
    return { columns: ["name"], rows: [["main"], ["Bugfixes"]] } as unknown as SqlQueryResult;
  };
  const { applied, el } = setup(param({ name: "branch", type: "branch" }), "main", { query });
  await waitFor(() => expect([...(el as HTMLSelectElement).options].map((o) => o.textContent)).toEqual(["All branches", "main", "Bugfixes"]));
  expect(asked).toContain("v_branch");
  fireEvent.change(el, { target: { value: "Bugfixes" } });
  fireEvent.change(el, { target: { value: "" } });
  expect(applied).toEqual([["branch", "Bugfixes"], ["branch", null]]);
});

test("date_range offers its presets, applying the key picked", () => {
  const { applied, el } = setup(param({ name: "range", type: "date_range", presets: ["7d", "all"] }), "7d");
  expect([...(el as HTMLSelectElement).options].map((o) => o.textContent)).toEqual(["Last 7 days", "All time", "Custom…"]);
  fireEvent.change(el, { target: { value: "all" } });
  expect(applied).toEqual([["range", "all"]]);
});

test("date_range defaults its presets", () => {
  const { el } = setup(param({ name: "range", type: "date_range" }), "1d");
  expect([...(el as HTMLSelectElement).options].map((o) => o.textContent)).toEqual(["Last day", "Last 7 days", "Last 30 days", "All time", "Custom…"]);
});

test("Custom… shows two dates and applies a..b, either side may be empty", () => {
  const { applied, el, view } = setup(param({ name: "range", type: "date_range" }), "7d");
  expect(view.queryByTestId("lens-param-range-from")).toBeNull();
  fireEvent.change(el, { target: { value: "__custom__" } });
  fireEvent.change(view.getByTestId("lens-param-range-from"), { target: { value: "2026-01-01" } });
  expect(applied).toEqual([["range", "2026-01-01.."]]);
});

test("a range value a..b shows Custom… with its dates", () => {
  const { applied, el, view } = setup(param({ name: "range", type: "date_range" }), "2026-01-01..2026-02-01");
  expect((el as HTMLSelectElement).value).toBe("__custom__");
  expect((view.getByTestId("lens-param-range-from") as HTMLInputElement).value).toBe("2026-01-01");
  fireEvent.change(view.getByTestId("lens-param-range-to"), { target: { value: "2026-03-01" } });
  expect(applied).toEqual([["range", "2026-01-01..2026-03-01"]]);
});

test("compact drops the label row and names the control", () => {
  const { view, el } = setup(param({ name: "q", type: "text", label: "Find" }), null, { compact: true });
  expect(el.getAttribute("aria-label")).toBe("Find");
  expect(view.queryByText("Find")).toBeNull();
});

/** A control shows what it just applied at once, before the lens's run
 *  echoes it back, so a click or pick never seems to bounce. */
test("a control shows the value it applied before the run echoes it", () => {
  const view = render(<ParamControl param={param({ name: "on", type: "bool" })} value={0} onApply={() => {}} />);
  const box = view.getByTestId("lens-param-on") as HTMLInputElement;
  fireEvent.click(box);
  expect(box.checked).toBe(true);
  cleanup();
  const pick = render(
    <ParamControl param={param({ name: "m", type: "choice", options: [{ value: "a", label: "A" }, { value: "b", label: "B" }] })} value="a" onApply={() => {}} />,
  );
  const select = pick.getByTestId("lens-param-m") as HTMLSelectElement;
  fireEvent.change(select, { target: { value: "b" } });
  expect(select.value).toBe("b");
});
