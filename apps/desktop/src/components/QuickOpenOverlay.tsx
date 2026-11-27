import type { CommandEntry } from "./quickOpenResults.js";
import { useAppPreference } from "../prefs/remembered.js";
import { commandUri, fileUri, pageUri, shortResource } from "../uri/uri.js";
import { SEARCH_TRIGGER_TESTID } from "./TitleBar.js";
import type { CSSProperties } from "react";
import { useResources } from "../resources.js";
import { formatShortcut } from "../keybindings.js";
import { useListNav } from "../kit/index.js";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useExtensions } from "../extensionsStore.js";
import {
  listRecentPageVisits,
  searchSite,
  subscribePageVisitEvents,
  type SearchHit,
  type Stream,
} from "../api.js";
import {
  buildLauncherTree,
  buildQuickOpenResults,
  buildRecentEntries,
  searchableCommands,
  nextSectionIndex,
  snippetParts,
  type LauncherNavRow,
  type LauncherPageEntry,
  type LauncherSection,
  type QuickOpenResult,
} from "./quickOpenResults.js";
import { kindForTabId, PageKindIcon, pageKindLabel } from "../pageKinds.js";
import type { TabRef } from "../tabs/tabState.js";
import { RAIL_HISTORY_EXCLUDE_KINDS } from "./RailHud/history.js";
import type { PageCategory } from "./RailHud/sections.js";
import { launcherDirectory, useLauncherCounts } from "./extensionLauncher.js";

interface Props {
  open: boolean;
  /** What the box starts with each time it opens (a Find Usages). */
  initialQuery?: string;
  stream: Stream | null;
  /** Active thread — scopes the "Recent" section to this thread's visits,
   *  matching the rail History block. */
  threadId: string | null;
  selectedFilePath: string | null;
  /** Menu commands flattened into the launcher so actions (Commit,
   *  New Task, …) are discoverable here too — this is the only palette. */
  /** What the command bus offers a person here (`commandOffers`). */
  offers: CommandEntry[];
  onClose(): void;
  onOpenFile(path: string): void;
  onOpenPage(ref: TabRef): void;
  /** Open a body-search hit (knowledge page/task/file content match). */
  onOpenSearchHit(hit: SearchHit): void;
}

type Result = QuickOpenResult;

/** A navigable row: the launcher's collapsible section headers + page
 *  rows (empty-query "start menu") or the ranked page/command/file/hit
 *  results (while searching). */
type Row = LauncherNavRow;

// How many ranked results one search shows.
const SEARCH_LIMIT = 100;
// How long typing pauses before the search runs.
const SEARCH_DEBOUNCE_MS = 60;

// Recent visits to load for the "Recent" section (10 most recent pages).
const RECENT_LIMIT = 10;

/** A row's id and aria state from the list contract. Not its hover: the
 *  launcher scrolls the cursor's row into view, so a cursor that follows
 *  the pointer would fight the wheel. */
const rowA11y = ({ onMouseEnter: _hover, ...rest }: ReturnType<ReturnType<typeof useListNav>["optionProps"]>) => rest;

export function QuickOpenOverlay({ open, initialQuery, stream, threadId, selectedFilePath, offers, onClose, onOpenFile, onOpenPage, onOpenSearchHit }: Props) {
  const [query, setQuery] = useState("");
  const [siteHits, setSiteHits] = useState<SearchHit[]>([]);
  // A hit of an extension's ref kind is drawn with that kind's icon and
  // label: redraw when the kinds change.
  useResources();
  const [recentEntries, setRecentEntries] = useState<LauncherPageEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  // The query the search last answered: "No matches." only once it has.
  const [answered, setAnswered] = useState<string | null>(null);
  const [expandedList, setExpandedList] = useAppPreference<PageCategory[]>("quickOpen.expandedCategories", []);
  const expandedCategories = useMemo(() => new Set(expandedList), [expandedList]);
  const [recentCollapsed, setRecentCollapsed] = useAppPreference<boolean>("quickOpen.recentCollapsed", false);
  const [panelCoords, setPanelCoords] = useState<CSSProperties>(centeredFallbackCoords);
  const inputRef = useRef<HTMLInputElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const resultsRef = useRef<HTMLDivElement>(null);

  // Anchor the panel *over* the title bar's "Search…" field so it reads
  // as that field expanding in place — across and down (the results) —
  // rather than a second search bar / modal appearing below it. The opaque panel starts at the box's top-left and fully covers
  // it. We read the always-visible trigger's rect by testid so both the
  // click and the Cmd-K/P keyboard paths land in the same place; if the
  // rail isn't mounted we fall back to a centered position near the top.
  useLayoutEffect(() => {
    if (!open) return;
    function place() {
      const anchor = document.querySelector(`[data-testid="${SEARCH_TRIGGER_TESTID}"]`);
      if (!anchor) {
        setPanelCoords(centeredFallbackCoords);
        return;
      }
      const rect = anchor.getBoundingClientRect();
      // Expand across: at least as wide as the field, wider where the
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
    setExpandedList(
      expandedCategories.has(category) ? expandedList.filter((c) => c !== category) : [...expandedList, category],
    );
  }

  /** Toggle any launcher section header — the static categories via the
   *  expanded-set, "Recent" via its own default-expanded collapse flag. */
  function toggleSection(section: LauncherSection) {
    if (section === "Recent") {
      setRecentCollapsed(!recentCollapsed);
      return;
    }
    toggleCategory(section);
  }

  // A fresh query each time it opens — only then: the stream finishing
  // loading while it's open must not wipe what the person typed.
  useEffect(() => {
    if (!open) return;
    setQuery(initialQuery ?? "");
  }, [open]);
  // Its input has the focus whenever it shows: it renders only once there
  // is a stream, which on a fresh window can be just after it opened.
  useEffect(() => {
    if (open && stream) inputRef.current?.focus();
  }, [open, stream?.id]);

  // The one search: the backend ranks pages, commands, this stream's
  // files, tasks, knowledge and every other searchable kind on one scale
  // (`search_rank`), by name and by contents; the launcher keeps its
  // order. Files are this stream's (`stream.id`); tasks and knowledge
  // belong to the project.
  useEffect(() => {
    if (!open || !stream) return;
    const q = query.trim();
    if (!q) {
      setSiteHits([]);
      setError(null);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      searchSite(q, stream.id, null, SEARCH_LIMIT)
        .then((rows) => {
          if (cancelled) return;
          setSiteHits(rows);
          setAnswered(q);
          setError(null);
        })
        .catch((e) => {
          if (cancelled) return;
          setSiteHits([]);
          setError(String(e));
        });
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [open, stream?.id, query]);

  // Pages from this stream's extensions join the page directory under
  // their `launcher.category` ("Dashboards" by default), so they're both browsable (start menu) and
  // searchable — from the shared extensions store, which reloads when
  // page files change (the agent may have just written one).
  // Their manifest `launcher:` entries join too: a ref as a page, a
  // command or prompt as an action (P6.D1).
  const exts = useExtensions();
  const counts = useLauncherCounts(exts, stream?.id ?? null, open);
  const pages = useMemo(() => launcherDirectory(exts, counts), [exts, counts]);

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

  // The launcher's commands: the bus's offers that can run now (an
  // extension's too — a command with a `ui`).
  const commands = useMemo(() => searchableCommands(offers), [offers]);

  // Empty input = launcher mode (pages only, grouped by category in the
  // render below). With a query: the backend's ranked hits, in its order,
  // each drawn as its kind's row (a pure helper, unit-testable).
  const results: Result[] = useMemo(
    () => buildQuickOpenResults({ query, pages: exts === null ? null : pages, commands, siteHits }),
    [exts, pages, commands, siteHits, query],
  );
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

  // The one list contract: ↑/↓/Enter/Escape (kit `useListNav`), a cursor
  // that stays on a row as the list shortens. Tab / Home / End are the
  // launcher's own, below.
  const nav = useListNav({
    count: rows.length,
    onPick: (i) => {
      const row = rows[i];
      if (row) confirm(row);
    },
    onEscape: onClose,
    id: "quick-open",
  });
  const { cursor: selectedIndex, setCursor: setSelectedIndex } = nav;
  // The rows showing answer an earlier query while the search for what
  // was typed runs: Enter then waits for its answer and takes its first
  // row, never a row of the earlier query.
  const stale = !launcherMode && answered !== query.trim();
  const [pickWhenAnswered, setPickWhenAnswered] = useState(false);
  useEffect(() => {
    if (!pickWhenAnswered || stale) return;
    setPickWhenAnswered(false);
    const row = rows[0];
    if (row) confirm(row);
  }, [pickWhenAnswered, stale, rows]);
  // A fresh query each time it opens starts at the first row.
  useEffect(() => {
    if (open) setSelectedIndex(0);
  }, [open, setSelectedIndex]);

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
    } else if (result.kind === "file") onOpenFile(result.path);
    else onOpenSearchHit(result.hit);
    onClose();
  }

  return (
    <div style={backdropStyle} onMouseDown={onClose}>
      <div
        ref={panelRef}
        style={{ ...panelStyle, ...panelCoords }}
        onMouseDown={(event) => {
          event.stopPropagation();
          // Pressing a row leaves focus in the input, where the keys are
          // (Escape, ↑/↓, Enter): a row is a button, and a button pressed
          // takes focus.
          if ((event.target as HTMLElement).closest("button")) event.preventDefault();
        }}
        // While it is open the launcher is where typing goes. After a reload
        // the window settles under it — a restored tab or terminal mounts and
        // takes focus — and the keys that follow reached neither.
        // Focus leaving for somewhere outside it, with the launcher still
        // showing, comes back; a click outside closes it first (the
        // backdrop), and focus inside it is left alone.
        onBlur={(event) => {
          const to = event.relatedTarget as Node | null;
          if (to && panelRef.current?.contains(to)) return;
          requestAnimationFrame(() => {
            const panel = panelRef.current;
            if (panel && !panel.contains(document.activeElement)) inputRef.current?.focus();
          });
        }}
      >
        <input
          ref={inputRef}
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setSelectedIndex(0);
          }}
          {...nav.inputProps}
          onKeyDown={(event) => {
            if (event.key === "Enter" && stale) {
              event.preventDefault();
              setPickWhenAnswered(true);
            } else if (event.key === "Tab") {
              // Section jump: launcher mode hops header-to-header, search
              // mode hops between result groups (page→command→file→hit).
              event.preventDefault();
              setSelectedIndex(nextSectionIndex(rows, selectedIndex, event.shiftKey ? -1 : 1));
            } else if (event.key === "Home") {
              event.preventDefault();
              setSelectedIndex(0);
            } else if (event.key === "End") {
              event.preventDefault();
              setSelectedIndex(Math.max(rows.length - 1, 0));
            } else {
              nav.inputProps.onKeyDown?.(event);
            }
          }}
          placeholder="Search everything — pages, files, commands, content…"
          style={inputStyle}
        />
        <div style={metaStyle}>
          <span>{stream.title}</span>
        </div>
        {error ? <div style={errorStyle}>{error}</div> : null}
        <div ref={resultsRef} style={resultsStyle}>
          {rows.length === 0 && query.trim() !== "" && answered === query.trim() ? (
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
                    {...rowA11y(nav.optionProps(index))}
                    onClick={() => confirm(result)}
                    style={{
                      ...categoryRowStyle,
                      background: active ? "var(--accent-soft-bg)" : "transparent",
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
                    key={pageUri(result.entry.id)}
                    data-testid={`launcher-page-${result.entry.id}`}
                    data-row-index={index}
                    {...rowA11y(nav.optionProps(index))}
                    onClick={() => confirm(result)}
                    style={{
                      ...resultStyle,
                      paddingLeft: launcherMode ? 28 : 10,
                      background: active ? "var(--accent-soft-bg)" : "transparent",
                    }}
                  >
                    <span style={{ width: 18, display: "inline-flex", justifyContent: "center" }}>
                      <PageKindIcon kind={kindForTabId(result.entry.ref.id)} size={14} style={{ color: "var(--text-secondary)" }} />
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
                    key={commandUri(result.entry.id)}
                    data-testid={`launcher-command-${result.entry.id}`}
                    data-row-index={index}
                    {...rowA11y(nav.optionProps(index))}
                    onClick={() => confirm(result)}
                    style={{
                      ...resultStyle,
                      background: active ? "var(--accent-soft-bg)" : "transparent",
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
                      <span style={{ color: "var(--muted)", fontSize: 11, fontFamily: "ui-monospace, monospace" }}>{formatShortcut(result.entry.shortcut)}</span>
                    ) : null}
                    <span style={{ color: "var(--muted)", fontSize: 11 }}>command</span>
                  </button>
                );
              }
              if (result.kind === "hit") {
                return (
                  <button type="button"
                    key={`hit:${result.hit.uri}:${result.hit.stream_id ?? ""}`}
                    data-testid={`launcher-hit-${result.hit.uri}`}
                    data-row-index={index}
                    {...rowA11y(nav.optionProps(index))}
                    onClick={() => confirm(result)}
                    style={{
                      ...resultStyle,
                      background: active ? "var(--accent-soft-bg)" : "transparent",
                    }}
                  >
                    <span style={{ width: 18, display: "inline-flex", justifyContent: "center" }}>
                      <PageKindIcon kind={shortResource(result.hit.resource)} size={14} style={{ color: "var(--text-secondary)" }} />
                    </span>
                    <span style={{ flexShrink: 0, maxWidth: "40%", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                      {result.hit.title || result.hit.uri}
                    </span>
                    <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", color: "var(--muted)", fontSize: 11 }}>
                      {snippetParts(result.hit.snippet).map((part, i) =>
                        part.hit ? (
                          <strong key={i} style={{ color: "var(--text-primary)", fontWeight: 600 }}>
                            {part.text}
                          </strong>
                        ) : (
                          <span key={i}>{part.text}</span>
                        ),
                      )}
                    </span>
                    <span style={{ color: "var(--muted)", fontSize: 11 }}>{pageKindLabel(shortResource(result.hit.resource))}</span>
                  </button>
                );
              }
              return (
                <button type="button"
                  key={fileUri(result.path)}
                  data-row-index={index}
                    {...rowA11y(nav.optionProps(index))}
                  onClick={() => confirm(result)}
                  style={{
                    ...resultStyle,
                    background: active ? "var(--accent-soft-bg)" : "transparent",
                    color: result.path === selectedFilePath ? "var(--accent)" : "var(--fg)",
                  }}
                >
                  <span style={{ width: 18, display: "inline-flex", justifyContent: "center" }}>
                    <PageKindIcon kind="file" size={14} style={{ color: "var(--text-secondary)" }} />
                  </span>
                  <FilePath path={result.path} />
                  {result.snippet ? (
                    <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", color: "var(--muted)", fontSize: 11 }}>
                      {snippetParts(result.snippet).map((part, i) =>
                        part.hit ? (
                          <strong key={i} style={{ color: "var(--text-primary)", fontWeight: 600 }}>
                            {part.text}
                          </strong>
                        ) : (
                          part.text
                        ),
                      )}
                    </span>
                  ) : null}
                </button>
              );
            })
          )}
        </div>
      </div>
    </div>
  );
}

// Transparent full-screen layer: it captures the outside-click that
// closes the launcher but no longer dims the app, so the panel reads as
// a dropdown anchored to the title bar's Search field rather than a modal.
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
  boxShadow: "var(--shadow-modal)",
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

/** A path that, when too long, loses the middle of its folders and keeps the file name. */
function FilePath({ path }: { path: string }) {
  const cut = path.lastIndexOf("/") + 1;
  return (
    <span style={{ flex: 1, minWidth: 0, display: "flex" }} title={path}>
      <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", minWidth: 0 }}>{path.slice(0, cut)}</span>
      <span style={{ flexShrink: 0, whiteSpace: "nowrap" }}>{path.slice(cut)}</span>
    </span>
  );
}
