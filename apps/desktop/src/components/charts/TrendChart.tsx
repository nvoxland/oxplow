/**
 * A single-series trend line with labeled time (x) and value (y) axes.
 * Shared by the metric pages, dashboard tiles and `line` lenses.
 */
import { useRef, useState } from "react";

import { formatMetricValue } from "../format.js";
import { type ChartPoint, type ChartScale, yDomain } from "../../pages/metricDetailData.js";

function fmt(v: number, unit?: string | null): string {
  return formatMetricValue(v, unit);
}

/** Short date+time label for an epoch-ms tick on the time axis. */
function fmtTick(t: number): string {
  return new Date(t).toLocaleString(undefined, {
    month: "numeric",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

/** Single-series trend line over already-transformed points, with a labeled
 *  time (x) axis and value (y) axis. Dragging horizontally calls
 *  `onSelectRange` with the [from,to] epoch-ms span dragged across. */
export function TrendChart({
  points: pts,
  target,
  onSelectRange,
  domain,
  unit,
  scale = "auto",
  width = 760,
  height = 220,
}: {
  points: ChartPoint[];
  target?: number | null;
  onSelectRange?: (from: number, to: number) => void;
  /** Time-axis span. When set, the x axis covers this whole window (e.g. the
   *  selected range) rather than just the first→last sample. */
  domain?: { from: number; to: number };
  /** Unit appended to the hover tooltip's value. */
  unit?: string | null;
  /** Y-axis scaling: `auto` fits the data (default), `zero` forces through 0. */
  scale?: ChartScale;
  /** SVG coordinate-space size. The chart scales to its container via the
   *  viewBox, so these also set the **effective text scale**: a 760-wide chart
   *  squeezed into a 310px dashboard tile shrinks its tick labels ~2.4× and
   *  they stop being readable. Callers rendering small (tiles) pass their own
   *  size so the drawing sits near 1:1 (tsk144). */
  width?: number;
  height?: number;
}) {
  const svgRef = useRef<SVGSVGElement | null>(null);
  // Drag selection in SVG-x coordinates (null = not dragging).
  const [drag, setDrag] = useState<{ x0: number; x1: number } | null>(null);
  // Index of the point under the pointer (hover-to-inspect), or null.
  const [hoverI, setHoverI] = useState<number | null>(null);

  const w = width;
  const h = height;
  // A compact chart can't afford the full y-label gutter or x-label strip.
  const compact = w < 520;
  const padL = compact ? 34 : 44;
  const padR = compact ? 8 : 12;
  const padT = 10;
  const padB = compact ? 24 : 36;

  if (pts.length < 2) {
    return <div style={{ opacity: 0.6, padding: 16 }}>Not enough samples to chart yet.</div>;
  }
  // Time axis spans the full window when a `domain` is given (so a series that
  // stops short of "now" still plots against the whole range); otherwise the
  // data extent.
  const tMin = domain ? domain.from : Math.min(...pts.map((p) => p.t));
  const tMax = domain ? domain.to : Math.max(...pts.map((p) => p.t));
  // Y-axis fits the data (auto) or is anchored at 0 (zero) — see `yDomain`.
  const { min: vMin, max: vMax } = yDomain(
    pts.map((p) => p.v),
    target,
    scale,
  );
  const tRange = tMax - tMin || 1;
  const vRange = vMax - vMin || 1;
  // Y-tick label precision scaled to the visible range — a tight auto-scaled
  // window (e.g. 1.94–1.97) needs decimals a fixed `.toFixed(1)` would collapse
  // to "1.9"/"2.0".
  const tickDecimals = vRange >= 10 ? 0 : vRange >= 1 ? 1 : vRange >= 0.1 ? 2 : 3;
  // NB: named `fmtYTick`, not `fmtTick` — an earlier revision called this
  // `fmtTick` and shadowed the module-level *time* formatter of that name, so
  // the x axis and the hover tooltip rendered raw epoch ms (tsk144).
  const fmtYTick = (v: number) => (tickDecimals === 0 ? String(Math.round(v)) : v.toFixed(tickDecimals));
  const x = (t: number) => padL + ((t - tMin) / tRange) * (w - padL - padR);
  const y = (v: number) => h - padB - ((v - vMin) / vRange) * (h - padT - padB);
  // Inverse of `x`: SVG-x pixel → time, clamped to the plot area.
  const timeAt = (svgX: number) => {
    const clamped = Math.max(padL, Math.min(w - padR, svgX));
    return tMin + ((clamped - padL) / (w - padL - padR)) * tRange;
  };
  // Pointer clientX → SVG-x (the svg renders scaled via maxWidth:100%).
  const toSvgX = (clientX: number) => {
    const rect = svgRef.current?.getBoundingClientRect();
    if (!rect || rect.width === 0) return padL;
    return (clientX - rect.left) * (w / rect.width);
  };
  const d = pts.map((p, i) => `${i === 0 ? "M" : "L"}${x(p.t).toFixed(1)},${y(p.v).toFixed(1)}`).join(" ");
  // Date labels are wide; a compact chart only has room for the two endpoints.
  const tickCount = compact ? 2 : 4;
  const ticks = Array.from({ length: tickCount }, (_, i) => tMin + (i / (tickCount - 1)) * tRange);

  // Nearest point (by x) to a given SVG-x — drives the hover tooltip.
  const nearestIndex = (svgX: number) => {
    let best = 0;
    let bestD = Infinity;
    for (let i = 0; i < pts.length; i++) {
      const dx = Math.abs(x(pts[i]!.t) - svgX);
      if (dx < bestD) {
        bestD = dx;
        best = i;
      }
    }
    return best;
  };

  const endDrag = () => {
    if (drag && onSelectRange && Math.abs(drag.x1 - drag.x0) > 4) {
      const a = timeAt(drag.x0);
      const b = timeAt(drag.x1);
      onSelectRange(Math.min(a, b), Math.max(a, b));
    }
    setDrag(null);
  };

  return (
    <svg
      ref={svgRef}
      // A viewBox (not fixed width/height alone) so the chart SCALES to the
      // container width instead of clipping its right edge — and the hover
      // tooltip near the edge scales with it (tsk300).
      viewBox={`0 0 ${w} ${h}`}
      preserveAspectRatio="xMidYMid meet"
      style={{
        display: "block",
        width: "100%",
        height: "auto",
        maxWidth: w,
        // Only a range-selectable chart is draggable; a read-only tile chart
        // shouldn't advertise a crosshair.
        cursor: onSelectRange ? "crosshair" : "default",
        userSelect: "none",
      }}
      role="img"
      aria-label="metric trend"
      onPointerDown={
        onSelectRange
          ? (e) => {
              (e.target as Element).setPointerCapture?.(e.pointerId);
              const sx = toSvgX(e.clientX);
              setDrag({ x0: sx, x1: sx });
            }
          : undefined
      }
      onPointerMove={(e) => {
        const sx = toSvgX(e.clientX);
        // Dragging takes precedence — extend the selection and hide the tooltip;
        // otherwise track the nearest point for hover inspection.
        if (drag) {
          setDrag((d0) => (d0 ? { ...d0, x1: sx } : null));
          setHoverI(null);
        } else {
          setHoverI(nearestIndex(sx));
        }
      }}
      onPointerUp={onSelectRange ? endDrag : undefined}
      onPointerLeave={() => setHoverI(null)}
      onPointerCancel={() => {
        setDrag(null);
        setHoverI(null);
      }}
    >
      <line x1={padL} y1={padT} x2={padL} y2={h - padB} stroke="var(--border, #2a2a2a)" />
      <line x1={padL} y1={h - padB} x2={w - padR} y2={h - padB} stroke="var(--border, #2a2a2a)" />
      {[0, 0.5, 1].map((fr) => {
        const v = vMin + fr * (vMax - vMin);
        return (
          <g key={fr}>
            <text x={padL - 6} y={y(v) + 3} textAnchor="end" fontSize={9} fill="var(--text-muted, #888)">
              {fmtYTick(v)}
            </text>
            <line x1={padL} y1={y(v)} x2={w - padR} y2={y(v)} stroke="var(--border, #2a2a2a)" opacity={0.3} />
          </g>
        );
      })}
      {ticks.map((t, i) => {
        const anchor = i === 0 ? "start" : i === ticks.length - 1 ? "end" : "middle";
        return (
          <g key={i}>
            <line x1={x(t)} y1={h - padB} x2={x(t)} y2={h - padB + 4} stroke="var(--border, #2a2a2a)" />
            <text
              x={x(t)}
              y={h - padB + 15}
              textAnchor={anchor}
              fontSize={9}
              fill="var(--text-muted, #888)"
            >
              {fmtTick(t)}
            </text>
          </g>
        );
      })}
      {target != null ? (
        <line x1={padL} y1={y(target)} x2={w - padR} y2={y(target)} stroke="var(--ok, #3fb950)" strokeDasharray="4 3" opacity={0.7} />
      ) : null}
      <path d={d} fill="none" stroke="var(--accent, #58a6ff)" strokeWidth={1.5} />
      {pts.map((p, i) => (
        <circle key={i} cx={x(p.t)} cy={y(p.v)} r={1.8} fill="var(--accent, #58a6ff)" />
      ))}
      {drag && Math.abs(drag.x1 - drag.x0) > 1 ? (
        <rect
          x={Math.min(drag.x0, drag.x1)}
          y={padT}
          width={Math.abs(drag.x1 - drag.x0)}
          height={h - padT - padB}
          fill="var(--accent, #58a6ff)"
          opacity={0.15}
        />
      ) : null}
      {hoverI != null && !drag && pts[hoverI]
        ? (() => {
            const p = pts[hoverI]!;
            const px = x(p.t);
            const py = y(p.v);
            const valLbl = fmt(p.v, unit);
            const timeLbl = fmtTick(p.t);
            const boxW = Math.max(valLbl.length, timeLbl.length) * 6.2 + 12;
            const boxH = 32;
            // Prefer the right of the point; flip left near the edge; clamp.
            let bx = px + 10;
            if (bx + boxW > w - padR) bx = px - 10 - boxW;
            bx = Math.max(padL, Math.min(bx, w - padR - boxW));
            return (
              <g pointerEvents="none">
                <line x1={px} y1={padT} x2={px} y2={h - padB} stroke="var(--text-muted, #888)" opacity={0.4} strokeDasharray="3 3" />
                <circle cx={px} cy={py} r={3.5} fill="var(--accent, #58a6ff)" stroke="var(--surface-card, #111)" strokeWidth={1.5} />
                <rect x={bx} y={padT} width={boxW} height={boxH} rx={4} fill="var(--surface-card, #1c1c1c)" stroke="var(--border, #2a2a2a)" />
                <text x={bx + 6} y={padT + 14} fontSize={11} fontWeight={600} fill="var(--text, #ddd)">
                  {valLbl}
                </text>
                <text x={bx + 6} y={padT + 26} fontSize={9} fill="var(--text-muted, #888)">
                  {timeLbl}
                </text>
              </g>
            );
          })()
        : null}
    </svg>
  );
}
