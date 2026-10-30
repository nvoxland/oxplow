import { expect, test } from "bun:test";

import { extensionProblemLine, problemsFromResult } from "./extensionProblems.js";

const res = (rows: unknown[][]) =>
  ({ columns: ["name", "enabled", "errors", "deployment"], rows, truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }) as never;

test("rows become problems with each diagnostic's shown line", () => {
  const errors = JSON.stringify([{ code: "manifest.parse", message: "bad", shown: "x" }, { code: "page.invalid", message: "worse", shown: "y" }]);
  expect(problemsFromResult(res([["a", 0, errors, null], ["b", 1, "[]", null]]))).toEqual([
    { name: "a", errors: ["x", "y"] },
    { name: "b", errors: [] },
  ]);
});

test("the line names the first error and counts the rest", () => {
  expect(extensionProblemLine({ name: "a", errors: ["x"] })).toBe("`a` isn't running as it should: x");
  expect(extensionProblemLine({ name: "a", errors: ["x", "y", "z"] })).toBe("`a` isn't running as it should: x (+2 more)");
});
