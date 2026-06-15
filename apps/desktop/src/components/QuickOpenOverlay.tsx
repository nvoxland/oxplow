import type { CSSProperties } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  listWorkspaceFiles,
  searchSite,
  subscribeWorkspaceEvents,
  type SearchHit,
  type Stream,
  type WorkspaceIndexedFile,
} from "../api.js";
import type { MenuGroup } from "../commands.js";
import { buildQuickOpenResults, flattenCommands, type QuickOpenResult } from "./quickOpenResults.js";
import { PageKindIcon } from "../pageKinds.js";
import type { TabRef } from "../tabs/tabState.js";
import type { PageDirectoryEntry } from "./RailHud/sections.js";

interface Props {
  open: boolean;
  stream: Stream | null;
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

export function QuickOpenOverlay({ open, stream, selectedFilePath, pages, menuGroups, onClose, onOpenFile, onOpenPage, onOpenSearchHit }: Props) {
  const [query, setQuery] = useState("");
  const [files, setFiles] = useState<WorkspaceIndexedFile[]>([]);
  const [siteHits, setSiteHits] = useState<SearchHit[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

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
    if (!q) {
      setSiteHits([]);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      searchSite(q, stream.id)
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

  // The launcher's commands — the retired CommandPalette's entries now
  // live here so this overlay is the single discovery surface.
  const commands = useMemo(() => flattenCommands(menuGroups), [menuGroups]);

  // Empty input = launcher mode (pages only, grouped by category in the
  // render below). With a query, rank pages → commands → files → body
  // hits. The ordering lives in a pure helper so it's unit-testable.
  const results = useMemo<Result[]>(
    () => buildQuickOpenResults({ query, pages, commands, files, siteHits }),
    [pages, commands, files, siteHits, query],
  );
  const launcherMode = query.trim() === "";

  useEffect(() => {
    if (selectedIndex < results.length) return;
    setSelectedIndex(results.length === 0 ? 0 : results.length - 1);
  }, [results, selectedIndex]);

  if (!open || !stream) {
    return null;
  }

  function confirm(result: Result) {
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
      <div style={panelStyle} onMouseDown={(event) => event.stopPropagation()}>
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
              setSelectedIndex((current) => Math.min(current + 1, Math.max(results.length - 1, 0)));
              return;
            }
            if (event.key === "ArrowUp") {
              event.preventDefault();
              setSelectedIndex((current) => Math.max(current - 1, 0));
              return;
            }
            if (event.key === "Enter") {
              event.preventDefault();
              const selected = results[selectedIndex];
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
        <div style={resultsStyle}>
          {results.length === 0 && !loading ? (
            <div style={emptyStyle}>No matches.</div>
          ) : (
            results.map((result, index) => {
              const active = index === selectedIndex;
              if (result.kind === "page") {
                // In launcher mode (empty query) all results are pages;
                // print a category heading whenever the section changes
                // so the empty state reads like a start menu. With a
                // query the list is ranked flat, so no headings.
                const prev = results[index - 1];
                const showHeading =
                  launcherMode && (index === 0 || (prev?.kind === "page" && prev.entry.category !== result.entry.category));
                return (
                  <div key={`page:${result.entry.id}`}>
                    {showHeading ? (
                      <div style={categoryHeadingStyle}>{result.entry.category}</div>
                    ) : null}
                    <button type="button"
                      onClick={() => confirm(result)}
                      style={{
                        ...resultStyle,
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
                  </div>
                );
              }
              if (result.kind === "command") {
                return (
                  <button type="button"
                    key={`command:${result.entry.id}`}
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

const backdropStyle: CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0, 0, 0, 0.45)",
  display: "flex",
  justifyContent: "center",
  alignItems: "flex-start",
  paddingTop: "10vh",
  zIndex: 40,
};

const panelStyle: CSSProperties = {
  width: "min(720px, calc(100vw - 32px))",
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

const categoryHeadingStyle: CSSProperties = {
  color: "var(--muted)",
  fontSize: 10,
  textTransform: "uppercase",
  letterSpacing: "0.06em",
  padding: "8px 10px 2px",
};

const badgeStyle: CSSProperties = {
  fontSize: 10,
  color: "var(--text-secondary)",
  background: "var(--surface-tab-inactive)",
  padding: "1px 6px",
  borderRadius: 999,
};
