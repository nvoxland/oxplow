import type { TabRef } from "../../tabs/tabState.js";
import { SETTINGS_AREAS } from "../../pages/settingsAreas.js";
import {
  backlogRef,
  gitDashboardRef,
  indexRef,
  settingsRef,
  tasksRef,
  uncommittedChangesRef,
} from "../../tabs/pageRefs.js";

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
 * Static directory of every top-level page. This is the single discovery
 * surface: the launcher (QuickOpen) shows it grouped by `category` in its
 * empty state and mixes it into ranked results when the user types. The
 * rail no longer renders a "Pages" section — users pin what they want via
 * Bookmarks instead. Entries are listed grouped by category so the flat
 * launcher order already reads top-to-bottom by section. Pure helper so it
 * can be unit-tested without mounting React. `backlogReadyCount` controls
 * the badge on "Backlog".
 */
export function computePagesDirectory(opts: { backlogReadyCount: number }): PageDirectoryEntry[] {
  return [
    // Labels are emoji-free — `PageKindIcon` resolves the leading
    // glyph from the entry's ref kind at render time.
    { id: "tasks", label: "Tasks", ref: tasksRef(), category: "Work", keywords: "dashboard" },
    {
      id: "backlog",
      label: "Backlog",
      ref: backlogRef(),
      category: "Work",
      badge: opts.backlogReadyCount > 0 ? opts.backlogReadyCount : undefined,
    },
    { id: "board", label: "Board", ref: indexRef("board"), category: "Work", keywords: "kanban columns state work items" },
    { id: "files", label: "Files", ref: indexRef("files"), category: "Code" },
    { id: "git-dashboard", label: "Git", ref: gitDashboardRef(), category: "Git" },
    { id: "git-history", label: "Git History", ref: indexRef("git-history"), category: "Git" },
    { id: "uncommitted-changes", label: "Uncommitted", ref: uncommittedChangesRef(), category: "Git" },
    { id: "local-history", label: "Local History", ref: indexRef("local-history"), category: "Activity" },
    { id: "comments", label: "Comments Dashboard", ref: indexRef("comments"), category: "Activity" },
    { id: "explore-data", label: "Explore Data", ref: indexRef("explore-data"), category: "Data", keywords: "sql query schema semantic layer lens" },
    { id: "metrics-recorded", label: "Metrics", ref: indexRef("metrics-recorded"), category: "Data", keywords: "recorded catalog" },
    { id: "terminal", label: "Terminal", ref: indexRef("terminal"), category: "System" },
    { id: "settings", label: "Settings", ref: settingsRef(), category: "System", keywords: "every setting config preferences" },
    // Each area is its own page, reachable by name.
    ...SETTINGS_AREAS.filter((a) => a.id !== "overview").map((a) => ({
      id: `settings-${a.id}`,
      label: `Settings: ${a.title}`,
      ref: settingsRef(a.id),
      category: "System" as const,
      keywords: `settings ${a.title} ${a.sections.map((s) => s.title).join(" ")}`,
    })),
  ];
}
