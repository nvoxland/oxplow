import { describe, expect, it } from "bun:test";

import type { MetricCatalogEntry } from "../../api.js";
import { flattenPickerRows, pickerSections } from "./metricPicker.js";

function entry(
  key: string,
  title: string,
  category: string | null,
  language: string | null = null,
): MetricCatalogEntry {
  return {
    key,
    title,
    kind: "gauge",
    language,
    scope: "built-in",
    enabled: true,
    target: null,
    trigger: "onCapture",
    toggleable: false,
    category,
  };
}

const CATALOG: MetricCatalogEntry[] = [
  entry("oxplow.coverage.line_pct", "Line coverage", "coverage"),
  entry("oxplow.tests.failed", "Distinct tests failed", "testing"),
  entry("oxplow.tokens.total", "Tokens", "operational"),
  entry("rust.unsafe", "unsafe blocks", "static-quality", "rust"),
  entry("ts.any", "explicit any", "static-quality", "typescript"),
  entry("gen.todo", "TODO / FIXME markers", "static-quality", null),
  entry("my.gauge", "My gauge", "custom"),
];

describe("pickerSections", () => {
  it("uses the canonical sectioning — Coverage/Tests labels and static-quality split by language", () => {
    const labels = pickerSections(CATALOG, "").map((s) => s.label);
    // Canonical labels (not "Testing"/"Static quality"), and static-quality
    // becomes per-language sections that sit as peers, all alphabetical.
    expect(labels).toContain("Coverage");
    expect(labels).toContain("Tests");
    expect(labels).toContain("Code gauges");
    expect(labels).toContain("General");
    expect(labels).toContain("Rust");
    expect(labels).toContain("TypeScript");
    expect(labels).not.toContain("Static quality");
    expect(labels).toEqual([...labels].sort((a, b) => a.localeCompare(b)));
  });

  it("filters entries by a fuzzy match on the title", () => {
    const sections = pickerSections(CATALOG, "unsf");
    expect(sections.map((s) => s.label)).toEqual(["Rust"]);
    expect(sections[0]?.entries.map((e) => e.title)).toEqual(["unsafe blocks"]);
  });

  it("also matches on the metric key, so a namespaced search works", () => {
    const sections = pickerSections(CATALOG, "oxplow.tokens");
    expect(sections.flatMap((s) => s.entries).map((e) => e.key)).toEqual(["oxplow.tokens.total"]);
  });

  it("is case-insensitive", () => {
    expect(pickerSections(CATALOG, "LINE COV").flatMap((s) => s.entries)).toHaveLength(1);
  });

  it("drops sections with no surviving entries, and returns none when nothing matches", () => {
    expect(pickerSections(CATALOG, "zzzznope")).toEqual([]);
  });
});

describe("flattenPickerRows", () => {
  it("flattens sections into the keyboard-navigable row order", () => {
    const sections = pickerSections(CATALOG, "cover");
    const rows = flattenPickerRows(sections);
    expect(rows.map((r) => r.key)).toEqual(["oxplow.coverage.line_pct"]);
  });

  it("preserves section order across the flattened list", () => {
    const rows = flattenPickerRows(pickerSections(CATALOG, ""));
    const sectionsInOrder = pickerSections(CATALOG, "").flatMap((s) => s.entries.map((e) => e.key));
    expect(rows.map((r) => r.key)).toEqual(sectionsInOrder);
  });
});
