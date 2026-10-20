import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import type { LensRun, SqlCell } from "../api.js";
import { ParamsForm, ParamsToolbar } from "./LensPage.js";

afterEach(cleanup);

// A choice param is a select over the values its lens declares.
test("a choice param is a select of its options", () => {
  const lens = {
    params: [
      { name: "mode", label: "Show", default: "recent", options: [{ value: "recent", label: "Recent" }, { value: "top", label: "Most visited" }] },
    ],
  } as unknown as LensRun["lens"];
  const applied: [string, SqlCell][] = [];
  const view = render(<ParamsForm lens={lens} values={{ mode: "recent" }} onApply={(n, v) => applied.push([n, v])} />);
  const select = view.getByTestId("lens-param-mode") as HTMLSelectElement;
  expect([...select.options].map((o) => o.textContent)).toEqual(["Recent", "Most visited"]);
  fireEvent.change(select, { target: { value: "top" } });
  expect(applied).toEqual([["mode", "top"]]);
});

const placed = {
  params: [
    { name: "q", label: "Search", default: null, options: [], type: "search", place: "toolbar", presets: [] },
    { name: "range", label: "When", default: "7d", options: [], type: "date_range", place: "toolbar", presets: [] },
    { name: "commit", label: "Commit", default: null, options: [], type: "text", place: "rail", presets: [] },
  ],
} as unknown as LensRun["lens"];

/** A param's place says where it draws: the toolbar above the body, or
 *  the rail; each draws only its own. */
test("toolbar params draw in the toolbar, rail params in the rail", () => {
  const rail = render(<ParamsForm lens={placed} values={{}} onApply={() => {}} />);
  expect(rail.queryByTestId("lens-param-commit")).not.toBeNull();
  expect(rail.queryByTestId("lens-param-q")).toBeNull();
  cleanup();
  const bar = render(<ParamsToolbar lens={placed} values={{ range: "7d" }} onApply={() => {}} />);
  expect(bar.getByTestId("lens-params-toolbar").className).toContain("ox-toolbar");
  expect(bar.queryByTestId("lens-param-q")).not.toBeNull();
  expect(bar.queryByTestId("lens-param-range")).not.toBeNull();
  expect(bar.queryByTestId("lens-param-commit")).toBeNull();
});

test("with no params of its place, neither draws", () => {
  const none = { params: [placed.params[2]] } as unknown as LensRun["lens"];
  expect(render(<ParamsToolbar lens={none} values={{}} onApply={() => {}} />).container.innerHTML).toBe("");
});
