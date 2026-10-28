import { expect, test } from "bun:test";

import { extensionProblemLine, problemsFromResult } from "./extensionProblems.js";

const res = (rows: unknown[][]) =>
  ({ columns: ["name", "enabled", "errors", "deployment"], rows, truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }) as never;

test("rows become problems with parsed errors", () => {
  expect(problemsFromResult(res([["a", 0, '["x","y"]', null], ["b", 1, "oops", null]]))).toEqual([
    { name: "a", errors: ["x", "y"] },
    { name: "b", errors: ["oops"] },
  ]);
});

test("the line names the first error and counts the rest", () => {
  expect(extensionProblemLine({ name: "a", errors: ["x"] })).toBe("`a` isn't running as it should: x");
  expect(extensionProblemLine({ name: "a", errors: ["x", "y", "z"] })).toBe("`a` isn't running as it should: x (+2 more)");
});
