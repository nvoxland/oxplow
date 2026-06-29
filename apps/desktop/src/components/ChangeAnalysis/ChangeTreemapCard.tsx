import { useMemo, useRef, useState, useEffect } from "react";
import type { BranchChangeEntry } from "../../api-types.js";
import type { FunctionChurnRow } from "./analysisHelpers.js";
import { classifyZone, ZONE_COLORS, ZONE_LABELS, type Zone } from "./zones.js";
import { usePageSnapshot } from "../../tabs/usePageSnapshot.js";

type TreemapView = "files" | "functions";
type AreaMetric = "total" | "added" | "deleted";

interface Props {
  files: BranchChangeEntry[];
  functionChurn: FunctionChurnRow[];
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  /** Open the file's diff at `line` (function tiles reveal their start
   *  line). Falls back to `onOpenFile` when absent. */
  onOpenFileDiff?(path: string, line?: number): void;
}

/**
 * The **Churn** treemap (subsumes the old Architectural-zones bar AND the
 * Churn list, tsk350/tsk358). A squarified WinDirStat-style map of the
 * change's churn, grouped by architectural zone:
 *
 *  - **Area:** Total / + Added / − Deleted picks which churn metric sizes
 *    each tile (and zone block).
 *  - **Files / Functions** picks the tile grain — changed files, or
 *    changed functions (from `functionChurn`).
 *
 * A colour legend below names every touched zone (the per-tile labels
 * can't — small tiles drop their text). Inline SVG, no graph library.
 */
export function ChangeTreemapCard({ files, functionChurn, onOpenFile, onOpenFileDiff }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [containerWidth, setContainerWidth] = useState(640);
  useEffect(() => {
    if (!ref.current) return;
    const ro = new ResizeObserver((entries) => {
      for (const e of entries) {
        const w = Math.max(120, e.contentRect.width);
        setContainerWidth(w);
      }
    });
    ro.observe(ref.current);
    return () => ro.disconnect();
  }, []);

  const [view, setView] = useState<TreemapView>("files");
  const [area, setArea] = useState<AreaMetric>("total");
  usePageSnapshot<{ treemapView: TreemapView; treemapArea: AreaMetric }>({
    serialize: () => ({ treemapView: view, treemapArea: area }),
    restore: (snap) => {
      if (snap.treemapView === "files" || snap.treemapView === "functions") setView(snap.treemapView);
      if (snap.treemapArea === "total" || snap.treemapArea === "added" || snap.treemapArea === "deleted") {
        setArea(snap.treemapArea);
      }
    },
    deps: [view, area],
  });

  const items = useMemo<ChurnItem[]>(() => {
    if (view === "files") {
      return files.map((f) => ({
        key: `file::${f.path}`,
        path: f.path,
        zone: classifyZone(f.path),
        label: basename(f.path),
        fnLabel: null,
        startLine: null,
        added: f.additions ?? 0,
        deleted: f.deletions ?? 0,
      }));
    }
    return functionChurn.map((c) => ({
      key: `fn::${c.path}::${c.containerPath.join("::")}::${c.name}`,
      path: c.path,
      zone: classifyZone(c.path),
      label: c.name,
      fnLabel: c.containerPath.length > 0 ? `${c.containerPath.join("::")}::${c.name}` : c.name,
      startLine: c.startLineHead,
      added: c.addedLines,
      deleted: c.deletedLines,
    }));
  }, [view, files, functionChurn]);

  const layout = useMemo(
    () => layoutTreemapByZone(items, area, containerWidth, 240),
    [items, area, containerWidth],
  );

  if (files.length === 0 && functionChurn.length === 0) {
    return null;
  }

  const open = (item: ChurnItem, newTab: boolean) => {
    if (newTab) {
      onOpenFile(item.path, { newTab: true });
      return;
    }
    if (onOpenFileDiff) onOpenFileDiff(item.path, item.startLine ?? undefined);
    else onOpenFile(item.path);
  };

  return (
    <div style={card} ref={ref}>
      <header style={cardHeader}>
        <div style={{ display: "flex", gap: 12, alignItems: "center", flexWrap: "wrap" }}>
          <div
            style={{ display: "flex", gap: 4, alignItems: "center" }}
            title="Size each tile by all changed lines, additions only, or deletions only."
          >
            <span style={{ fontSize: 11, color: "var(--text-muted)" }}>Area:</span>
            {([
              ["total", "All changes"],
              ["added", "+ Added"],
              ["deleted", "− Deleted"],
            ] as const).map(([key, label]) => (
              <button
                key={key}
                type="button"
                data-testid={`change-analysis-treemap-area-${key}`}
                onClick={() => setArea(key)}
                style={area === key ? activeTab : tab}
              >
                {label}
              </button>
            ))}
          </div>
        </div>
        <div style={{ display: "flex", gap: 4 }}>
          {([
            ["files", "Files"],
            ["functions", "Functions"],
          ] as const).map(([key, label]) => (
            <button
              key={key}
              type="button"
              data-testid={`change-analysis-treemap-view-${key}`}
              onClick={() => setView(key)}
              style={view === key ? activeTab : tab}
            >
              {label}
            </button>
          ))}
        </div>
      </header>
      {layout.cells.length === 0 ? (
        <div style={muted}>
          {view === "functions"
            ? "No function-level churn for this metric."
            : "No file churn for this metric."}
        </div>
      ) : (
        <>
          <svg
            width={containerWidth}
            height={240}
            style={{ display: "block" }}
            role="img"
            aria-label="Treemap of churn, grouped by architectural zone"
          >
            {layout.cells.map((cell) => (
              <g key={cell.item.key}>
                <rect
                  x={cell.x}
                  y={cell.y}
                  width={cell.w}
                  height={cell.h}
                  fill={ZONE_COLORS[cell.item.zone]}
                  stroke="var(--surface-card)"
                  strokeWidth={1}
                  onClick={(e) => open(cell.item, e.metaKey || e.ctrlKey)}
                  style={{ cursor: "pointer" }}
                >
                  <title>
                    {cell.item.fnLabel ? `${cell.item.fnLabel} — ${cell.item.path}` : cell.item.path} (
                    {ZONE_LABELS[cell.item.zone]}) · +{cell.item.added} −{cell.item.deleted}
                  </title>
                </rect>
                {cell.w > 60 && cell.h > 20 ? (
                  <text
                    x={cell.x + 4}
                    y={cell.y + 14}
                    fontSize={11}
                    fill="white"
                    pointerEvents="none"
                    style={{ fontFamily: "var(--font-mono, monospace)" }}
                  >
                    {truncate(cell.item.label, Math.floor(cell.w / 7))}
                  </text>
                ) : null}
              </g>
            ))}
          </svg>
          <div style={legend} aria-label="Architectural zone colors">
            {layout.zones.map((z) => (
              <span key={z} style={legendItem}>
                <span style={{ ...legendSwatch, background: ZONE_COLORS[z] }} />
                {ZONE_LABELS[z]}
              </span>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

/** A treemap tile — a changed file or a changed function. */
interface ChurnItem {
  key: string;
  path: string;
  zone: Zone;
  /** Tile label: basename (files) or function name (functions). */
  label: string;
  /** Fully-qualified function name for the tooltip; null for files. */
  fnLabel: string | null;
  /** Function start line (head side) for the diff reveal; null for files. */
  startLine: number | null;
  added: number;
  deleted: number;
}

interface TreemapCell {
  item: ChurnItem;
  x: number;
  y: number;
  w: number;
  h: number;
}

/** The churn value that sizes a tile, per the chosen Area metric. */
function metricValue(item: ChurnItem, area: AreaMetric): number {
  if (area === "added") return item.added;
  if (area === "deleted") return item.deleted;
  return item.added + item.deleted;
}

/**
 * Two-level squarified treemap: outer pass packs zones by total churn (in
 * the chosen metric), inner pass packs each zone's items into its rect.
 * Items with zero churn in the chosen metric are dropped — this is an
 * explicit churn display. Returns the touched zones (metric-desc, for the
 * legend) and the tile cells.
 */
function layoutTreemapByZone(
  items: ChurnItem[],
  area: AreaMetric,
  width: number,
  height: number,
): { zones: Zone[]; cells: TreemapCell[] } {
  if (items.length === 0 || width <= 0 || height <= 0) {
    return { zones: [], cells: [] };
  }

  type Bucket = { zone: Zone; items: Array<{ item: ChurnItem; value: number }>; total: number };
  const bucketMap = new Map<Zone, Bucket>();
  for (const item of items) {
    const value = metricValue(item, area);
    if (value <= 0) continue;
    const b = bucketMap.get(item.zone) ?? { zone: item.zone, items: [], total: 0 };
    b.items.push({ item, value });
    b.total += value;
    bucketMap.set(item.zone, b);
  }
  const buckets = [...bucketMap.values()].sort((a, b) => b.total - a.total);

  // Outer pass: each item is a zone, value = zone's total churn.
  const zoneRects = squarify(
    buckets.map((b) => ({ value: b.total, payload: b })),
    0,
    0,
    width,
    height,
  );

  const cells: TreemapCell[] = [];
  for (const zr of zoneRects) {
    const b = zr.payload;
    if (zr.h <= 0 || zr.w <= 0) continue;
    // Inner pass: each item is a tile, value = its churn in the metric.
    const tileRects = squarify(
      b.items.map((it) => ({ value: it.value, payload: it.item })),
      zr.x,
      zr.y,
      zr.w,
      zr.h,
    );
    for (const tr of tileRects) {
      cells.push({ item: tr.payload, x: tr.x, y: tr.y, w: tr.w, h: tr.h });
    }
  }

  return { zones: buckets.map((b) => b.zone), cells };
}

interface SquarifyInput<T> {
  value: number;
  payload: T;
}
interface SquarifyOutput<T> {
  payload: T;
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * Squarified treemap (Bruls, Huijsen, Van Wijk 2000) generalized
 * to lay out into any rectangle. Sorts items by value desc, then
 * greedily packs rows whose worst aspect ratio doesn't degrade.
 */
function squarify<T>(
  items: SquarifyInput<T>[],
  x0: number,
  y0: number,
  width: number,
  height: number,
): SquarifyOutput<T>[] {
  if (items.length === 0 || width <= 0 || height <= 0) return [];
  const sorted = [...items].sort((a, b) => b.value - a.value);
  const totalValue = sorted.reduce((acc, i) => acc + i.value, 0);
  if (totalValue <= 0) return [];
  const totalArea = width * height;
  const scaled = sorted.map((i) => ({
    payload: i.payload,
    area: (i.value / totalValue) * totalArea,
  }));

  const out: SquarifyOutput<T>[] = [];
  let x = x0;
  let y = y0;
  let w = width;
  let h = height;
  let queue = scaled;

  while (queue.length > 0) {
    const shorter = Math.min(w, h);
    const row: typeof queue = [queue[0]!];
    queue = queue.slice(1);
    while (queue.length > 0) {
      const candidate = [...row, queue[0]!];
      if (worstRatio(candidate, shorter) <= worstRatio(row, shorter)) {
        row.push(queue[0]!);
        queue = queue.slice(1);
      } else {
        break;
      }
    }
    const rowTotal = row.reduce((acc, r) => acc + r.area, 0);
    const rowExtent = rowTotal / shorter;
    if (w >= h) {
      let cy = y;
      for (const r of row) {
        const cellH = r.area / rowExtent;
        out.push({ payload: r.payload, x, y: cy, w: rowExtent, h: cellH });
        cy += cellH;
      }
      x += rowExtent;
      w -= rowExtent;
    } else {
      let cx = x;
      for (const r of row) {
        const cellW = r.area / rowExtent;
        out.push({ payload: r.payload, x: cx, y, w: cellW, h: rowExtent });
        cx += cellW;
      }
      y += rowExtent;
      h -= rowExtent;
    }
  }
  return out;
}

/** Worst aspect ratio if `row` is laid along edge of length `shorter`. */
function worstRatio(row: { area: number }[], shorter: number): number {
  const s = row.reduce((acc, r) => acc + r.area, 0);
  if (s === 0) return Number.POSITIVE_INFINITY;
  let max = 0;
  let min = Number.POSITIVE_INFINITY;
  for (const r of row) {
    if (r.area > max) max = r.area;
    if (r.area < min) min = r.area;
  }
  const ss = s * s;
  const w2 = shorter * shorter;
  return Math.max((w2 * max) / ss, ss / (w2 * min));
}

function basename(p: string): string {
  const i = p.lastIndexOf("/");
  return i === -1 ? p : p.slice(i + 1);
}

function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  if (max <= 1) return s.slice(0, max);
  return s.slice(0, max - 1) + "…";
}

const card: React.CSSProperties = {
  color: "var(--text-primary)",
  display: "flex",
  flexDirection: "column",
  gap: 8,
};
const cardHeader: React.CSSProperties = {
  display: "flex",
  justifyContent: "space-between",
  alignItems: "center",
  flexWrap: "wrap",
  gap: 8,
};
const muted: React.CSSProperties = {
  color: "var(--text-muted)",
  fontSize: 11,
};
const legend: React.CSSProperties = {
  display: "flex",
  flexWrap: "wrap",
  gap: "4px 12px",
  fontSize: 11,
  color: "var(--text-secondary)",
};
const legendItem: React.CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: 5,
  fontFamily: "var(--font-mono, monospace)",
};
const legendSwatch: React.CSSProperties = {
  width: 10,
  height: 10,
  borderRadius: 2,
  flexShrink: 0,
  display: "inline-block",
};
const tab: React.CSSProperties = {
  padding: "3px 8px",
  background: "transparent",
  color: "var(--text-muted)",
  borderWidth: 1,
  borderStyle: "solid",
  borderColor: "var(--border-subtle)",
  borderRadius: 4,
  cursor: "pointer",
  fontSize: "var(--text-xs)",
};
const activeTab: React.CSSProperties = {
  ...tab,
  background: "var(--accent-soft-bg, var(--surface-app))",
  color: "var(--text-primary)",
  borderColor: "var(--text-link, #2563eb)",
};
