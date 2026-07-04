// Shared category grouping for the metric pages (Catalog + Recorded Metrics):
// one display order + label set, and a generic group-by-category so both pages
// organize metrics the same way. Pure — no React, unit-testable.

/** Display order: code gauges (the toggleable opt-in compute) lead; the
 *  always-on producer families follow. Unknown categories fall to the end. */
export const CATEGORY_ORDER = [
  "custom",
  "testing",
  "coverage",
  "static-quality",
  "operational",
] as const;

const CATEGORY_LABEL: Record<string, string> = {
  custom: "Code gauges",
  testing: "Tests",
  coverage: "Coverage",
  "static-quality": "Static analysis",
  operational: "Operational",
};

/** Human label for a category key; "Other" for null, raw key for unknown. */
export function categoryLabel(cat: string | null): string {
  if (!cat) return "Other";
  return CATEGORY_LABEL[cat] ?? cat;
}

function categoryOrder(cat: string | null): number {
  const i = CATEGORY_ORDER.indexOf((cat ?? "") as (typeof CATEGORY_ORDER)[number]);
  return i === -1 ? CATEGORY_ORDER.length : i;
}

/** Group rows by their category in display order. Pure — entries keep their
 *  incoming order within each group. `getCategory` reads the category off each
 *  row (so it works for both `MetricCatalogEntry` and `{def}` rows). */
export function groupByCategory<T>(
  rows: T[],
  getCategory: (row: T) => string | null,
): Array<{ category: string | null; entries: T[] }> {
  const byCat = new Map<string | null, T[]>();
  for (const r of rows) {
    const cat = getCategory(r) ?? null;
    const list = byCat.get(cat);
    if (list) list.push(r);
    else byCat.set(cat, [r]);
  }
  return [...byCat.entries()]
    .map(([category, entries]) => ({ category, entries }))
    .sort((a, b) => categoryOrder(a.category) - categoryOrder(b.category));
}

// Display labels for language slugs (as carried on `MetricCatalogEntry.language`
// — the built-in gauges use `""`/null for language-agnostic, else a lowercase
// slug). Unknown slugs fall back to a Capitalized form.
const LANGUAGE_LABEL: Record<string, string> = {
  rust: "Rust",
  typescript: "TypeScript",
  javascript: "JavaScript",
  python: "Python",
  go: "Go",
  csharp: "C#",
  cpp: "C++",
  c: "C",
  java: "Java",
  kotlin: "Kotlin",
  ruby: "Ruby",
  php: "PHP",
  swift: "Swift",
  clojure: "Clojure",
};

/** Human label for a language slug; the language-agnostic bucket (empty/null)
 *  is "General". Unknown slugs are Capitalized. */
export function languageLabel(lang: string | null): string {
  if (!lang || lang.trim() === "") return "General";
  return LANGUAGE_LABEL[lang] ?? lang.charAt(0).toUpperCase() + lang.slice(1);
}

/** Sub-group rows by language slug. Pure — entries keep their incoming order.
 *  The language-agnostic bucket (null / `""`) collapses into one "General" group
 *  that sorts first; named languages follow, ordered by display label. Used to
 *  break the flat "Static analysis" category into per-language sections. */
export function groupByLanguage<T>(
  rows: T[],
  getLanguage: (row: T) => string | null,
): Array<{ language: string | null; label: string; entries: T[] }> {
  const byLang = new Map<string | null, T[]>();
  for (const r of rows) {
    const raw = getLanguage(r);
    const lang = raw && raw.trim() !== "" ? raw : null;
    const list = byLang.get(lang);
    if (list) list.push(r);
    else byLang.set(lang, [r]);
  }
  return [...byLang.entries()]
    .map(([language, entries]) => ({ language, label: languageLabel(language), entries }))
    .sort((a, b) => {
      // "General" (the language-agnostic bucket) always leads; the rest by label.
      if (a.language === null) return b.language === null ? 0 : -1;
      if (b.language === null) return 1;
      return a.label.localeCompare(b.label);
    });
}
