import { expect, test } from "bun:test";

import { extensionProblemLine, problemsFromResult } from "./extensionProblems.js";

const res = (rows: unknown[][]) =>
  ({ columns: ["name", "problem", "at"], rows, truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }) as never;

test("a row per problem gathers into one problem per extension, in order, at its newest failure", () => {
  expect(problemsFromResult(res([["a", "collector x stopped after 3 failures", "2026-11-24T09:00:00Z"], ["a", "effect y is failing, 1 in a row", "2026-11-24T10:00:00Z"], ["a", "needs your approval to run", null], ["b", "bad page", null]]))).toEqual([
    { name: "a", errors: ["collector x stopped after 3 failures", "effect y is failing, 1 in a row", "needs your approval to run"], at: "2026-11-24T10:00:00Z" },
    { name: "b", errors: ["bad page"], at: null },
  ]);
});

test("the line names the first error and counts the rest", () => {
  expect(extensionProblemLine({ name: "a", errors: ["x"], at: null })).toBe("`a` isn't running as it should: x");
  expect(extensionProblemLine({ name: "a", errors: ["x", "y", "z"], at: null })).toBe("`a` isn't running as it should: x (+2 more)");
});
