/**
 * Page-kind scheme metadata.
 *
 * `kindForTabId` extracts the kind from a tab id like
 * `file:src/foo.ts` → `"file"`, `work_item:oxplow:tsk42` → `"work_item"`,
 * or `page:tasks` → `"tasks"` (a shell route).
 *
 * `pageKindIconComponent` / `PageKindIcon` map that kind to the
 * lucide-react icon used everywhere a page-kind label renders —
 * tabs, rail history/finished, backlinks list, markdown links.
 *
 * If you add a new page kind in `tabs/pageRefs.ts`, add its icon and
 * label here too.
 */
import {
  Activity,
  Bell,
  Archive,
  BarChart3,
  BookOpen,
  Columns3,
  TriangleAlert,
  ListTree,
  Braces,
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
import { refKindIcon, refKindInfo } from "./refKinds.js";
import { pageKindOf } from "./tabs/pageRefs.js";
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
    // A snapshot, an effort and a turn open as their diff (P2.11).
    case "snapshot":
    case "effort":
    case "turn":
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
    case "alerts":
      return Bell;
    case "stream-settings":
    case "thread-settings":
    case "settings":
      return Settings;
    case "external-url":
      return ExternalLink;
    case "uncommitted-changes":
      return GitBranch;

    case "lens":
    case "ext-page":
      return Glasses;
    case "explore-data":
      return Database;
    case "catalog":
      return BookOpen;
    case "board":
      return Columns3;
    case "problems":
      return TriangleAlert;
    case "symbols":
      return ListTree;
    case "symbol":
      return Braces;

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

    // An extension's ref kind (`ref_kinds:`, P8.D7) draws its declared icon.
    default:
      return refKindIcon(kind);
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
    case "alerts":
      return "alerts";
    case "metrics-recorded":
      return "metrics";
    case "custom-dashboard":
      return "dashboard";
    default:
      return refKindInfo(kind)?.label ?? kind;
  }
}

/**
 * The kind a tab id renders as — an entity ref's kind
 * (`work_item:oxplow:tsk42` → `work_item`, `file:src/a.rs@git:HEAD` →
 * `file`) or a route's name (`page:diff-view?effort=eff9` → `diff-view`).
 * Text that is not a tab id comes back unchanged so a caller can still
 * label it. The chrome uses this for icons and kind chips.
 */
export function kindForTabId(tabId: string): string {
  const kind = pageKindOf(tabId);
  if (kind) return kind;
  // A ref of a kind with no page of its own (`finding:…`) still has an icon.
  return parseRef(tabId)?.kind ?? tabId;
}
