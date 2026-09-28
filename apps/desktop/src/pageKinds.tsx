/**
 * Page-kind scheme metadata.
 *
 * `kindForTabId` extracts the kind from a tab id like
 * `file:src/foo.ts` → `"file"`, `work_item:oxplow:tsk42` → `"work_item"`,
 * or `tasks` → `"tasks"` (literal index pages have no prefix).
 *
 * `pageKindIconComponent` / `PageKindIcon` map that kind to the
 * lucide-react icon used everywhere a page-kind label renders —
 * tabs, rail history/finished, backlinks list, markdown links.
 *
 * Scheme list lives next to this file's mapping; if you add a new
 * tab scheme in `tabs/pageRefs.ts`, add it here too.
 */
import {
  Activity,
  AlertCircle,
  Archive,
  BarChart3,
  BookOpen,
  CheckCheck,
  CheckSquare,
  Copy,
  ExternalLink,
  FileText,
  Folder,
  FolderTree,
  Gauge,
  GitBranch,
  GitCommit,
  GitCompare,
  GitMerge,
  Glasses,
  Database,
  History,
  Inbox,
  LayoutDashboard,
  Library,
  type LucideIcon,
  Plus,
  Settings,
  Terminal,
} from "lucide-react";
import type { ComponentProps, ReactElement } from "react";
import { parseRef } from "./refs/ref.js";

/**
 * Map a page-kind string to its icon component. Returns `null`
 * for unknown kinds so the caller can fall back to text-only.
 *
 * Accepts every value that can appear as a `TabRef.kind` or as the
 * literal id of an index page. Strings rather than a typed enum so
 * we can pass through `BacklinkEdge.target_kind` (loose string) and
 * arbitrary `tab.id` prefixes without an exhaustive switch eating
 * future kinds.
 */
export function pageKindIconComponent(kind: string): LucideIcon | null {
  switch (kind) {
    // Scheme-prefixed kinds (TabRef.kind).
    case "file":
      return FileText;
    case "dir":
    // The file tree's entries (`WorkspaceEntry.kind`) still say `directory`.
    case "directory":
      return Folder;
    case "diff":
    case "diff-view":
      return GitCompare;
    case "duplicate-block":
      return Copy;
    case "wiki":
      return BookOpen;
    case "wiki-freshness":
      return Gauge;
    case "work_item":
    // Comment anchors and the rail's finished list still say `task`.
    case "task":
      return CheckSquare;
    case "commit":
      return GitCommit;
    case "dashboard":
    case "custom-dashboard":
    case "dashboards":
      return LayoutDashboard;
    case "op-error":
      return AlertCircle;
    case "stream-settings":
    case "thread-settings":
    case "settings":
      return Settings;
    case "external-url":
      return ExternalLink;
    case "uncommitted-changes":
      return GitBranch;

    case "lens":
      return Glasses;
    case "explore-data":
      return Database;

    // Literal-id index pages (kind === id).
    case "agent":
      // The agent tab is always present and unambiguous; an icon
      // there just makes the tab wider without adding info.
      return null;
    case "tasks":
      return CheckSquare;
    case "done-work":
      return CheckCheck;
    case "backlog":
      return Inbox;
    case "archived":
    case "closed-threads":
      return Archive;
    case "wiki-index":
      return Library;
    case "files":
      return FolderTree;
    case "comments":
      return Inbox;
    case "local-history":
    case "local-history-full":
    case "local-history-by-commit-full":
      return History;
    case "git-history":
      return GitMerge;
    case "git-dashboard":
      return GitBranch;
    case "hook-events":
      return Activity;
    case "metrics-recorded":
      return BarChart3;
    case "metric":
      return Gauge;
    case "terminal":
      return Terminal;
    case "new-stream":
    case "new-task":
      return Plus;

    default:
      return null;
  }
}

export interface PageKindIconProps extends Omit<ComponentProps<LucideIcon>, "ref"> {
  kind: string;
  /**
   * Pixel size for the icon. Defaults to 14 — small enough to sit
   * inline next to text labels at the project's default font size.
   */
  size?: number;
}

/**
 * Render the icon for `kind`. Returns `null` for unknown kinds so
 * call sites can interleave `<PageKindIcon …/>` with a label and
 * unrecognized kinds simply lose the leading glyph rather than
 * crashing or rendering a question-mark placeholder.
 *
 * `aria-hidden` is set by default because the adjacent text label
 * is the accessible name; the icon is decorative.
 */
export function PageKindIcon({
  kind,
  size = 14,
  ...rest
}: PageKindIconProps): ReactElement | null {
  const Icon = pageKindIconComponent(kind);
  if (!Icon) return null;
  return <Icon aria-hidden size={size} {...rest} />;
}

/**
 * Human display label for a scheme kind — what the page chrome's
 * kind chip renders. Defaults to the kind string itself for the
 * many cases where the canonical kind reads fine ("file",
 * "diff", "tasks"); short-circuits the cases where the chrome
 * historically rendered a softer phrasing.
 */
export function pageKindLabel(kind: string): string {
  switch (kind) {
    case "wiki":
      return "wiki page";
    case "work_item":
      return "task";
    case "diff-view":
      return "diff";
    case "new-task":
      return "new task";
    case "new-stream":
      return "new stream";
    case "closed-threads":
      return "threads";
    case "lens":
      return "lens";
    case "explore-data":
      return "explore data";
    case "wiki-index":
      return "wiki";
    case "done-work":
      return "done work";
    case "local-history":
      return "local history";
    case "local-history-full":
      return "all snapshots";
    case "local-history-by-commit-full":
      return "all commits";
    case "git-history":
      return "git history";
    case "git-dashboard":
      return "git";
    case "hook-events":
      return "hook events";
    case "duplicate-block":
      return "duplicate";
    case "external-url":
      return "external link";
    case "uncommitted-changes":
      return "uncommitted";
    case "stream-settings":
      return "stream settings";
    case "thread-settings":
      return "thread settings";
    case "op-error":
      return "error";
    case "metrics-recorded":
      return "metrics";
    case "custom-dashboard":
      return "dashboard";
    default:
      return kind;
  }
}

/**
 * Index-page ids that double as their own kind — these tab ids
 * carry no scheme prefix and the whole id is the kind label.
 */
const INDEX_KINDS = new Set<string>([
  "agent",
  "tasks",
  "done-work",
  "backlog",
  "archived",
  "wiki-index",
  "files",
  "comments",
  "local-history",
  "local-history-full",
  "local-history-by-commit-full",
  "git-history",
  "git-dashboard",
  "hook-events",
  "terminal",
  "settings",
  "new-stream",
  "new-task",
  "closed-threads",
  "uncommitted-changes",
  "metrics-recorded",
  "dashboards",
  "explore-data",
]);

/**
 * The kind of a tab id. A canonical ref (`work_item:oxplow:tsk42`,
 * `file:src/a.rs@git:HEAD`) yields its kind through the grammar, so the
 * `:` inside an id never splits it. Index pages (`tasks`) are their own
 * kind. Shell routes whose scheme is not a valid ref kind
 * (`external-url:…`, `diff-view:…`, `uncommitted-changes:…`) yield the
 * text before the first `:` until they move to `page:` ids (P1.3b).
 */
export function kindForTabId(tabId: string): string {
  if (INDEX_KINDS.has(tabId)) return tabId;
  const canonical = parseRef(tabId);
  if (canonical) return canonical.kind;
  const idx = tabId.indexOf(":");
  if (idx === -1) return tabId;
  return tabId.slice(0, idx);
}
