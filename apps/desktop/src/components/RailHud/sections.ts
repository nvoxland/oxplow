import type { TabRef } from "../../tabs/tabState.js";
import { SETTINGS_AREAS } from "../../pages/settingsAreas.js";
import { settingsRef } from "../../tabs/pageRefs.js";

/**
 * Sections the launcher's "start menu" empty state groups pages under.
 * `PAGE_CATEGORY_ORDER` is the render order for those headings.
 */
export type PageCategory = "Work" | "Code" | "Git" | "Activity" | "Knowledge" | "Data" | "Lenses" | "System";

export const PAGE_CATEGORY_ORDER: readonly PageCategory[] = [
  "Work",
  "Code",
  "Git",
  "Activity",
  "Knowledge",
  "Data",
  "Lenses",
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

/**
 * The Settings areas as launcher entries, each reachable by name. Every
 * other page in the launcher is a declared page (`launcherPages`); these
 * go when the Settings areas are pages of their own.
 */
export function settingsAreaEntries(): PageDirectoryEntry[] {
  return SETTINGS_AREAS.filter((a) => a.id !== "overview").map((a) => ({
    id: `settings-${a.id}`,
    label: `Settings: ${a.title}`,
    ref: settingsRef(a.id),
    category: "System" as const,
    keywords: `settings ${a.title} ${a.sections.map((s) => s.title).join(" ")}`,
  }));
}
