import type { TabRef } from "../../tabs/tabState.js";

/**
 * Sections the launcher's "start menu" empty state groups pages under.
 * `PAGE_CATEGORY_ORDER` is the render order for those headings.
 */
export type PageCategory = "Work" | "Code" | "Git" | "Activity" | "Knowledge" | "Data" | "Dashboards" | "System";

export const PAGE_CATEGORY_ORDER: readonly PageCategory[] = [
  "Work",
  "Code",
  "Git",
  "Activity",
  "Knowledge",
  "Data",
  "Dashboards",
  "System",
];

export interface PageDirectoryEntry {
  id: string;
  label: string;
  ref: TabRef;
  category: PageCategory;
  badge?: number;
  /** Extra search terms the launcher fuzzy-matches beyond label/id — for
   *  a page whose name doesn't contain a word users reach for (e.g. the
   *  "Tasks" page found by typing "dashboard"). */
  keywords?: string;
}
