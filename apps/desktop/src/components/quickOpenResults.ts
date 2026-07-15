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
/// user can't take right now. **Page-navigation commands (`item.opensPage`)
/// are also skipped** — they exist in `commands.ts` only so the native
/// menu bar has File/View/Git/Tasks entries, but in the launcher they'd
/// duplicate the canonical `page` row for the same destination (and
/// mislabel it as "command"). The marker lives on the command definition
/// (declarative, can't drift), so a new nav command is excluded
/// automatically — no denylist to keep in sync.
export function flattenCommands(menuGroups: MenuGroup[]): CommandEntry[] {
  const out: CommandEntry[] = [];
  for (const group of menuGroups) {
    for (const item of group.items) {
      if (!item.enabled || !item.run || item.opensPage) continue;
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

/// Does a result's displayed identity exactly equal the query (case-
/// insensitive)? Such a result is floated above every fuzzy section — the
/// "type the thing, jump to it" affordance. tsk51's task-id case falls
/// out for free: query `tsk30` === the task hit's `ref_id`.
function isExactMatch(r: QuickOpenResult, q: string): boolean {
  switch (r.kind) {
    case "page":
      return r.entry.label.toLowerCase() === q || r.entry.id.toLowerCase() === q;
    case "command":
      return r.entry.label.toLowerCase() === q;
    case "file": {
      const base = r.file.path.split("/").pop() ?? r.file.path;
      return base.toLowerCase() === q || r.file.path.toLowerCase() === q;
    }
    case "hit":
      return r.hit.title.toLowerCase() === q || r.hit.ref_id.toLowerCase() === q;
  }
}

/// Stable identity of a result, so exact matches floated to the top
/// aren't also shown again in their capped section below.
function resultKey(r: QuickOpenResult): string {
  switch (r.kind) {
    case "page":
      return `page:${r.entry.id}`;
    case "command":
      return `command:${r.entry.id}`;
    case "file":
      return `file:${r.file.path}`;
    case "hit":
      return `hit:${r.hit.kind}:${r.hit.ref_id}:${r.hit.stream_id ?? ""}`;
  }
}

/// Per-section row caps: the launcher shows at most this many file-path
/// and body-hit rows so a broad query can't flood the list. Overflow is
/// reported via `QuickOpenBuild.truncated`.
const MAX_FILE_ROWS = 80;
const MAX_HIT_ROWS = 30;

export interface QuickOpenBuild {
  results: QuickOpenResult[];
  /// Count of matches dropped by the per-section caps — drives the
  /// "+N more — refine your search" footer so a missing result reads as
  /// "narrow the query," not "not found."
  truncated: number;
}

/// Build the ordered launcher result list — the single source of truth
/// for what the one search shows. Empty query = launcher mode (pages
/// only, in their fixed category-grouped order). With a query: exact
/// matches first (see `isExactMatch`), then pages, commands, files, and
/// body hits. Pages and commands are small curated lists, so they rank
/// above file-path/body noise; a "git" / "files" search shouldn't scroll
/// past matching file paths to reach the page. Body hits come last
/// (already BM25-ranked), minus file hits whose path matched by name
/// above, and minus file hits from other streams (project-wide search
/// returns every stream's files, but another worktree's aren't openable
/// here — tasks/wiki/notes/comments stay project-wide).
export function buildQuickOpenResults(input: {
  query: string;
  pages: PageDirectoryEntry[];
  commands: CommandEntry[];
  files: WorkspaceIndexedFile[];
  siteHits: SearchHit[];
  currentStreamId?: string | null;
}): QuickOpenBuild {
  const q = input.query.trim().toLowerCase();
  if (!q) {
    return { results: input.pages.map((entry) => ({ kind: "page" as const, entry })), truncated: 0 };
  }
  const tokens = q.split(/\s+/).filter(Boolean);
  // Full (uncapped) matched sets. Pages also match their optional
  // `keywords` so e.g. the Tasks page (label "Tasks") is still found by
  // typing "dashboard".
  const matchedPages: QuickOpenResult[] = input.pages
    .filter(
      (entry) =>
        matchesAllTokens(entry.label.toLowerCase(), tokens) ||
        matchesAllTokens(entry.id, tokens) ||
        (entry.keywords ? matchesAllTokens(entry.keywords.toLowerCase(), tokens) : false),
    )
    .map((entry) => ({ kind: "page", entry }));
  const matchedCommands: QuickOpenResult[] = input.commands
    .filter((entry) => matchesAllTokens(entry.searchKey, tokens))
    .map((entry) => ({ kind: "command", entry }));
  const matchedFilesAll = input.files.filter((file) => matchesAllTokens(file.path.toLowerCase(), tokens));
  const matchedPathsAll = new Set(matchedFilesAll.map((f) => f.path));
  // Keep file hits only from the current stream (+ global); tasks/wiki/
  // notes/comments are project-wide. A null/undefined stream disables
  // the filter (keep everything).
  const streamScopedHits =
    input.currentStreamId == null
      ? input.siteHits
      : input.siteHits.filter(
          (h) => h.kind !== "file" || h.stream_id == null || h.stream_id === input.currentStreamId,
        );
  const dedupedHits = dedupeSiteHits(streamScopedHits, matchedPathsAll);
  const fileResultsAll: QuickOpenResult[] = matchedFilesAll.map((file) => ({ kind: "file", file }));
  const hitResultsAll: QuickOpenResult[] = dedupedHits.map((hit) => ({ kind: "hit", hit }));

  // Exact-identity matches float to the very top — computed over the FULL
  // (uncapped) set so an exact filename/id match ranked past a section cap
  // still surfaces ("type the thing, jump to it" must never lose to a cap).
  const everything = [...matchedPages, ...matchedCommands, ...fileResultsAll, ...hitResultsAll];
  const exact = everything.filter((r) => isExactMatch(r, q));
  const floated = new Set(exact.map(resultKey));
  const notFloated = (r: QuickOpenResult) => !floated.has(resultKey(r));

  // The remaining sections; caps apply to what's left after floating, so a
  // floated item never counts against (or hides behind) the cap.
  const restFiles = fileResultsAll.filter(notFloated);
  const restHits = hitResultsAll.filter(notFloated);
  const results = [
    ...exact,
    ...matchedPages.filter(notFloated),
    ...matchedCommands.filter(notFloated),
    ...restFiles.slice(0, MAX_FILE_ROWS),
    ...restHits.slice(0, MAX_HIT_ROWS),
  ];
  const truncated =
    Math.max(0, restFiles.length - MAX_FILE_ROWS) + Math.max(0, restHits.length - MAX_HIT_ROWS);
  return { results, truncated };
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
