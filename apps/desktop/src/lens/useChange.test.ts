import { expect, test } from "bun:test";
import { shouldReensure } from "./useChange.js";

const row = { id: 5, streamId: 1 };

test("a stale stream re-ensures mutable changes on it only", () => {
  const working = { kind: "working" as const, streamId: "str1" };
  expect(shouldReensure({ kind: "changeStale", streamId: 1 }, working, row)).toBe(true);
  expect(shouldReensure({ kind: "changeStale", streamId: 2 }, working, row)).toBe(false);
  const commit = { kind: "commit" as const, sha: "abc", streamId: null };
  expect(shouldReensure({ kind: "changeStale", streamId: 1 }, commit, row)).toBe(false);
});

test("the change's own analysis landing refreshes it", () => {
  const commit = { kind: "commit" as const, sha: "abc", streamId: null };
  expect(shouldReensure({ kind: "changeAnalyzed", changeId: 5 }, commit, row)).toBe(true);
  expect(shouldReensure({ kind: "changeAnalyzed", changeId: 6 }, commit, row)).toBe(false);
  expect(shouldReensure({ kind: "pageVisitChanged" }, commit, row)).toBe(false);
  expect(shouldReensure({ kind: "changeStale", streamId: 1 }, { kind: "effort", effortId: "eff3" }, null)).toBe(false);
});
