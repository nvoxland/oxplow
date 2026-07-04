import { describe, expect, it } from "bun:test";

import { groupByLanguage, languageLabel } from "./metricCategories.js";

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
