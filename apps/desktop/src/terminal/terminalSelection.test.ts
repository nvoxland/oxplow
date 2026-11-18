import { expect, test } from "bun:test";

import { linesAround } from "./terminalSelection.js";

// A terminal's selection has no address to read back, so it goes to the
// basket with the lines around it: a few rows either side, a wrapped row
// joined to the row it continues, blank edges dropped.

const screen = (rows: Array<string | [string, "wrapped"]>) => (y: number) => {
  const row = rows[y];
  if (row === undefined) return undefined;
  return typeof row === "string" ? { text: row, wrapped: false } : { text: row[0], wrapped: true };
};

test("the selected rows and three either side, blank edges dropped", () => {
  const rows = screen(["", "one", "two", "three", "Checks:", "- e2e: the full suite passes, 124 of 124", "- lint: clean", "", "", "", "after"]);
  expect(linesAround(rows, 5, 5)).toBe("two\nthree\nChecks:\n- e2e: the full suite passes, 124 of 124\n- lint: clean");
});

test("a wrapped row joins the row it continues, and trailing spaces go", () => {
  const rows = screen(["before   ", "- e2e: the full suite ", ["passes, 124 of 124", "wrapped"], "after"]);
  expect(linesAround(rows, 2, 2, 1)).toBe("- e2e: the full suite passes, 124 of 124\nafter");
  expect(linesAround(rows, 1, 2, 1)).toBe("before\n- e2e: the full suite passes, 124 of 124\nafter");
});

test("near the buffer's edges it takes what there is", () => {
  const rows = screen(["first", "second"]);
  expect(linesAround(rows, 0, 1)).toBe("first\nsecond");
});
