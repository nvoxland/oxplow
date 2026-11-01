import { expect, test } from "bun:test";

import type { BranchRun } from "../viz/vizModel.js";
import { keepPrevious } from "./pageBranches.js";

const ran = { run: { rows: 1 }, error: null } as unknown as BranchRun;
const failed: BranchRun = { run: null, error: "boom" };

test("a re-opened branch with no answer yet keeps its previous rows", () => {
  const out = keepPrevious(new Map(), new Map([["a", ran]]), ["a"]);
  expect(out.get("a")).toBe(ran);
});

test("an answered branch is not replaced by the previous one", () => {
  const next = { run: { rows: 2 }, error: null } as unknown as BranchRun;
  expect(keepPrevious(new Map([["a", next]]), new Map([["a", ran]]), ["a"]).get("a")).toBe(next);
});

test("a branch never answered, or one that only failed, stays loading", () => {
  expect(keepPrevious(new Map(), new Map([["a", failed]]), ["a", "b"]).size).toBe(0);
});

test("a branch that is no longer wanted is not kept", () => {
  expect(keepPrevious(new Map(), new Map([["a", ran]]), []).size).toBe(0);
});
