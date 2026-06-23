import { describe, expect, test } from "bun:test";

import type { EffortObservation } from "../api.js";
import {
  type StaticAnalysisPayload,
  analysisCounts,
  analysisHeadline,
  clusterTestRuns,
  groupFindingsByFile,
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
  test("merges same-time stacks of one test:collect into a single iteration", () => {
    // Rust + frontend runs seconds apart → one logical pass.
    const iters = clusterTestRuns([
      testRun("2026-06-23T10:00:00Z", 300, 0),
      testRun("2026-06-23T10:00:08Z", 419, 0),
    ]);
    expect(iters).toHaveLength(1);
    expect(iters[0]).toEqual({ at: "2026-06-23T10:00:00Z", passed: 719, failed: 0 });
  });

  test("groups by snapshot when present — same code state is one iteration", () => {
    // Two test commands against snapshot 5 (same code), then an edit → snapshot
    // 7. Exact grouping: 2 iterations, not 3, regardless of timing.
    const iters = clusterTestRuns([
      testRun("2026-06-23T10:06:00Z", 720, 0, 7),
      testRun("2026-06-23T10:01:30Z", 419, 0, 5), // > 90s after the first run…
      testRun("2026-06-23T10:00:00Z", 300, 3, 5), // …but same snapshot → merged
    ]);
    expect(iters.map((i) => [i.passed, i.failed])).toEqual([
      [719, 3], // snapshot 5: 300+419 passed, 3 failed
      [720, 0], // snapshot 7
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
