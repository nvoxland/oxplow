import { expect, test } from "bun:test";

import { verdictItems, verdictOf } from "./effortVerdict.js";
import { foldEmpty } from "../lens/lensModel.js";
import type { LensRun } from "../tauri-bridge/generated/bindings.js";

// tsk1036: the effort page leads with what a reviewer needs first — did it
// test, how much of the change ran, and what's left to check.

const row = (cells: Record<string, number | null>) => ({
  columns: Object.keys(cells),
  rows: [Object.values(cells)],
  truncated: false,
  reads: { models: [], tables: [], measures: [] },
  freshness: [],
});

test("a tested, covered, checked effort reads green", () => {
  const v = verdictOf(row({ runs: 2, last_passed: 3, last_failed: 0, diff_coverage: 100, unverified: 0, to_confirm: 0, decisions: 1 }));
  expect(verdictItems(v)).toEqual([
    { key: "tests", text: "Tests: the last of 2 runs passed (3)", tone: "good" },
    { key: "coverage", text: "Diff coverage: 100%", tone: "good" },
    { key: "claims", text: "Every claim is backed", tone: "good" },
    { key: "decisions", text: "1 decision made", tone: "neutral" },
  ]);
});

test("what needs a look says so", () => {
  const v = verdictOf(row({ runs: 0, last_passed: null, last_failed: null, diff_coverage: null, unverified: 2, to_confirm: 1, decisions: 0 }));
  expect(verdictItems(v)).toEqual([
    { key: "tests", text: "Tests: none ran", tone: "bad" },
    { key: "coverage", text: "Diff coverage: not measured", tone: "neutral" },
    { key: "claims", text: "2 unverified claims", tone: "bad" },
    { key: "decisions", text: "1 decision to confirm", tone: "bad" },
  ]);
  const failing = verdictOf(row({ runs: 1, last_passed: 2, last_failed: 1, diff_coverage: 40, unverified: 0, to_confirm: 0, decisions: 0 }));
  expect(verdictItems(failing)[0]).toEqual({ key: "tests", text: "Tests: the last run failed (1 of 3)", tone: "bad" });
  expect(verdictItems(failing)[1]).toEqual({ key: "coverage", text: "Diff coverage: 40%", tone: "bad" });
});

test("sections with nothing to show fold into one line; grids and errors stay", () => {
  const run = (title: string, rows: number, viz = "table") =>
    ({ lens: { title, viz }, result: { rows: Array.from({ length: rows }, () => []) } }) as unknown as LensRun;
  const runs = [
    { id: "a", run: run("Duplication", 0), error: null },
    { id: "b", run: run("Test Runs", 2), error: null },
    { id: "c", run: run("Change Analysis", 0, "grid"), error: null },
    { id: "d", run: null, error: "boom" },
    { id: "e", run: run("Co-change", 0), error: null },
  ];
  const { shown, empty } = foldEmpty(runs);
  expect(shown.map((r) => r.id)).toEqual(["b", "c", "d"]);
  expect(empty.map((r) => r.id)).toEqual(["a", "e"]);
});
