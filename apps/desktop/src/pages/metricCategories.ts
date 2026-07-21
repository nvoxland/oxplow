// Shared category grouping for the metric pages (Catalog + Metrics):
// one display order + label set, and a generic group-by-category so both pages
// organize metrics the same way. Pure — no React, unit-testable.

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

/** Group rows by their category, groups in first-seen order (the section list
 *  alphabetizes at the end — see `buildMetricSections`). Pure — entries keep
 *  their incoming order within each group. `getCategory` reads the category off
 *  each row (so it works for both `MetricCatalogEntry` and `{def}` rows). */
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
  return [...byCat.entries()].map(([category, entries]) => ({ category, entries }));
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

/** One rendered section on a metric page: a stable `key` (React key + testid
 *  suffix) and the display `label` above its rows. */
export type MetricSection<T> = {
  key: string;
  label: string;
  entries: T[];
};

/**
 * The section list Metrics renders. Sections ALPHABETICAL
 * by label, **except that `static-quality`** gets no section of its own: its real
 * top-level division is by language, so each language is promoted to a top-level
 * section (a peer of Tests / Coverage / Operational) and the language-agnostic
 * analysers fall under "General".
 *
 * Kept as the one sectioning rule (tsk81) for the same reason the Rust specs
 * read their language off the gauge: two copies of this rule drift, and two
 * surfaces then group the same metrics differently. Pure — rows sort
 * alphabetically within each section (tsk118), same locale collation as the
 * section labels.
 */
export function buildMetricSections<T>(
  rows: T[],
  getCategory: (row: T) => string | null,
  getLanguage: (row: T) => string | null,
  getLabel: (row: T) => string,
): Array<MetricSection<T>> {
  const out: Array<MetricSection<T>> = [];
  for (const group of groupByCategory(rows, getCategory)) {
    if (group.category === "static-quality") {
      for (const lang of groupByLanguage(group.entries, getLanguage)) {
        out.push({
          // `static-` prefixed so a language section can't collide with a
          // category key.
          key: `static-${lang.language ?? "general"}`,
          label: lang.label,
          entries: lang.entries,
        });
      }
    } else {
      out.push({
        key: group.category ?? "other",
        label: categoryLabel(group.category),
        entries: group.entries,
      });
    }
  }
  // ALPHABETICAL by display label, uniformly (tsk116): language sections
  // interleave with category sections as equals, and a brand-new language or
  // category slots itself in with zero curation — there is no hand-maintained
  // order list to forget to update. Locale-aware compare, matching the
  // locale-formatted values the rows show (tsk114). Rows inside each section
  // sort the same way (tsk118).
  for (const section of out) {
    section.entries.sort((a, b) => getLabel(a).localeCompare(getLabel(b)));
  }
  return out.sort((a, b) => a.label.localeCompare(b.label));
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
