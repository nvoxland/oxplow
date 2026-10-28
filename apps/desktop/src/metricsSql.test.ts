import { describe, expect, test } from "bun:test";
import { seriesPoints } from "./metricsSql.js";

const reads = { datasets: [], tables: [], measures: [] };

describe("metrics read through SQL", () => {
  test("rows become points", () => {
    const points = seriesPoints({
      columns: ["capture_id", "captured_at", "value", "group", "branch", "provenance", "vcs_rev", "source"],
      rows: [[7, "2026-03-02T10:00:00.000000Z", 3.5, null, "main", "observed", "abc123", "builtin"]],
      truncated: false,
      reads,
      freshness: [],
    });
    expect(points).toEqual([
      {
        capture_id: 7,
        captured_at: "2026-03-02T10:00:00.000000Z",
        value: 3.5,
        group: null,
        branch: "main",
        provenance: "observed",
        vcs_rev: "abc123",
        source: "builtin",
      },
    ]);
  });
});
