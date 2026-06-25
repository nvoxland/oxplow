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
