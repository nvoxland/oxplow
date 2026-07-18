import type { MetricCatalogEntry } from "../../api.js";
import { fuzzyMatches } from "../../fuzzy-match.js";
import { type MetricSection, buildMetricSections } from "../../pages/metricCategories.js";

// Pure logic behind the dashboard's add-metric picker (tsk145). React-free so
// the sectioning + search are unit-testable without mounting the panel.

/**
 * The picker's sections for a search query.
 *
 * Sectioning delegates to {@link buildMetricSections} — the project's **one**
 * metric sectioning rule (see `pages/metricCategories.ts`). An earlier revision
 * of this picker carried its own category table and consequently grouped
 * metrics differently from the Recorded Metrics page: that is exactly the drift
 * `buildMetricSections`'s doc comment warns about, so do not reintroduce a local
 * copy here. Notably it splits `static-quality` into per-language sections,
 * which is also what keeps the list navigable.
 *
 * Filtering is the launcher's `fuzzyMatches` (subsequence, so terse typing
 * works) applied to both the title and the metric key, case-insensitively.
 * Sections left empty by the filter are dropped.
 */
export function pickerSections(
  catalog: MetricCatalogEntry[],
  query: string,
): Array<MetricSection<MetricCatalogEntry>> {
  const needle = query.trim().toLowerCase();
  const matched = needle
    ? catalog.filter(
        (e) => fuzzyMatches(e.title.toLowerCase(), needle) || fuzzyMatches(e.key.toLowerCase(), needle),
      )
    : catalog;
  return buildMetricSections(
    matched,
    (e) => e.category,
    (e) => e.language,
    (e) => e.title,
  ).filter((s) => s.entries.length > 0);
}

/** The sections' entries in render order — the list the keyboard cursor walks,
 *  so the highlighted row and the rendered row can't drift apart. */
export function flattenPickerRows(
  sections: Array<MetricSection<MetricCatalogEntry>>,
): MetricCatalogEntry[] {
  return sections.flatMap((s) => s.entries);
}
