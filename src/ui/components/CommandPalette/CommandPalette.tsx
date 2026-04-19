import { useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties } from "react";
import type { MenuGroup } from "../../commands.js";

/**
 * Cmd/Ctrl+K palette — a second renderer for the same command registry that
 * feeds the native menu in `commands.ts`. Fuzzy-matches on the "group /
 * label" path so users can type "work new" or "file save" with any gap
 * between characters. Disabled commands are hidden so the list doesn't
 * advertise actions the user can't take right now.
 */

interface Entry {
  commandId: string;
  group: string;
  label: string;
  shortcut?: string;
  run: () => void;
  searchKey: string;
}

export function CommandPalette({
  menuGroups,
  onClose,
}: {
  menuGroups: MenuGroup[];
  onClose(): void;
}) {
  const [query, setQuery] = useState("");
  const [highlight, setHighlight] = useState(0);
  const listRef = useRef<HTMLDivElement | null>(null);

  const entries: Entry[] = useMemo(() => {
    const out: Entry[] = [];
    for (const group of menuGroups) {
      for (const item of group.items) {
        if (!item.enabled || !item.run) continue;
        const label = item.label;
        const groupLabel = group.label;
        out.push({
          commandId: item.id,
          group: groupLabel,
          label,
          shortcut: item.shortcut,
          run: item.run,
          searchKey: `${groupLabel} ${label}`.toLowerCase(),
        });
      }
    }
    return out;
  }, [menuGroups]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return entries;
    return entries.filter((entry) => fuzzyMatches(entry.searchKey, q));
  }, [entries, query]);

  useEffect(() => { setHighlight(0); }, [query]);

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  useEffect(() => {
    // Keep the highlighted row visible as it moves past the viewport edge.
    const row = listRef.current?.querySelector<HTMLDivElement>(`[data-palette-row="${highlight}"]`);
    row?.scrollIntoView({ block: "nearest" });
  }, [highlight]);

  const runHighlighted = () => {
    const entry = filtered[highlight];
    if (!entry) return;
    onClose();
    // Defer the command so the modal's unmount doesn't race against any
    // focus-restoration the command might do.
    setTimeout(entry.run, 0);
  };

  return (
    <div style={overlayStyle} onClick={onClose}>
      <div style={paletteStyle} onClick={(event) => event.stopPropagation()}>
        <input
          autoFocus
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              setHighlight((prev) => Math.min(prev + 1, Math.max(0, filtered.length - 1)));
            } else if (event.key === "ArrowUp") {
              event.preventDefault();
              setHighlight((prev) => Math.max(prev - 1, 0));
            } else if (event.key === "Enter") {
              event.preventDefault();
              runHighlighted();
            }
          }}
          placeholder="Run a command…"
          style={inputStyle}
        />
        <div ref={listRef} style={listStyle}>
          {filtered.length === 0 ? (
            <div style={emptyStyle}>No matching commands.</div>
          ) : (
            filtered.map((entry, index) => {
              const active = index === highlight;
              return (
                <div
                  key={entry.commandId}
                  data-palette-row={index}
                  onMouseEnter={() => setHighlight(index)}
                  onClick={() => { onClose(); setTimeout(entry.run, 0); }}
                  style={rowStyle(active)}
                >
                  <span style={{ color: active ? "rgba(255,255,255,0.8)" : "var(--muted)", fontSize: 11, flexShrink: 0 }}>{entry.group}</span>
                  <span style={{ color: active ? "rgba(255,255,255,0.6)" : "var(--muted)", fontSize: 11 }}>›</span>
                  <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {entry.label}
                  </span>
                  {entry.shortcut ? (
                    <span style={{ color: active ? "rgba(255,255,255,0.75)" : "var(--muted)", fontSize: 11, fontFamily: "ui-monospace, monospace" }}>{entry.shortcut}</span>
                  ) : null}
                </div>
              );
            })
          )}
        </div>
      </div>
    </div>
  );
}

// Subsequence match — each character in the query must appear in order in
// the haystack, with any gap allowed. Matches Linear's feel ("wn" → "work /
// new work item") without pulling in a scoring library for six commands.
export function fuzzyMatches(haystack: string, needle: string): boolean {
  if (!needle) return true;
  let ni = 0;
  for (let hi = 0; hi < haystack.length && ni < needle.length; hi++) {
    if (haystack[hi] === needle[ni]) ni++;
  }
  return ni === needle.length;
}

const overlayStyle: CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0,0,0,0.5)",
  display: "flex",
  alignItems: "flex-start",
  justifyContent: "center",
  paddingTop: "15vh",
  zIndex: 3000,
};

const paletteStyle: CSSProperties = {
  background: "var(--bg-1)",
  border: "1px solid var(--border-strong)",
  borderRadius: 8,
  width: "min(600px, 90vw)",
  maxHeight: "70vh",
  display: "flex",
  flexDirection: "column",
  boxShadow: "0 12px 32px rgba(0,0,0,0.6)",
  overflow: "hidden",
};

const inputStyle: CSSProperties = {
  background: "transparent",
  border: "none",
  borderBottom: "1px solid var(--border)",
  color: "var(--fg)",
  font: "inherit",
  fontSize: 14,
  padding: "10px 14px",
  outline: "none",
};

const listStyle: CSSProperties = {
  flex: 1,
  overflowY: "auto",
  padding: 4,
};

const emptyStyle: CSSProperties = {
  padding: "12px 14px",
  color: "var(--muted)",
  fontSize: 12,
};

function rowStyle(active: boolean): CSSProperties {
  return {
    display: "flex",
    alignItems: "center",
    gap: 8,
    padding: "6px 10px",
    borderRadius: 4,
    fontSize: 13,
    cursor: "pointer",
    background: active ? "var(--accent)" : "transparent",
    color: active ? "#fff" : "var(--fg)",
  };
}
