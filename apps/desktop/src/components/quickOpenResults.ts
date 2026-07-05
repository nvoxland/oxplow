import type { MenuGroup } from "../commands.js";
import { fuzzyMatches } from "../fuzzy-match.js";
import type { SearchHit, WorkspaceIndexedFile } from "../api.js";
import type { TabRef } from "../tabs/tabState.js";
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
}

export type QuickOpenResult =
  | { kind: "page"; entry: PageDirectoryEntry }
  | { kind: "command"; entry: CommandEntry }
  | { kind: "file"; file: WorkspaceIndexedFile }
  | { kind: "hit"; hit: SearchHit };

/// Flatten enabled, runnable menu commands into searchable entries.
/// Disabled commands (and the native responder-chain placeholders with
/// no `run`) are skipped so the launcher never advertises an action the
/// user can't take right now.
export function flattenCommands(menuGroups: MenuGroup[]): CommandEntry[] {
  const out: CommandEntry[] = [];
  for (const group of menuGroups) {
    for (const item of group.items) {
      if (!item.enabled || !item.run) continue;
      out.push({
        id: item.id,
        group: group.label,
        label: item.label,
        shortcut: item.shortcut,
        run: item.run,
        searchKey: `${group.label} ${item.label}`.toLowerCase(),
      });
    }
  }
  return out;
}

/// Body-search hits merged into the quick-open list after the filename
/// matches. File-kind hits whose path already matched by filename are
/// dropped (they'd be duplicate rows); everything else keeps the
/// backend's BM25 order.
export function dedupeSiteHits(
  hits: SearchHit[],
  matchedFilePaths: ReadonlySet<string>,
): SearchHit[] {
  return hits.filter((h) => !(h.kind === "file" && matchedFilePaths.has(h.ref_id)));
}

/// Split a query on whitespace and require each token to fuzzy-match
/// independently — so "thread new" finds "Tasks › New Thread" just as
/// well as "new thread". Within a token the subsequence match stays
/// order-sensitive.
function matchesAllTokens(haystack: string, tokens: string[]): boolean {
  return tokens.every((tok) => fuzzyMatches(haystack, tok));
}

/// Build the ordered launcher result list — the single source of truth
/// for what the one search shows. Empty query = launcher mode (pages
/// only, in their fixed category-grouped order). With a query: pages,
/// then commands, then files, then body hits. Pages and commands are
/// small curated lists, so they rank above file-path/body noise; a
/// "git" / "files" search shouldn't scroll past matching file paths to
/// reach the page. Body hits come last (already BM25-ranked), minus
/// file hits whose path matched by name above.
export function buildQuickOpenResults(input: {
  query: string;
  pages: PageDirectoryEntry[];
  commands: CommandEntry[];
  files: WorkspaceIndexedFile[];
  siteHits: SearchHit[];
}): QuickOpenResult[] {
  const q = input.query.trim().toLowerCase();
  if (!q) {
    return input.pages.map((entry) => ({ kind: "page" as const, entry }));
  }
  const tokens = q.split(/\s+/).filter(Boolean);
  const matchedPages: QuickOpenResult[] = input.pages
    .filter((entry) => matchesAllTokens(entry.label.toLowerCase(), tokens) || matchesAllTokens(entry.id, tokens))
    .map((entry) => ({ kind: "page", entry }));
  const matchedCommands: QuickOpenResult[] = input.commands
    .filter((entry) => matchesAllTokens(entry.searchKey, tokens))
    .map((entry) => ({ kind: "command", entry }));
  const matchedFiles = input.files
    .filter((file) => matchesAllTokens(file.path.toLowerCase(), tokens))
    .slice(0, 80);
  const matchedFileResults: QuickOpenResult[] = matchedFiles.map((file) => ({ kind: "file", file }));
  const matchedPaths = new Set(matchedFiles.map((f) => f.path));
  const bodyHits: QuickOpenResult[] = dedupeSiteHits(input.siteHits, matchedPaths)
    .slice(0, 30)
    .map((hit) => ({ kind: "hit", hit }));
  return [...matchedPages, ...matchedCommands, ...matchedFileResults, ...bodyHits];
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
/// the id (a bare `{id,kind}` would leave file/wiki/task pages unopenable).
/// Label falls back to the ref id when the stored visit label is empty.
export function buildRecentEntries(
  visits: Array<{ refId: string; label: string }>,
): LauncherPageEntry[] {
  return visits.map((v) => ({
    id: `recent:${v.refId}`,
    label: v.label.trim() || v.refId,
    ref: refFromTabId(v.refId),
  }));
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
