import { describe, expect, it } from "bun:test";

import { buildMetricSections, groupByLanguage, languageLabel } from "./metricCategories.js";

describe("languageLabel", () => {
  it("maps known slugs to their display names", () => {
    expect(languageLabel("rust")).toBe("Rust");
    expect(languageLabel("typescript")).toBe("TypeScript");
    expect(languageLabel("csharp")).toBe("C#");
    expect(languageLabel("clojure")).toBe("Clojure");
  });

  it("capitalizes an unknown slug and buckets empty/null under General", () => {
    expect(languageLabel("elixir")).toBe("Elixir");
    expect(languageLabel("")).toBe("General");
    expect(languageLabel(null)).toBe("General");
  });
});

describe("groupByLanguage", () => {
  const row = (key: string, language: string | null) => ({ key, language });

  it("puts the General bucket first, then languages by display label", () => {
    const rows = [
      row("oxplow.ts.any_usage", "typescript"),
      row("oxplow.todos", ""),
      row("oxplow.rust.unsafe_blocks", "rust"),
      row("oxplow.analysis.errors", null),
      row("oxplow.csharp.empty_catch", "csharp"),
    ];
    const groups = groupByLanguage(rows, (r) => r.language);
    // General (null/"" collapsed) leads; the rest ordered by label: C#, Rust, TypeScript.
    expect(groups.map((g) => g.label)).toEqual(["General", "C#", "Rust", "TypeScript"]);
  });

  it("collapses null and empty-string into one General bucket, keeping incoming order", () => {
    const rows = [row("a", null), row("b", ""), row("c", "rust")];
    const groups = groupByLanguage(rows, (r) => r.language);
    const general = groups.find((g) => g.language === null);
    expect(general?.entries.map((e) => e.key)).toEqual(["a", "b"]);
    expect(groups.at(-1)?.label).toBe("Rust");
  });
});

describe("buildMetricSections", () => {
  const row = (key: string, category: string | null, language: string | null = null) => ({
    key,
    category,
    language,
  });
  const sections = <T extends { category: string | null; language: string | null }>(rows: T[]) =>
    buildMetricSections(
      rows,
      (r) => r.category,
      (r) => r.language,
    );

  it("explodes static analysis into one top-level section per language", () => {
    // The point of the split: Rust / TypeScript / C# become peers of Tests and
    // Operational, instead of one cross-language "Static analysis" pile.
    const built = sections([
      row("oxplow.tests.failed", "testing"),
      row("oxplow.ts.any_usage", "static-quality", "typescript"),
      row("oxplow.rust.unsafe_blocks", "static-quality", "rust"),
      row("oxplow.analysis.errors", "static-quality", null),
      row("oxplow.agent.turns", "operational"),
    ]);
    expect(built.map((s) => s.label)).toEqual([
      "Tests",
      // Static analysis' own sections, General first then languages by label.
      "General",
      "Rust",
      "TypeScript",
      "Operational",
    ]);
    expect(built.find((s) => s.label === "Rust")?.entries.map((e) => e.key)).toEqual([
      "oxplow.rust.unsafe_blocks",
    ]);
  });

  it("keeps every other category as a single section, in display order", () => {
    const built = sections([
      row("c", "operational"),
      row("a", "custom"),
      row("b", "coverage"),
    ]);
    expect(built.map((s) => s.label)).toEqual(["Code gauges", "Coverage", "Operational"]);
    expect(built.map((s) => s.key)).toEqual(["custom", "coverage", "operational"]);
  });

  it("gives each section a key distinct from any category key", () => {
    // Keys drive React keys + testids; a language section must not collide with
    // a category section of the same name.
    // Ordered by CATEGORY_ORDER, so testing leads static-quality's languages.
    const built = sections([row("x", "static-quality", "rust"), row("y", "testing")]);
    expect(built.map((s) => s.key)).toEqual(["testing", "static-rust"]);
  });

  it("has no static-analysis section at all when nothing is in that category", () => {
    const built = sections([row("a", "testing")]);
    expect(built.map((s) => s.key)).toEqual(["testing"]);
  });

  it("sorts an unknown category last and buckets nulls under Other", () => {
    const built = sections([
      row("agent.tokens.total", "operational"),
      row("oxplow.rust.unsafe_blocks", "custom"),
      row("weird.metric", "mystery"),
      row("orphan", null),
      row("oxplow.tests.passed", "testing"),
    ]);
    expect(built.map((s) => s.label)).toEqual([
      "Code gauges",
      "Tests",
      "Operational",
      // Unknown category renders under its raw key; null under "Other".
      "mystery",
      "Other",
    ]);
  });

  it("keeps incoming order within a section", () => {
    const built = sections([row("b", null), row("a", null)]);
    expect(built[0]?.entries.map((e) => e.key)).toEqual(["b", "a"]);
  });
});
