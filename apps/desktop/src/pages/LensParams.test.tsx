import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import type { LensRun, SqlCell } from "../api.js";
import { ParamsForm } from "./LensPage.js";

afterEach(cleanup);

// tsk1100: a choice param is a select over the values its lens declares.
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
