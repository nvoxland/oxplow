import type { CSSProperties } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import { listWorkspaceFiles, subscribeWorkspaceEvents, type Stream, type WorkspaceIndexedFile } from "../api.js";
import { fuzzyMatches } from "../fuzzy-match.js";
import type { TabRef } from "../tabs/tabState.js";
import type { PageDirectoryEntry } from "./RailHud/sections.js";

interface Props {
  open: boolean;
  stream: Stream | null;
  selectedFilePath: string | null;
  /** Top-level pages/apps surfaced as launcher entries when the input
   *  is empty, and mixed into search results when the user types. */
  pages: PageDirectoryEntry[];
  onClose(): void;
  onOpenFile(path: string): void;
  onOpenPage(ref: TabRef): void;
}

type Result =
  | { kind: "page"; entry: PageDirectoryEntry }
  | { kind: "file"; file: WorkspaceIndexedFile };

export function QuickOpenOverlay({ open, stream, selectedFilePath, pages, onClose, onOpenFile, onOpenPage }: Props) {
  const [query, setQuery] = useState("");
  const [files, setFiles] = useState<WorkspaceIndexedFile[]>([]);
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

  // Empty input = launcher mode (pages only, fixed order). With a
  // query, search both pages and files in parallel and show pages
  // first — they're a finite curated list and a "plan" / "files"
  // / "git" search shouldn't have to scroll past matching file paths.
  const results = useMemo<Result[]>(() => {
    const q = query.trim().toLowerCase();
    if (!q) {
      return pages.map((entry) => ({ kind: "page" as const, entry }));
    }
    const matchedPages: Result[] = pages
      .filter((entry) => fuzzyMatches(entry.label.toLowerCase(), q) || fuzzyMatches(entry.id, q))
      .map((entry) => ({ kind: "page" as const, entry }));
    const matchedFiles: Result[] = files
      .filter((file) => fuzzyMatches(file.path.toLowerCase(), q))
      .slice(0, 80)
      .map((file) => ({ kind: "file" as const, file }));
    return [...matchedPages, ...matchedFiles];
  }, [pages, files, query]);

  useEffect(() => {
    if (selectedIndex < results.length) return;
    setSelectedIndex(results.length === 0 ? 0 : results.length - 1);
  }, [results, selectedIndex]);

  if (!open || !stream) {
    return null;
  }

  function confirm(result: Result) {
    if (result.kind === "page") onOpenPage(result.entry.ref);
    else onOpenFile(result.file.path);
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
          placeholder="Search pages and files…"
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
                return (
                  <button type="button"
                    key={`page:${result.entry.id}`}
                    onClick={() => confirm(result)}
                    style={{
                      ...resultStyle,
                      background: active ? "rgba(74, 158, 255, 0.18)" : "transparent",
                    }}
                  >
                    <span style={{ width: 18, textAlign: "center" }}>{result.entry.label.split("  ")[0]}</span>
                    <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis" }}>
                      {result.entry.label.split("  ").slice(1).join("  ") || result.entry.label}
                    </span>
                    <span style={{ color: "var(--muted)", fontSize: 11 }}>page</span>
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
                  <span style={{ width: 18, textAlign: "center" }}>📄</span>
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
  fontSize: 13,
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
  fontSize: 12,
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
  fontSize: 12,
};
