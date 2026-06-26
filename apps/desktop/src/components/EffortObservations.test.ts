import { describe, expect, test } from "bun:test";

import type { EffortObservation } from "../api.js";
import {
  type StaticAnalysisPayload,
  analysisCounts,
  analysisHeadline,
  buildMultiRunTree,
  clusterTestRuns,
  groupFindingsByFile,
  runsLabel,
} from "./EffortObservations.js";

const testRun = (
  at: string,
  passed: number,
  failed: number,
  snapshot?: number,
): EffortObservation =>
  ({
    created_at: at,
    local_snapshot_id: snapshot ?? null,
    payload_json: JSON.stringify({ command: "x", passed, failed, total: passed + failed }),
  }) as unknown as EffortObservation;

describe("clusterTestRuns (TDD iteration timeline)", () => {
  test("one iteration per run — each observation is its own bar", () => {
    // Two test commands seconds apart are two runs, not one. Snapshots are
    // coarse (effort boundaries), so we don't fold runs together by code state.
    const iters = clusterTestRuns([
      testRun("2026-06-23T10:00:00Z", 300, 0),
      testRun("2026-06-23T10:00:08Z", 419, 0),
    ]);
    expect(iters.map((i) => [i.passed, i.failed])).toEqual([
      [300, 0],
      [419, 0],
    ]);
  });

  test("does not group by snapshot — every run is a distinct iteration", () => {
    // Three runs sharing two snapshots → still three iterations, oldest-first.
    const iters = clusterTestRuns([
      testRun("2026-06-23T10:06:00Z", 720, 0, 7),
      testRun("2026-06-23T10:01:30Z", 419, 0, 5),
      testRun("2026-06-23T10:00:00Z", 300, 3, 5),
    ]);
    expect(iters.map((i) => [i.passed, i.failed])).toEqual([
      [300, 3],
      [419, 0],
      [720, 0],
    ]);
  });

  test("splits time-separated runs into iterations, oldest-first (red→green)", () => {
    // Given newest-first input (as the API returns), the strip reads oldest→newest.
    const iters = clusterTestRuns([
      testRun("2026-06-23T10:10:00Z", 720, 0),
      testRun("2026-06-23T10:05:00Z", 717, 3),
      testRun("2026-06-23T10:00:00Z", 714, 6),
    ]);
    expect(iters.map((i) => [i.passed, i.failed])).toEqual([
      [714, 6],
      [717, 3],
      [720, 0],
    ]);
  });
});

const suiteRun = (
  at: string,
  cases: Array<{ classname: string; name: string; status: "passed" | "failed" | "skipped"; timeMs?: number }>,
  suiteName = "crate",
): EffortObservation =>
  ({
    created_at: at,
    local_snapshot_id: null,
    payload_json: JSON.stringify({
      command: "x",
      passed: cases.filter((c) => c.status === "passed").length,
      failed: cases.filter((c) => c.status === "failed").length,
      total: cases.length,
      suites: [{ name: suiteName, cases }],
    }),
  }) as unknown as EffortObservation;

describe("runsLabel (run-count header, zero called out)", () => {
  test("zero runs is called out explicitly, not hidden", () => {
    expect(runsLabel(0)).toBe("no runs");
  });
  test("singular vs plural", () => {
    expect(runsLabel(1)).toBe("1 run");
    expect(runsLabel(4)).toBe("4 runs");
  });
});

describe("buildMultiRunTree (per-run pass/fail per node + per-leaf statuses)", () => {
  const r1 = suiteRun("2026-06-23T10:00:00Z", [
    { classname: "crate", name: "mod::test_a", status: "failed" },
    { classname: "crate", name: "mod::test_b", status: "passed" },
  ]);
  const r2 = suiteRun("2026-06-23T10:05:00Z", [
    { classname: "crate", name: "mod::test_a", status: "passed" },
    { classname: "crate", name: "mod::test_b", status: "passed" },
    { classname: "crate", name: "mod::test_c", status: "passed" },
  ]);

  test("orders runs oldest-first regardless of input order; node counts per run", () => {
    const tree = buildMultiRunTree([r2, r1]); // newest-first input (as the API returns)
    const root = tree[0];
    expect(root.label).toBe("crate");
    expect(root.counts).toEqual([
      { passed: 1, failed: 1, skipped: 0 },
      { passed: 3, failed: 0, skipped: 0 },
    ]);
  });

  test("each leaf carries its per-run status; a run it didn't appear in is null", () => {
    const tree = buildMultiRunTree([r1, r2]);
    const mod = tree[0].children[0];
    expect(mod.label).toBe("mod");
    const byName = Object.fromEntries(mod.leaves.map((l) => [l.name, l]));
    expect(byName.test_a.statuses).toEqual(["failed", "passed"]);
    expect(byName.test_a.everFailed).toBe(true);
    expect(byName.test_a.finalStatus).toBe("passed");
    expect(byName.test_b.statuses).toEqual(["passed", "passed"]);
    expect(byName.test_c.statuses).toEqual([null, "passed"]);
  });

  test("counts array has exactly one entry per run", () => {
    expect(buildMultiRunTree([r1, r2])[0].counts).toHaveLength(2);
    expect(buildMultiRunTree([r1])[0].counts).toHaveLength(1);
  });
});

describe("static-analysis payload helpers", () => {
  test("analysisCounts prefers explicit counts", () => {
    const p: StaticAnalysisPayload = {
      errorCount: 1,
      warningCount: 3,
      // info/note absent → default 0
      findings: [],
    };
    expect(analysisCounts(p)).toEqual({ error: 1, warning: 3, info: 0, note: 0 });
  });

  test("analysisCounts derives from findings when counts absent", () => {
    const p: StaticAnalysisPayload = {
      findings: [
        { path: "a.rs", severity: "error", message: "x" },
        { path: "a.rs", severity: "warning", message: "y" },
        { path: "b.rs", severity: "warning", message: "z" },
        { path: "b.rs", severity: "note", message: "n" },
      ],
    };
    expect(analysisCounts(p)).toEqual({ error: 1, warning: 2, info: 0, note: 1 });
  });

  test("analysisHeadline shows errors+warnings always, info/note only when present", () => {
    expect(analysisHeadline({ error: 0, warning: 0, info: 0, note: 0 })).toBe("0 errors, 0 warnings");
    expect(analysisHeadline({ error: 1, warning: 1, info: 0, note: 0 })).toBe("1 error, 1 warning");
    expect(analysisHeadline({ error: 0, warning: 2, info: 1, note: 3 })).toBe(
      "0 errors, 2 warnings, 1 info, 3 notes",
    );
  });

  test("groupFindingsByFile groups preserving first-seen order", () => {
    const grouped = groupFindingsByFile([
      { path: "b.rs", severity: "error", message: "1" },
      { path: "a.rs", severity: "warning", message: "2" },
      { path: "b.rs", severity: "warning", message: "3" },
    ]);
    expect(grouped.map((g) => g.path)).toEqual(["b.rs", "a.rs"]);
    expect(grouped[0].findings).toHaveLength(2);
    expect(grouped[1].findings).toHaveLength(1);
  });
});
