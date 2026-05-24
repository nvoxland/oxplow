import { useEffect, useRef, useState } from "react";

import { searchSite, type SearchHit } from "../api.js";

/// Site-wide search overlay. Type to query the unified FTS index across
/// tasks, comments, notes, wiki pages, and (current-stream) file contents;
/// Enter / click opens the selected hit, Escape closes. Navigation is
/// delegated to the parent via `onOpen` so this component stays decoupled
/// from the tab system.
export function SearchPalette({
  streamId,
  onOpen,
  onClose,
}: {
  streamId: string | null;
  onOpen: (hit: SearchHit) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [selected, setSelected] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // Debounced query — 150ms after the last keystroke, matching the wiki
  // search box. A cancel flag drops stale responses that resolve out of order.
  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setHits([]);
      setSelected(0);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      searchSite(q, streamId)
        .then((rows) => {
          if (!cancelled) {
            setHits(rows);
            setSelected(0);
          }
        })
        .catch(() => {
          if (!cancelled) setHits([]);
        });
    }, 150);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [query, streamId]);

  function onKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelected((s) => (hits.length === 0 ? 0 : Math.min(s + 1, hits.length - 1)));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelected((s) => Math.max(s - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const hit = hits[selected];
      if (hit) {
        onOpen(hit);
        onClose();
      }
    }
  }

  return (
    <div onMouseDown={onClose} style={overlay}>
      <div onMouseDown={(e) => e.stopPropagation()} style={panel}>
        <input
          ref={inputRef}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder="Search tasks, comments, wiki, files…"
          style={input}
        />
        <div style={list}>
          {hits.map((hit, i) => (
            <button
              type="button"
              key={`${hit.kind}:${hit.ref_id}:${hit.stream_id ?? ""}`}
              onMouseEnter={() => setSelected(i)}
              onClick={() => {
                onOpen(hit);
                onClose();
              }}
              style={{ ...row, ...(i === selected ? rowSelected : null) }}
            >
              <span style={badge}>{hit.kind}</span>
              <span style={title}>{hit.title || hit.ref_id}</span>
              <span style={snippet}>{hit.snippet}</span>
            </button>
          ))}
          {query.trim() && hits.length === 0 ? (
            <div style={empty}>No matches</div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

const overlay: React.CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0, 0, 0, 0.4)",
  display: "flex",
  justifyContent: "center",
  alignItems: "flex-start",
  paddingTop: "12vh",
  zIndex: 1000,
};

const panel: React.CSSProperties = {
  width: "min(680px, 90vw)",
  maxHeight: "70vh",
  display: "flex",
  flexDirection: "column",
  background: "var(--panel-bg, #1e1e1e)",
  color: "var(--fg, #e6e6e6)",
  border: "1px solid var(--border, #3a3a3a)",
  borderRadius: 8,
  boxShadow: "0 12px 40px rgba(0, 0, 0, 0.5)",
  overflow: "hidden",
};

const input: React.CSSProperties = {
  padding: "12px 14px",
  fontSize: 15,
  border: "none",
  borderBottom: "1px solid var(--border, #3a3a3a)",
  background: "transparent",
  color: "inherit",
  outline: "none",
};

const list: React.CSSProperties = { overflowY: "auto" };

const row: React.CSSProperties = {
  display: "flex",
  gap: 8,
  alignItems: "baseline",
  width: "100%",
  padding: "8px 14px",
  border: "none",
  background: "transparent",
  color: "inherit",
  textAlign: "left",
  cursor: "pointer",
  fontSize: 13,
};

const rowSelected: React.CSSProperties = { background: "var(--accent-bg, rgba(120,160,255,0.18))" };

const badge: React.CSSProperties = {
  flex: "0 0 auto",
  fontSize: 10,
  textTransform: "uppercase",
  letterSpacing: 0.5,
  opacity: 0.7,
  minWidth: 56,
};

const title: React.CSSProperties = {
  flex: "0 0 auto",
  fontWeight: 600,
  maxWidth: "40%",
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
};

const snippet: React.CSSProperties = {
  flex: "1 1 auto",
  opacity: 0.7,
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
};

const empty: React.CSSProperties = { padding: "12px 14px", opacity: 0.6, fontSize: 13 };
