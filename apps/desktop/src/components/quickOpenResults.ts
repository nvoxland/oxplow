import type { MenuGroup } from "../commands.js";
import { fuzzyMatches } from "../fuzzy-match.js";
import type { SearchHit, WorkspaceIndexedFile } from "../api.js";
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

/// A row in the empty-query launcher's collapsible "start menu" tree:
/// either a category header (toggles its section) or a page beneath an
/// expanded category. Pure of React so the keyboard/render contract is
/// unit-testable.
export type LauncherRow =
  | { kind: "category"; category: PageCategory; expanded: boolean }
  | { kind: "page"; entry: PageDirectoryEntry };

/// Assemble the launcher tree from the (already category-grouped) page
/// directory. Categories appear in first-seen order — which the curated
/// directory keeps aligned with `PAGE_CATEGORY_ORDER`. A category's page
/// rows are emitted only when it's in `expanded`; default-collapsed means
/// an empty set, so the launcher opens to just the category headers.
export function buildLauncherTree(
  pages: PageDirectoryEntry[],
  expanded: ReadonlySet<PageCategory>,
): LauncherRow[] {
  const rows: LauncherRow[] = [];
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
