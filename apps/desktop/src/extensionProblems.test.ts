import { expect, test } from "bun:test";

import { extensionProblemLine, problemsFromResult } from "./extensionProblems.js";

const res = (rows: unknown[][]) =>
  ({ columns: ["name", "problem"], rows, truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }) as never;

test("a row per problem gathers into one problem per extension, in order", () => {
  expect(problemsFromResult(res([["a", "collector x stopped after 3 failures"], ["a", "needs your approval to run"], ["b", "bad page"]]))).toEqual([
    { name: "a", errors: ["collector x stopped after 3 failures", "needs your approval to run"] },
    { name: "b", errors: ["bad page"] },
  ]);
});

test("the line names the first error and counts the rest", () => {
  expect(extensionProblemLine({ name: "a", errors: ["x"] })).toBe("`a` isn't running as it should: x");
  expect(extensionProblemLine({ name: "a", errors: ["x", "y", "z"] })).toBe("`a` isn't running as it should: x (+2 more)");
});
