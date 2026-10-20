import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import type { LensRun, SqlCell } from "../api.js";
import { ParamsForm } from "./LensPage.js";

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

const params = {
  params: [
    { name: "q", label: "Search", default: null, options: [], type: "search", role: "filter", presets: [] },
    { name: "range", label: "When", default: "7d", options: [], type: "date_range", role: "filter", presets: [] },
    { name: "commit", label: "Commit", default: null, options: [], type: "text", role: "selection", presets: [] },
  ],
} as unknown as LensRun["lens"];

/** A page has one place for its params: the rail draws every one. */
test("every param draws in the rail", () => {
  const rail = render(<ParamsForm lens={params} values={{ range: "7d" }} onApply={() => {}} />);
  for (const name of ["q", "range", "commit"]) expect(rail.queryByTestId(`lens-param-${name}`)).not.toBeNull();
});

test("with no params, the rail draws nothing", () => {
  const none = { params: [] } as unknown as LensRun["lens"];
  expect(render(<ParamsForm lens={none} values={{}} onApply={() => {}} />).container.innerHTML).toBe("");
});
