/**
 * Page-kind scheme metadata.
 *
 * `kindForTabId` extracts the kind from a tab id like
 * `file:src/foo.ts` → `"file"`, `work_item:oxplow:tsk42` → `"work_item"`,
 * or, for a declared page, its page id: `page:tasks` → `"tasks"`,
 * `page:gh/open-prs` → `"gh/open-prs"` (what visits and bookmarks
 * record, `recordedPageKind`).
 *
 * `pageKindIconComponent` / `PageKindIcon` map that kind to the
 * lucide-react icon used everywhere a page-kind label renders —
 * tabs, rail history/finished, backlinks list, markdown links. An entity
 * kind's icon is here; a declared page's is the one its `pages:` entry
 * names, so a page added to a manifest needs nothing here.
 */
import {
  Bot,
  BookOpen,
  Braces,
  CheckSquare,
  FileText,
  Folder,
  Forward,
  Gauge,
  GitCommit,
  GitCompare,
  Glasses,
  LayoutDashboard,
  type LucideIcon,
  MessageSquareText,
  MessagesSquare,
  Scale,
  StickyNote,
} from "lucide-react";
import type { ComponentProps, ReactElement } from "react";
import { REF_KIND_ICONS, resourceIcon, resourceInfo } from "./resources.js";
import { useExtensions } from "./extensionsStore.js";
import { pageKindOf, recordedPageKind, refFromTabId } from "./tabs/pageRefs.js";
import type { Extension, ExtensionPage } from "./tauri-bridge/generated/bindings.js";
import { extPageUri, pageUri as corePageRef, shortResource, typeOf } from "./uri/uri.js";

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
export function pageKindIconComponent(kind: string, extensions: Extension[] | null = null): LucideIcon | null {
  switch (kind) {
    // Scheme-prefixed kinds (TabRef.kind).
    case "file":
      return FileText;
    case "dir":
    // The file tree's entries (`WorkspaceEntry.kind`) still say `directory`.
    case "directory":
      return Folder;
    case "diff":
    // A snapshot, an effort and a turn open as their diff (P2.11).
    case "snapshot":
    case "effort":
    case "turn":
      return GitCompare;
    case "wiki":
      return BookOpen;
    case "work_item":
    // Comment anchors and the rail's finished list still say `task`.
    case "task":
      return CheckSquare;
    case "commit":
      return GitCommit;
    case "dashboard":
      return LayoutDashboard;

    case "lens":
    // A page no enabled extension declares (yet).
    case "page":
      return Glasses;
    case "symbol":
      return Braces;

    case "agent_session":
      return Bot;
    case "thread":
      return MessagesSquare;
    case "thread_note":
      return StickyNote;
    case "handoff":
      return Forward;
    case "comment":
      return MessageSquareText;
    case "decision":
      return Scale;

    case "metric":
      return Gauge;

    // An extension's ref kind (`ref_kinds:`, P8.D7) draws its declared icon.
    // A declared page draws the icon its `pages:` entry names.
    default:
      return resourceIcon(kind) ?? declaredPageIcon(extensions, kind);
  }
}

/** The declared page a page kind names (`tasks`, `gh/open-prs`). */
function declaredPageOf(extensions: Extension[] | null, kind: string): ExtensionPage | null {
  if (!extensions) return null;
  // A page kind is the page's id, `<extension>/` first for any but oxplow's.
  const slash = kind.indexOf("/");
  const pageRef = slash === -1 ? corePageRef(kind) : extPageUri(kind.slice(0, slash), kind.slice(slash + 1));
  for (const e of extensions) {
    const found = e.enabled ? e.pages.find((p) => p.pageRef === pageRef) : undefined;
    if (found) return found;
  }
  return null;
}

/** The icon a declared page names (`pages: - icon:`), or null when no
 *  page has that id or it names none. */
export function declaredPageIcon(extensions: Extension[] | null, kind: string): LucideIcon | null {
  const icon = declaredPageOf(extensions, kind)?.icon;
  return icon ? (REF_KIND_ICONS[icon] ?? null) : null;
}

export interface PageKindIconProps extends Omit<ComponentProps<LucideIcon>, "ref"> {
  /** An entity kind, or a declared page's id (`kindForTabId`). */
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
export function PageKindIcon({ kind, size = 14, ...rest }: PageKindIconProps): ReactElement | null {
  const Icon = pageKindIconComponent(kind);
  if (!Icon) return <DeclaredPageIcon kind={kind} size={size} {...rest} />;
  return <Icon aria-hidden size={size} {...rest} />;
}

/** A declared page's icon, which only the loaded extensions know. */
function DeclaredPageIcon({ kind, size, ...rest }: PageKindIconProps): ReactElement | null {
  const Icon = declaredPageIcon(useExtensions(), kind);
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
export function pageKindLabel(kind: string, extensions: Extension[] | null = null): string {
  switch (kind) {
    case "wiki":
      return "wiki page";
    case "work_item":
      return "work item";
    case "agent_session":
      return "agent session";
    case "thread_note":
      return "note";
    default:
      return resourceInfo(kind)?.label ?? declaredPageOf(extensions, kind)?.title.toLowerCase() ?? kind;
  }
}

/**
 * The kind a tab id renders as — an entity ref's kind
 * (`work_item:oxplow:tsk42` → `work_item`, `file:src/a.rs@git:HEAD` →
 * `file`) or a declared page's id (`page:diff-view?effort=eff9` →
 * `diff-view`, `page:gh/open-prs` → `gh/open-prs`).
 * Text that is not a tab id comes back unchanged so a caller can still
 * label it. The chrome uses this for icons and kind chips.
 */
export function kindForTabId(tabId: string): string {
  const kind = pageKindOf(tabId);
  if (kind === "page") return recordedPageKind(refFromTabId(tabId)!);
  if (kind) return kind;
  // A resource with no page of its own (a finding) still has an icon.
  // URI-SWITCH: the icon table is keyed by resource name (`v_resource`).
  const resource = typeOf(tabId);
  return resource === null ? tabId : shortResource(resource);
}
