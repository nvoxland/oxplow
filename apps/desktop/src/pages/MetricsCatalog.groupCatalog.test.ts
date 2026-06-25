import { describe, expect, it } from "bun:test";

import type { MetricCatalogEntry } from "../api.js";
import { groupCatalog } from "./MetricsCatalog.js";

function entry(key: string, category: string | null, toggleable = false): MetricCatalogEntry {
  return {
    key,
    title: key,
    kind: "gauge",
    language: null,
    scope: "built-in",
    enabled: true,
    target: null,
    trigger: "auto",
    toggleable,
    category,
  };
}

describe("groupCatalog", () => {
  it("orders groups: code gauges, tests, static analysis, operational, then unknown", () => {
    const rows = [
      entry("agent.tokens.total", "operational"),
      entry("oxplow.rust.unsafe_blocks", "custom", true),
      entry("weird.metric", "mystery"),
      entry("oxplow.tests.passed", "testing"),
      entry("clippy.warnings", "static-quality"),
    ];
    const groups = groupCatalog(rows);
    expect(groups.map((g) => g.category)).toEqual([
      "custom",
      "testing",
      "static-quality",
      "operational",
      "mystery",
    ]);
  });

  it("keeps incoming order within a group and buckets nulls under Other", () => {
    const rows = [
      entry("b", null),
      entry("a", null),
      entry("z.gauge", "custom", true),
    ];
    const groups = groupCatalog(rows);
    // custom leads; null bucket last.
    expect(groups[0].category).toBe("custom");
    const other = groups.find((g) => g.category === null);
    expect(other?.entries.map((e) => e.key)).toEqual(["b", "a"]);
  });
});
