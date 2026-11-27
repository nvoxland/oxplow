import { commandUri, keyOf } from "../uri/uri.js";
import type { SearchHit } from "../api.js";
import type { TabRef } from "../tabs/tabState.js";
import type { MenuPlace } from "../tauri-bridge/generated/bindings.js";
import { refFromTabId } from "../tabs/pageRefs.js";
import type { PageCategory, PageDirectoryEntry } from "./RailHud/sections.js";

/// A runnable menu command flattened for the launcher. Mirrors the
/// shape the retired CommandPalette used: `searchKey = "group label"`.
export interface CommandEntry {
  id: string;
  group: string;
  label: string;
  shortcut?: string;
  run: () => void;
  searchKey: string;
  /** Its shortcut runs it while typing in a field too. */
  whileTyping?: boolean;
  /** Where the menu bar shows it. */
  menu?: MenuPlace | null;
  /** It can run now; one that can't is greyed in a menu, left out of
   *  search. Unsaid, it can. */
  enabled?: boolean;
}

export type QuickOpenResult =
  | { kind: "page"; entry: PageDirectoryEntry }
  | { kind: "command"; entry: CommandEntry }
  /** A file of the current stream, by its path (and where its contents
   *  matched, when they did). */
  | { kind: "file"; path: string; snippet: string }
  | { kind: "hit"; hit: SearchHit };

/// The commands search offers: those that can run now — one that can't
/// (nothing for it to act on) is left out rather than advertised. Every
/// command the bus offers a person is a search command, on the menu bar
/// or not; pages aren't commands — they're the launcher's page rows.
export function searchableCommands(entries: CommandEntry[]): CommandEntry[] {
  return entries.filter((e) => e.enabled !== false);
}

/// Build the ordered launcher result list — what the one search shows.
/// Empty query = launcher mode (pages only, in their fixed
/// category-grouped order). With a query, the backend's search ranked
/// everything — pages, commands, files of this stream, tasks, knowledge,
/// … — on one scale (`search_rank`): its order is kept, each hit drawn
/// as its kind's row. A page hit is the directory's page (its badge, its
/// ref); a command hit runs the offer it names, and is left out when
/// nothing here offers it or it can't run now (`searchableCommands`).
export function buildQuickOpenResults(input: {
  query: string;
  /** The page directory; null while the extensions load — then nothing
   *  shows, so a quick Enter can't take a hit over the page it names. */
  pages: PageDirectoryEntry[] | null;
  commands: CommandEntry[];
  siteHits: SearchHit[];
}): QuickOpenResult[] {
  if (input.pages === null) return [];
  if (!input.query.trim()) {
    return input.pages.map((entry) => ({ kind: "page" as const, entry }));
  }
  const pages = new Map(input.pages.map((entry) => [entry.ref.id, entry]));
  const commands = new Map(searchableCommands(input.commands).map((entry) => [commandUri(entry.id), entry]));
  return input.siteHits.flatMap((hit): QuickOpenResult[] => {
    switch (hit.resource) {
      case "oxplow/page": {
        const entry = pages.get(hit.uri);
        return [entry ? { kind: "page", entry } : { kind: "hit", hit }];
      }
      case "oxplow/command": {
        const entry = commands.get(hit.uri);
        return entry ? [{ kind: "command", entry }] : [];
      }
      case "oxplow/file": {
        const path = keyOf(hit.uri);
        return [path ? { kind: "file", path, snippet: hit.snippet } : { kind: "hit", hit }];
      }
      default:
        return [{ kind: "hit", hit }];
    }
  });
}

/// The launcher's collapsible sections: the static page categories plus
/// the synthetic "Recent" section (recently visited pages), which isn't a
/// member of the static pages directory.
export type LauncherSection = PageCategory | "Recent";

/// A page-openable launcher row. `PageDirectoryEntry` is a superset (it
/// also carries a static `category`); recent-visit rows supply just these
/// fields, so the launcher tree accepts either.
export interface LauncherPageEntry {
  id: string;
  label: string;
  ref: TabRef;
  badge?: number;
}

/// A row in the empty-query launcher's collapsible "start menu" tree:
/// either a section header (toggles its section) or a page beneath an
/// expanded section. Pure of React so the keyboard/render contract is
/// unit-testable.
export type LauncherRow =
  | { kind: "category"; category: LauncherSection; expanded: boolean }
  | { kind: "page"; entry: LauncherPageEntry };

/// Map recent page-visit rows into launcher entries. Ids are `recent:`-
/// prefixed so they never collide with a static directory page's id (both
/// can be visible when Recent + a category are expanded). Visit rows don't
/// persist the ref payload, so `refFromTabId` rebuilds an openable ref from
/// the id (a bare `{id,kind}` would leave file/knowledge/task pages unopenable).
/// Label falls back to the ref id when the stored visit label is empty.
export function buildRecentEntries(
  visits: Array<{ refId: string; label: string }>,
): LauncherPageEntry[] {
  return visits.flatMap((v) => {
    const ref = refFromTabId(v.refId);
    return ref ? [{ id: `recent:${v.refId}`, label: v.label.trim() || v.refId, ref }] : [];
  });
}

/// Assemble the launcher tree. The `recent` entries lead as a "Recent"
/// section (only when non-empty), then the (already category-grouped) page
/// directory: categories in first-seen order, aligned with
/// `PAGE_CATEGORY_ORDER`. A section's page rows are emitted only when it's
/// in `expanded`, so a section absent from the set renders just its header.
export function buildLauncherTree(
  recent: LauncherPageEntry[],
  pages: PageDirectoryEntry[],
  expanded: ReadonlySet<LauncherSection>,
): LauncherRow[] {
  const rows: LauncherRow[] = [];
  if (recent.length > 0) {
    const isExpanded = expanded.has("Recent");
    rows.push({ kind: "category", category: "Recent", expanded: isExpanded });
    if (isExpanded) {
      for (const entry of recent) rows.push({ kind: "page", entry });
    }
  }
  let current: PageCategory | null = null;
  for (const entry of pages) {
    if (entry.category !== current) {
      current = entry.category;
      rows.push({ kind: "category", category: current, expanded: expanded.has(current) });
    }
    if (expanded.has(entry.category)) {
      rows.push({ kind: "page", entry });
    }
  }
  return rows;
}

/// A navigable launcher row: the collapsible section tree (launcher mode)
/// or the ranked results (search mode). One array drives both the
/// keyboard cursor and the render.
export type LauncherNavRow = LauncherRow | QuickOpenResult;

/// Index of the next/previous "section boundary" for Tab / Shift+Tab
/// traversal. In launcher mode (any `category` rows present) a boundary
/// is a category header, so Tab hops header-to-header. In search mode a
/// boundary is the first row of a new *kind* run (page→command→file→hit),
/// so Tab hops between result groups. Clamps to the last/first row when
/// there's no further section. Pure so it's unit-testable.
export function nextSectionIndex(rows: LauncherNavRow[], current: number, dir: 1 | -1): number {
  if (rows.length === 0) return 0;
  const hasCategories = rows.some((r) => r.kind === "category");
  const isBoundary = (i: number): boolean =>
    hasCategories ? rows[i]!.kind === "category" : i === 0 || rows[i]!.kind !== rows[i - 1]!.kind;
  for (let i = current + dir; i >= 0 && i < rows.length; i += dir) {
    if (isBoundary(i)) return i;
  }
  return dir > 0 ? rows.length - 1 : 0;
}

/** A part of a content hit's snippet: matched text (`hit`) or not. */
export interface SnippetPart {
  text: string;
  hit: boolean;
}

/** A content hit's snippet as parts: the index marks each match with «
 *  and » (`search_store`'s `snippet()`), which are drawn as matches and
 *  never shown, and a markdown heading's `#`s aren't text. */
export function snippetParts(snippet: string): SnippetPart[] {
  const text = snippet.replace(/(^|\s)#{1,6}\s+/g, "$1");
  const parts: SnippetPart[] = [];
  for (const [i, piece] of text.split(/[«»]/).entries()) {
    if (piece) parts.push({ text: piece, hit: i % 2 === 1 });
  }
  return parts;
}
