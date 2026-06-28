import { useMemo, useRef, useState, useEffect } from "react";
import type { BranchChangeEntry } from "../../api-types.js";
import { classifyZone, ZONE_COLORS, ZONE_LABELS, type Zone } from "./zones.js";

interface Props {
  files: BranchChangeEntry[];
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
}

/**
 * Squarified treemap of the change's file churn, grouped by
 * architectural zone (WinDirStat-style). Each touched zone gets one
 * contiguous block sized by its total churn; files within the zone are
 * laid out as a sub-treemap inside that block sharing the zone colour.
 *
 * Subsumes the old Architectural-zones bar (tsk350): the map IS the zone
 * breakdown, and a colour **legend** below names every touched zone —
 * which the per-tile labels can't, since small tiles drop their text.
 *
 * Renders inline SVG sized to the container width — no external graph
 * library dependency.
 */
export function ChangeTreemapCard({ files, onOpenFile }: Props) {
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

  const layout = useMemo(
    () => layoutTreemapByZone(files, containerWidth, 240),
    [files, containerWidth],
  );

  if (layout.cells.length === 0) {
    return null;
  }

  return (
    <div style={card} ref={ref}>
      <header style={cardHeader}>
        <h3 style={cardTitle}>Change treemap</h3>
        <span style={muted}>grouped by zone · area ∝ churn</span>
      </header>
      <svg
        width={containerWidth}
        height={240}
        style={{ display: "block" }}
        role="img"
        aria-label="Treemap of files by churn, grouped by architectural zone"
      >
        {layout.cells.map((cell) => (
          <g key={cell.file.path}>
            <rect
              x={cell.x}
              y={cell.y}
              width={cell.w}
              height={cell.h}
              fill={ZONE_COLORS[cell.zone]}
              stroke="var(--surface-card)"
              strokeWidth={1}
              onClick={(e) =>
                onOpenFile(cell.file.path, { newTab: e.metaKey || e.ctrlKey })
              }
              style={{ cursor: "pointer" }}
            >
              <title>
                {cell.file.path} ({ZONE_LABELS[cell.zone]}) · +
                {cell.file.additions ?? 0} −{cell.file.deletions ?? 0}
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
                {truncate(basename(cell.file.path), Math.floor(cell.w / 7))}
              </text>
            ) : null}
          </g>
        ))}
      </svg>
      {layout.zones.length > 0 ? (
        <div style={legend} aria-label="Architectural zone colors">
          {layout.zones.map((z) => (
            <span key={z} style={legendItem}>
              <span style={{ ...legendSwatch, background: ZONE_COLORS[z] }} />
              {ZONE_LABELS[z]}
            </span>
          ))}
        </div>
      ) : null}
    </div>
  );
}

interface TreemapCell {
  file: BranchChangeEntry;
  zone: Zone;
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * Two-level squarified treemap: outer pass packs zones by total churn,
 * inner pass packs each zone's files into its rect. Returns the touched
 * zones (churn-desc, for the legend) and the file cells (the fill rects).
 */
function layoutTreemapByZone(
  files: BranchChangeEntry[],
  width: number,
  height: number,
): { zones: Zone[]; cells: TreemapCell[] } {
  if (files.length === 0 || width <= 0 || height <= 0) {
    return { zones: [], cells: [] };
  }

  // Bucket files by zone, accumulating churn. Floor each file at 1
  // so rename-only / binary files still take a sliver of space.
  type Bucket = { zone: Zone; files: BranchChangeEntry[]; churn: number };
  const bucketMap = new Map<Zone, Bucket>();
  for (const f of files) {
    const z = classifyZone(f.path);
    const churn = Math.max(1, (f.additions ?? 0) + (f.deletions ?? 0));
    const entry = bucketMap.get(z) ?? { zone: z, files: [], churn: 0 };
    entry.files.push(f);
    entry.churn += churn;
    bucketMap.set(z, entry);
  }
  const buckets = [...bucketMap.values()].sort((a, b) => b.churn - a.churn);

  // Outer pass: each item is a zone, value = zone churn.
  const zoneRects = squarify(
    buckets.map((b) => ({ value: b.churn, payload: b })),
    0,
    0,
    width,
    height,
  );

  const cells: TreemapCell[] = [];
  for (const zr of zoneRects) {
    const b = zr.payload;
    if (zr.h <= 0 || zr.w <= 0) continue;

    // Inner pass: each item is a file, value = per-file churn. Files fill
    // the whole zone rect — no header band is reserved anymore.
    const fileRects = squarify(
      b.files.map((f) => ({
        value: Math.max(1, (f.additions ?? 0) + (f.deletions ?? 0)),
        payload: f,
      })),
      zr.x,
      zr.y,
      zr.w,
      zr.h,
    );
    for (const fr of fileRects) {
      cells.push({
        file: fr.payload,
        zone: b.zone,
        x: fr.x,
        y: fr.y,
        w: fr.w,
        h: fr.h,
      });
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
  alignItems: "baseline",
};
const cardTitle: React.CSSProperties = {
  margin: 0,
  fontSize: "var(--text-base, 14px)",
  fontWeight: 600,
  color: "var(--text-primary)",
};
const muted: React.CSSProperties = {
  color: "var(--text-secondary)",
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
