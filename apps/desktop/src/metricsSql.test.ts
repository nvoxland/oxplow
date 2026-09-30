import { describe, expect, test } from "bun:test";
import { catalogEntries, metricSeriesSql, seriesPoints } from "./metricsSql.js";

const reads = { models: [], tables: [], measures: [] };

describe("metrics read through SQL", () => {
  test("a metric's captures, grouped and bounded", () => {
    expect(metricSeriesSql("oxplow.coverage.abs_pct")).toBe(
      [
        `SELECT g.capture_id, g.bucket AS captured_at, MEASURE('oxplow.coverage.abs_pct') AS value, NULL AS "group",`,
        "       c.branch, c.provenance, c.closest_vcs_rev AS vcs_rev, c.source",
        "FROM metric_grid('capture') g LEFT JOIN v_capture c ON c.id = g.capture_id",
        "WHERE MEASURE('oxplow.coverage.abs_pct') IS NOT NULL",
        "ORDER BY g.bucket DESC",
      ].join("\n"),
    );
    const grouped = metricSeriesSql("it's", "zone", true);
    expect(grouped).toContain("MEASURE('it''s')");
    expect(grouped).toContain(`metric_grid('capture', 'zone')`);
    expect(grouped).toContain(`g."zone" AS "group"`);
    expect(grouped).toContain("AND g.bucket >= ?1 AND g.bucket <= ?2");
  });

  test("rows become points and catalog entries", () => {
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
    const [entry] = catalogEntries({
      columns: ["key", "title", "kind", "language", "scope", "enabled", "target", "trigger", "toggleable", "category"],
      rows: [["a", "A", "gauge", null, "built-in", 1, null, "auto", 0, "testing"]],
      truncated: false,
      reads,
      freshness: [],
    });
    expect(entry.enabled).toBe(true);
    expect(entry.toggleable).toBe(false);
  });
});
