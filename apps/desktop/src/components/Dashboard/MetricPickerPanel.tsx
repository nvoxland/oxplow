import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import type { MetricCatalogEntry } from "../../api.js";
import { flattenPickerRows, pickerSections } from "./metricPicker.js";

const PANEL_W = 340;
const PANEL_MAX_H = 460;

/**
 * The dashboard's **add metric** picker (tsk145) — an anchored popover with a
 * search box over a scrollable, categorized list.
 *
 * Replaces a nested context menu: ~60 metrics in submenus overflowed the
 * viewport and couldn't be searched. Sections come from the canonical
 * {@link pickerSections} (same grouping as the Metrics page).
 *
 * Interaction: the search input takes focus on open; ↑/↓ walk the flattened row
 * list and Enter adds the highlighted metric; Escape or a click outside closes.
 * **Clicking a metric adds it and leaves the panel open** — assembling a
 * dashboard means adding several tiles, so the panel is a workbench rather than
 * a one-shot menu; rows already on the dashboard render with a ✓.
 */
export function MetricPickerPanel({
  catalog,
  anchor,
  addedKeys,
  onPick,
  onAddText,
  onClose,
}: {
  catalog: MetricCatalogEntry[];
  anchor: { x: number; y: number };
  /** Metric keys already on this dashboard — rendered as added. */
  addedKeys: Set<string>;
  onPick: (metricKey: string) => void;
  onAddText?: () => void;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);
  const [pos, setPos] = useState(anchor);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const rowRefs = useRef<Map<string, HTMLButtonElement>>(new Map());

  const sections = useMemo(() => pickerSections(catalog, query), [catalog, query]);
  const rows = useMemo(() => flattenPickerRows(sections), [sections]);

  // Clamp into the viewport — the same rule ContextMenu applies, so the panel
  // can't hang off an edge when opened near one.
  useLayoutEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const maxX = Math.max(8, window.innerWidth - root.offsetWidth - 8);
    const maxY = Math.max(8, window.innerHeight - root.offsetHeight - 8);
    setPos({
      x: Math.min(Math.max(8, anchor.x), maxX),
      y: Math.min(Math.max(8, anchor.y), maxY),
    });
  }, [anchor, sections.length]);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // A narrowed list can leave the cursor past the end.
  useEffect(() => {
    setCursor((c) => (c >= rows.length ? 0 : c));
  }, [rows.length]);

  // Keep the highlighted row in view as the cursor moves.
  useEffect(() => {
    const key = rows[cursor]?.key;
    if (key) rowRefs.current.get(key)?.scrollIntoView({ block: "nearest" });
  }, [cursor, rows]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) onClose();
    };
    // `mousedown` (not click) so the close beats any re-render of what's under.
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [onClose]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setCursor((c) => (rows.length ? (c + 1) % rows.length : 0));
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      setCursor((c) => (rows.length ? (c - 1 + rows.length) % rows.length : 0));
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      const row = rows[cursor];
      if (row) onPick(row.key);
    }
  };

  let rowIndex = -1;

  return (
    <div
      ref={rootRef}
      data-testid="metric-picker"
      onKeyDown={onKeyDown}
      style={{
        position: "fixed",
        left: pos.x,
        top: pos.y,
        width: PANEL_W,
        maxHeight: PANEL_MAX_H,
        display: "flex",
        flexDirection: "column",
        background: "var(--surface-card)",
        border: "1px solid var(--border-subtle)",
        borderRadius: 8,
        boxShadow: "0 8px 28px rgba(0,0,0,0.45)",
        zIndex: 1000,
        overflow: "hidden",
      }}
    >
      <div style={{ padding: 8, borderBottom: "1px solid var(--border-subtle)" }}>
        <input
          ref={inputRef}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setCursor(0);
          }}
          placeholder="Search metrics…"
          aria-label="Search metrics"
          data-testid="metric-picker-search"
          style={{
            width: "100%",
            boxSizing: "border-box",
            fontSize: 13,
            padding: "6px 8px",
            borderRadius: 6,
            border: "1px solid var(--border-subtle)",
            background: "var(--surface-app, #1a1a1a)",
            color: "var(--text, #ddd)",
          }}
        />
      </div>

      <div style={{ overflowY: "auto", flex: 1, padding: "4px 0" }}>
        {rows.length === 0 ? (
          <div style={{ padding: "12px 12px", fontSize: 13, opacity: 0.6 }} data-testid="metric-picker-empty">
            No metrics match “{query}”.
          </div>
        ) : (
          sections.map((section) => (
            <div key={section.key}>
              <div
                style={{
                  position: "sticky",
                  top: 0,
                  // A sticky element does NOT automatically paint above later
                  // siblings: without this the row buttons (which follow it in
                  // DOM order) render on top and the header text collides with
                  // a row scrolling under it.
                  zIndex: 1,
                  // Must be fully opaque so rows disappear cleanly underneath.
                  background: "var(--surface-card, #161a20)",
                  padding: "7px 12px 4px",
                  fontSize: 10,
                  fontWeight: 700,
                  letterSpacing: "0.06em",
                  textTransform: "uppercase",
                  // `opacity` on the header would make the whole box (including
                  // its background) translucent — dim the text color instead.
                  color: "var(--text-muted, #8b949e)",
                }}
              >
                {section.label}
              </div>
              {section.entries.map((e) => {
                rowIndex += 1;
                const active = rowIndex === cursor;
                const added = addedKeys.has(e.key);
                return (
                  <button
                    key={e.key}
                    ref={(el) => {
                      if (el) rowRefs.current.set(e.key, el);
                      else rowRefs.current.delete(e.key);
                    }}
                    type="button"
                    onClick={() => onPick(e.key)}
                    onMouseEnter={() => setCursor(rows.findIndex((r) => r.key === e.key))}
                    data-testid={`metric-picker-row-${e.key}`}
                    title={e.key}
                    style={{
                      all: "unset",
                      boxSizing: "border-box",
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "space-between",
                      gap: 8,
                      width: "100%",
                      padding: "6px 12px",
                      fontSize: 13,
                      cursor: "pointer",
                      color: "var(--text, #ddd)",
                      background: active ? "var(--accent-soft-bg, #23334a)" : "transparent",
                    }}
                  >
                    <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                      {e.title}
                    </span>
                    {added ? (
                      <span style={{ color: "var(--success, #3fb950)", fontSize: 12 }} aria-label="already added">
                        ✓
                      </span>
                    ) : null}
                  </button>
                );
              })}
            </div>
          ))
        )}
      </div>

      <div
        style={{
          borderTop: "1px solid var(--border-subtle)",
          padding: 6,
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          gap: 8,
        }}
      >
        {onAddText ? (
          <button
            type="button"
            onClick={onAddText}
            data-testid="metric-picker-add-text"
            style={{
              all: "unset",
              cursor: "pointer",
              fontSize: 12,
              padding: "4px 6px",
              color: "var(--text, #ddd)",
            }}
          >
            + Heading
          </button>
        ) : (
          <span />
        )}
        <span style={{ fontSize: 11, opacity: 0.45, paddingRight: 4 }}>↑↓ · Enter · Esc</span>
      </div>
    </div>
  );
}
