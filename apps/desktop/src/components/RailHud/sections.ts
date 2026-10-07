import type { TabRef } from "../../tabs/tabState.js";
import {
  backlogRef,
  closedThreadsRef,
  dashboardRef,
  dashboardsRef,
  doneWorkRef,
  gitDashboardRef,
  indexRef,
  tasksRef,
  uncommittedChangesRef,
  symbolsRef,
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
    { id: "done-work", label: "Done Work", ref: doneWorkRef(), category: "Work" },
    {
      id: "backlog",
      label: "Backlog",
      ref: backlogRef(),
      category: "Work",
      badge: opts.backlogReadyCount > 0 ? opts.backlogReadyCount : undefined,
    },
    { id: "board", label: "Board", ref: indexRef("board"), category: "Work", keywords: "kanban columns state work items" },
    { id: "files", label: "Files", ref: indexRef("files"), category: "Code" },
    { id: "problems", label: "Problems", ref: indexRef("problems"), category: "Code", keywords: "diagnostics errors warnings lsp compiler" },
    { id: "symbols", label: "Symbols", ref: symbolsRef(), category: "Code", keywords: "outline functions classes definitions lsp" },
    { id: "git-dashboard", label: "Git", ref: gitDashboardRef(), category: "Git" },
    { id: "git-history", label: "Git History", ref: indexRef("git-history"), category: "Git" },
    { id: "uncommitted-changes", label: "Uncommitted", ref: uncommittedChangesRef(), category: "Git" },
    { id: "local-history", label: "Local History", ref: indexRef("local-history"), category: "Activity" },
    { id: "hook-events", label: "Hook Events", ref: indexRef("hook-events"), category: "Activity" },
    { id: "comments", label: "Comments Dashboard", ref: indexRef("comments"), category: "Activity" },
    { id: "dashboard-visits", label: "Go To", ref: dashboardRef("visits"), category: "Activity" },
    { id: "wiki-index", label: "Wiki", ref: indexRef("wiki-index"), category: "Knowledge" },
    { id: "explore-data", label: "Explore Data", ref: indexRef("explore-data"), category: "Data", keywords: "sql query schema semantic layer lens" },
    { id: "metrics-recorded", label: "Metrics", ref: indexRef("metrics-recorded"), category: "Data", keywords: "recorded catalog" },
    { id: "dashboards", label: "Dashboards", ref: dashboardsRef(), category: "Data", keywords: "custom metric tiles lens" },
    { id: "catalog", label: "Catalog", ref: indexRef("catalog"), category: "System", keywords: "ask prompts questions what can i help data config" },
    { id: "terminal", label: "Terminal", ref: indexRef("terminal"), category: "System" },
    { id: "closed-threads", label: "Closed Threads", ref: closedThreadsRef(), category: "System" },
    { id: "settings", label: "Settings", ref: indexRef("settings"), category: "System" },
  ];
}
