import type { CSSProperties } from "react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  listRecentPageVisits,
  listWorkspaceFiles,
  searchSite,
  subscribePageVisitEvents,
  subscribeWorkspaceEvents,
  type SearchHit,
  type Stream,
  type WorkspaceIndexedFile,
} from "../api.js";
import type { MenuGroup } from "../commands.js";
import {
  buildLauncherTree,
  buildQuickOpenResults,
  buildRecentEntries,
  flattenCommands,
  nextSectionIndex,
  type LauncherNavRow,
  type LauncherPageEntry,
  type LauncherSection,
  type QuickOpenResult,
} from "./quickOpenResults.js";
import { PageKindIcon } from "../pageKinds.js";
import type { TabRef } from "../tabs/tabState.js";
import { RAIL_HISTORY_EXCLUDE_KINDS } from "./RailHud/history.js";
import type { PageCategory, PageDirectoryEntry } from "./RailHud/sections.js";

interface Props {
  open: boolean;
  stream: Stream | null;
  /** Active thread — scopes the "Recent" section to this thread's visits,
   *  matching the rail History block. */
  threadId: string | null;
  selectedFilePath: string | null;
  /** Top-level pages/apps surfaced as launcher entries when the input
   *  is empty, and mixed into search results when the user types. */
  pages: PageDirectoryEntry[];
  /** Menu commands flattened into the launcher so actions (Commit,
   *  New Task, …) are discoverable here too — this is the only palette. */
  menuGroups: MenuGroup[];
  onClose(): void;
  onOpenFile(path: string): void;
  onOpenPage(ref: TabRef): void;
  /** Open a body-search hit (wiki/task/comment/file content match). */
  onOpenSearchHit(hit: SearchHit): void;
}

type Result = QuickOpenResult;

/** A navigable row: the launcher's collapsible section headers + page
 *  rows (empty-query "start menu") or the ranked page/command/file/hit
 *  results (while searching). */
type Row = LauncherNavRow;

// Below this query length the launcher filters only the in-memory
// pages/commands/files (fast, client-side) and skips the backend BM25
// round-trip — a single character isn't a meaningful content search and
// firing one per keystroke is pure noise.
const MIN_BODY_QUERY_LEN = 2;

// Over-fetch the project-wide body search. The search is project-wide
// (searchSite(q, null)) but buildQuickOpenResults discards other streams'
// FILE hits client-side; at the default limit of 50 those discarded hits
// could eat the budget and starve current-stream results that rank below
// 50. A larger ceiling leaves headroom after the client-side filter.
const BODY_SEARCH_LIMIT = 200;

const EXPANDED_CATEGORIES_KEY = "oxplow.launcher.expandedCategories";
// "Recent" is expanded by default (the section exists to *show* recent
// pages), tracked by a dedicated collapsed-flag so existing users' static
// category prefs in EXPANDED_CATEGORIES_KEY are untouched.
const RECENT_COLLAPSED_KEY = "oxplow.launcher.recentCollapsed";
// Recent visits to load for the "Recent" section (10 most recent pages).
const RECENT_LIMIT = 10;

function loadExpandedCategories(): Set<PageCategory> {
  try {
    const raw = localStorage.getItem(EXPANDED_CATEGORIES_KEY);
    if (!raw) return new Set();
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? new Set(parsed as PageCategory[]) : new Set();
  } catch {
    return new Set();
  }
}

function persistExpandedCategories(set: ReadonlySet<PageCategory>): void {
  try {
    localStorage.setItem(EXPANDED_CATEGORIES_KEY, JSON.stringify([...set]));
  } catch {
    // localStorage unavailable (private mode / SSR) — expansion just
    // won't persist across opens; the in-session state still works.
  }
}

function loadRecentCollapsed(): boolean {
  try {
    return localStorage.getItem(RECENT_COLLAPSED_KEY) === "1";
  } catch {
    return false;
  }
}

function persistRecentCollapsed(collapsed: boolean): void {
  try {
    localStorage.setItem(RECENT_COLLAPSED_KEY, collapsed ? "1" : "0");
  } catch {
    // localStorage unavailable — collapse state just won't persist.
  }
}

export function QuickOpenOverlay({ open, stream, threadId, selectedFilePath, pages, menuGroups, onClose, onOpenFile, onOpenPage, onOpenSearchHit }: Props) {
  const [query, setQuery] = useState("");
  const [files, setFiles] = useState<WorkspaceIndexedFile[]>([]);
  const [siteHits, setSiteHits] = useState<SearchHit[]>([]);
  const [recentEntries, setRecentEntries] = useState<LauncherPageEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [expandedCategories, setExpandedCategories] = useState<Set<PageCategory>>(loadExpandedCategories);
  const [recentCollapsed, setRecentCollapsed] = useState<boolean>(loadRecentCollapsed);
  const [panelCoords, setPanelCoords] = useState<CSSProperties>(centeredFallbackCoords);
  const inputRef = useRef<HTMLInputElement>(null);
  const resultsRef = useRef<HTMLDivElement>(null);

  // Anchor the panel *over* the rail "Search…" box so it reads as that
  // bar expanding in place — across (wider than the rail) and down (the
  // results) — rather than a second search bar / modal appearing below
  // it. The opaque panel starts at the box's top-left and fully covers
  // it. We read the always-visible trigger's rect by testid so both the
  // click and the Cmd-K/P keyboard paths land in the same place; if the
  // rail isn't mounted we fall back to a centered position near the top.
  useLayoutEffect(() => {
    if (!open) return;
    function place() {
      const anchor = document.querySelector('[data-testid="rail-search"]');
      if (!anchor) {
        setPanelCoords(centeredFallbackCoords);
        return;
      }
      const rect = anchor.getBoundingClientRect();
      // Expand across: at least as wide as the rail box, wider where the
      // window allows, clamped to the viewport.
      const width = Math.min(Math.max(rect.width, 460), window.innerWidth - 16);
      setPanelCoords({
        position: "fixed",
        left: Math.max(8, Math.min(window.innerWidth - width - 8, rect.left)),
        top: rect.top,
        width,
      });
    }
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [open]);

  function toggleCategory(category: PageCategory) {
    setExpandedCategories((current) => {
      const next = new Set(current);
      if (next.has(category)) next.delete(category);
      else next.add(category);
      persistExpandedCategories(next);
      return next;
    });
  }

  /** Toggle any launcher section header — the static categories via the
   *  expanded-set, "Recent" via its own default-expanded collapse flag. */
  function toggleSection(section: LauncherSection) {
    if (section === "Recent") {
      setRecentCollapsed((collapsed) => {
        const next = !collapsed;
        persistRecentCollapsed(next);
        return next;
      });
      return;
    }
    toggleCategory(section);
  }

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setSelectedIndex(0);
    inputRef.current?.focus();
  }, [open, stream?.id]);

  useEffect(() => {
    if (!open || !stream) return;
    let cancelled = false;
    let refreshTimer: ReturnType<typeof setTimeout> | null = null;
    const loadFiles = () => {
      setLoading(true);
      setError(null);
      listWorkspaceFiles(stream.id)
        .then((result) => {
          if (cancelled) return;
          setFiles(result.files);
        })
        .catch((e) => {
          if (cancelled) return;
          setError(String(e));
        })
        .finally(() => {
          if (cancelled) return;
          setLoading(false);
        });
    };
    loadFiles();
    const unsubscribe = subscribeWorkspaceEvents(stream.id, (event) => {
      if (event.kind === "updated") return;
      if (refreshTimer) clearTimeout(refreshTimer);
      refreshTimer = setTimeout(loadFiles, 75);
    });
    return () => {
      cancelled = true;
      unsubscribe();
      if (refreshTimer) clearTimeout(refreshTimer);
    };
  }, [open, stream?.id]);

  // Body search (unified BM25 index: wiki/task/comment/note/file
  // contents), debounced. This is what makes a multi-word phrase like
  // "workspace isolation" surface the wiki page it appears in — the
  // filename fuzzy-match alone can never see bodies.
  useEffect(() => {
    if (!open || !stream) return;
    const q = query.trim();
    // Skip the backend round-trip on a too-short query (a single char is
    // noise); the in-memory pages/commands/files still filter live.
    if (q.length < MIN_BODY_QUERY_LEN) {
      setSiteHits([]);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      // Project-wide (null): the launcher is the single discovery surface,
      // so cross-stream tasks/wiki/notes/comments are findable. File hits
      // are re-scoped to the current stream client-side in
      // buildQuickOpenResults (another worktree's files aren't openable here).
      searchSite(q, null, null, BODY_SEARCH_LIMIT)
        .then((rows) => {
          if (!cancelled) setSiteHits(rows);
        })
        .catch(() => {
          if (!cancelled) setSiteHits([]);
        });
    }, 150);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [open, stream?.id, query]);

  // Recent pages for the "Recent" start-menu section: the 10 most recent
  // visits (deduped by ref), reusing the rail History source + exclude set.
  // Kept live via page-visit events so opening a page reorders it here.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    const refresh = () => {
      void listRecentPageVisits({
        threadId,
        limit: RECENT_LIMIT,
        dedupeByRef: true,
        excludeKinds: RAIL_HISTORY_EXCLUDE_KINDS,
      })
        .then((rows) => {
          if (!cancelled) setRecentEntries(buildRecentEntries(rows));
        })
        .catch(() => {
          if (!cancelled) setRecentEntries([]);
        });
    };
    refresh();
    const off = subscribePageVisitEvents(refresh);
    return () => {
      cancelled = true;
      off();
    };
  }, [open, threadId]);

  // The launcher's commands — the retired CommandPalette's entries now
  // live here so this overlay is the single discovery surface.
  const commands = useMemo(() => flattenCommands(menuGroups), [menuGroups]);

  // Empty input = launcher mode (pages only, grouped by category in the
  // render below). With a query: exact matches first, then pages →
  // commands → files → body hits. The ordering + stream scoping + cap
  // accounting live in a pure helper so they're unit-testable.
  const build = useMemo(
    () => buildQuickOpenResults({ query, pages, commands, files, siteHits, currentStreamId: stream?.id ?? null }),
    [pages, commands, files, siteHits, query, stream?.id],
  );
  const results: Result[] = build.results;
  const truncated = build.truncated;
  const launcherMode = query.trim() === "";

  // Effective expanded sections: the persisted static categories, plus
  // "Recent" unless the user has explicitly collapsed it (default open).
  const expandedSections = useMemo<Set<LauncherSection>>(() => {
    const set = new Set<LauncherSection>(expandedCategories);
    if (!recentCollapsed) set.add("Recent");
    return set;
  }, [expandedCategories, recentCollapsed]);

  // Navigable rows: the collapsible section tree in launcher mode, the
  // flat ranked list while searching. One array drives both the keyboard
  // cursor and the render so they can't drift apart.
  const rows = useMemo<Row[]>(
    () => (launcherMode ? buildLauncherTree(recentEntries, pages, expandedSections) : results),
    [launcherMode, recentEntries, pages, expandedSections, results],
  );

  useEffect(() => {
    if (selectedIndex < rows.length) return;
    setSelectedIndex(rows.length === 0 ? 0 : rows.length - 1);
  }, [rows, selectedIndex]);

  // Keep the keyboard cursor visible: scroll the active row into the
  // results viewport when it moves (it can otherwise sit off-screen in
  // the maxHeight:50vh scroll area).
  useEffect(() => {
    resultsRef.current
      ?.querySelector<HTMLElement>(`[data-row-index="${selectedIndex}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex, rows]);

  if (!open || !stream) {
    return null;
  }

  function confirm(result: Row) {
    if (result.kind === "category") {
      toggleSection(result.category);
      return;
    }
    if (result.kind === "page") onOpenPage(result.entry.ref);
    else if (result.kind === "command") {
      onClose();
      // Defer so the overlay's unmount doesn't race any focus
      // restoration the command performs.
      setTimeout(result.entry.run, 0);
      return;
    } else if (result.kind === "file") onOpenFile(result.file.path);
    else onOpenSearchHit(result.hit);
    onClose();
  }

  return (
    <div style={backdropStyle} onMouseDown={onClose}>
      <div style={{ ...panelStyle, ...panelCoords }} onMouseDown={(event) => event.stopPropagation()}>
        <input
          ref={inputRef}
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setSelectedIndex(0);
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              onClose();
              return;
            }
            if (event.key === "ArrowDown") {
              event.preventDefault();
              setSelectedIndex((current) => Math.min(current + 1, Math.max(rows.length - 1, 0)));
              return;
            }
            if (event.key === "ArrowUp") {
              event.preventDefault();
              setSelectedIndex((current) => Math.max(current - 1, 0));
              return;
            }
            if (event.key === "Tab") {
              // Section jump: launcher mode hops header-to-header, search
              // mode hops between result groups (page→command→file→hit).
              event.preventDefault();
              setSelectedIndex((current) => nextSectionIndex(rows, current, event.shiftKey ? -1 : 1));
              return;
            }
            if (event.key === "Home") {
              event.preventDefault();
              setSelectedIndex(0);
              return;
            }
            if (event.key === "End") {
              event.preventDefault();
              setSelectedIndex(Math.max(rows.length - 1, 0));
              return;
            }
            if (event.key === "Enter") {
              event.preventDefault();
              const selected = rows[selectedIndex];
              if (selected) confirm(selected);
            }
          }}
          placeholder="Search everything — pages, files, commands, content…"
          style={inputStyle}
        />
        <div style={metaStyle}>
          <span>{stream.title}</span>
          <span>{loading ? "Indexing files…" : `${pages.length} pages · ${files.length} files`}</span>
        </div>
        {error ? <div style={errorStyle}>{error}</div> : null}
        <div ref={resultsRef} style={resultsStyle}>
          {rows.length === 0 && !loading ? (
            <div style={emptyStyle}>No matches.</div>
          ) : (
            rows.map((result, index) => {
              const active = index === selectedIndex;
              if (result.kind === "category") {
                // Collapsible "start menu" section header. Collapsed by
                // default so the empty launcher is a short list of
                // sections, not all ~21 pages at once.
                return (
                  <button type="button"
                    key={`category:${result.category}`}
                    data-testid={`launcher-category-${result.category}`}
                    data-row-index={index}
                    onClick={() => confirm(result)}
                    style={{
                      ...categoryRowStyle,
                      background: active ? "rgba(74, 158, 255, 0.18)" : "transparent",
                    }}
                  >
                    <span style={{ width: 22, display: "inline-flex", justifyContent: "center", color: "var(--muted)", fontSize: 16, lineHeight: 1 }}>
                      {result.expanded ? "▾" : "▸"}
                    </span>
                    <span style={{ flex: 1 }}>{result.category}</span>
                  </button>
                );
              }
              if (result.kind === "page") {
                // Page beneath an expanded category (launcher) or a
                // ranked match (search). Indented under its category in
                // launcher mode so the tree structure reads clearly.
                return (
                  <button type="button"
                    key={`page:${result.entry.id}`}
                    data-row-index={index}
                    onClick={() => confirm(result)}
                    style={{
                      ...resultStyle,
                      paddingLeft: launcherMode ? 28 : 10,
                      background: active ? "rgba(74, 158, 255, 0.18)" : "transparent",
                    }}
                  >
                    <span style={{ width: 18, display: "inline-flex", justifyContent: "center" }}>
                      <PageKindIcon kind={result.entry.ref.kind} size={14} style={{ color: "var(--text-secondary)" }} />
                    </span>
                    <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis" }}>
                      {result.entry.label}
                    </span>
                    {result.entry.badge ? (
                      <span style={badgeStyle}>{result.entry.badge}</span>
                    ) : null}
                    <span style={{ color: "var(--muted)", fontSize: 11 }}>page</span>
                  </button>
                );
              }
              if (result.kind === "command") {
                return (
                  <button type="button"
                    key={`command:${result.entry.id}`}
                    data-row-index={index}
                    onClick={() => confirm(result)}
                    style={{
                      ...resultStyle,
                      background: active ? "rgba(74, 158, 255, 0.18)" : "transparent",
                    }}
                  >
                    <span style={{ width: 18, display: "inline-flex", justifyContent: "center", color: "var(--muted)", fontSize: 11 }}>
                      ⌘
                    </span>
                    <span style={{ flexShrink: 0, color: "var(--muted)", fontSize: 11 }}>{result.entry.group}</span>
                    <span style={{ color: "var(--muted)", fontSize: 11 }}>›</span>
                    <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                      {result.entry.label}
                    </span>
                    {result.entry.shortcut ? (
                      <span style={{ color: "var(--muted)", fontSize: 11, fontFamily: "ui-monospace, monospace" }}>{result.entry.shortcut}</span>
                    ) : null}
                    <span style={{ color: "var(--muted)", fontSize: 11 }}>command</span>
                  </button>
                );
              }
              if (result.kind === "hit") {
                return (
                  <button type="button"
                    key={`hit:${result.hit.kind}:${result.hit.ref_id}:${result.hit.stream_id ?? ""}`}
                    data-row-index={index}
                    onClick={() => confirm(result)}
                    style={{
                      ...resultStyle,
                      background: active ? "rgba(74, 158, 255, 0.18)" : "transparent",
                    }}
                  >
                    <span style={{ width: 18, display: "inline-flex", justifyContent: "center" }}>
                      <PageKindIcon kind={result.hit.kind} size={14} style={{ color: "var(--text-secondary)" }} />
                    </span>
                    <span style={{ flexShrink: 0, maxWidth: "40%", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                      {result.hit.title || result.hit.ref_id}
                    </span>
                    <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", color: "var(--muted)", fontSize: 11 }}>
                      {result.hit.snippet}
                    </span>
                    <span style={{ color: "var(--muted)", fontSize: 11 }}>{result.hit.kind}</span>
                  </button>
                );
              }
              return (
                <button type="button"
                  key={`file:${result.file.path}`}
                  data-row-index={index}
                  onClick={() => confirm(result)}
                  style={{
                    ...resultStyle,
                    background: active ? "rgba(74, 158, 255, 0.18)" : "transparent",
                    color: result.file.path === selectedFilePath ? "var(--accent)" : "var(--fg)",
                  }}
                >
                  <span style={{ width: 18, display: "inline-flex", justifyContent: "center" }}>
                    <PageKindIcon kind="file" size={14} style={{ color: "var(--text-secondary)" }} />
                  </span>
                  <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis" }}>{result.file.path}</span>
                  <span style={{ color: "var(--muted)", fontSize: 11 }}>{shortStatus(result.file.gitStatus)}</span>
                </button>
              );
            })
          )}
          {truncated > 0 ? (
            <div style={moreStyle} data-testid="launcher-more">
              +{truncated} more — refine your search
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function shortStatus(status: WorkspaceIndexedFile["gitStatus"]): string {
  switch (status) {
    case "modified":
      return "M";
    case "added":
      return "A";
    case "deleted":
      return "D";
    case "renamed":
      return "R";
    case "untracked":
      return "U";
    default:
      return "";
  }
}

// Transparent full-screen layer: it captures the outside-click that
// closes the launcher but no longer dims the app, so the panel reads as
// a dropdown anchored to the rail Search box rather than a modal.
const backdropStyle: CSSProperties = {
  position: "fixed",
  inset: 0,
  zIndex: 40,
};

// Used when the rail Search trigger isn't on screen (e.g. a narrow
// layout that hid the rail): drop the panel near the top, centered.
const centeredFallbackCoords: CSSProperties = {
  position: "fixed",
  left: "50%",
  top: "10vh",
  transform: "translateX(-50%)",
  width: "min(720px, calc(100vw - 32px))",
};

const panelStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 8,
  padding: 12,
  border: "1px solid var(--border)",
  borderRadius: 8,
  background: "var(--bg)",
  boxShadow: "0 0 0 1px rgba(255,255,255,0.12), 0 16px 40px rgba(0, 0, 0, 0.45)",
};

const inputStyle: CSSProperties = {
  background: "var(--bg-2)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  padding: "10px 12px",
  borderRadius: 6,
  fontFamily: "inherit",
  fontSize: "var(--text-sm)",
};

const metaStyle: CSSProperties = {
  display: "flex",
  justifyContent: "space-between",
  gap: 12,
  color: "var(--muted)",
  fontSize: 11,
};

const errorStyle: CSSProperties = {
  color: "#ff6b6b",
  fontSize: "var(--text-xs)",
};

const resultsStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  maxHeight: "50vh",
  overflow: "auto",
  gap: 2,
};

const resultStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  width: "100%",
  border: "none",
  borderRadius: 4,
  padding: "8px 10px",
  background: "transparent",
  cursor: "pointer",
  fontFamily: "inherit",
  textAlign: "left",
};

const emptyStyle: CSSProperties = {
  color: "var(--muted)",
  padding: "8px 10px",
  fontSize: "var(--text-xs)",
};

// Non-selectable footer shown when a section hit its row cap — a missing
// result then reads as "narrow the query," not "not found."
const moreStyle: CSSProperties = {
  color: "var(--muted)",
  padding: "6px 10px",
  fontSize: 11,
  fontStyle: "italic",
};

const categoryRowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  width: "100%",
  border: "none",
  borderRadius: 4,
  padding: "8px 10px",
  cursor: "pointer",
  fontFamily: "inherit",
  textAlign: "left",
  color: "var(--text-secondary)",
  fontSize: 11,
  textTransform: "uppercase",
  letterSpacing: "0.06em",
};

const badgeStyle: CSSProperties = {
  fontSize: 10,
  color: "var(--text-secondary)",
  background: "var(--surface-tab-inactive)",
  padding: "1px 6px",
  borderRadius: 999,
};
