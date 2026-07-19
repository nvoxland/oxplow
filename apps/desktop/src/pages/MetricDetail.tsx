import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";

import type {
  EffortMetricDelta,
  FactFinding,
  MetricSpec,
  RollupRow,
  SeriesPoint,
} from "../api.js";
import { metricDimensionRollup } from "../api.js";
import { formatMetricValue, formatMetricValueExact } from "../components/format.js";
import {
  deltaColor,
  deltaSummary,
  fmtSigned,
} from "../components/EffortMetrics.js";
import { metricRecordingRef } from "../tabs/pageRefs.js";
import { useRouteDispatch } from "../tabs/RouteLink.js";
import type { TabRef } from "../tabs/tabState.js";
import {
  CHART_MODES,
  CHART_SCALES,
  type ChartMode,
  type ChartPoint,
  type ChartScale,
  RANGE_PRESETS,
  type TimeRange,
  breakdownDimensions,
  fromLocalInput,
  inRangeStat,
  matchPresetKey,
  rangeFromPreset,
  toLocalInput,
  yDomain,
} from "./metricDetailData.js";

// Composable pieces of the Metric detail page (tsk213, P4 / tsk232 / tsk291).
// The page (`MetricDetailPage`) lays these out: the right rail carries the
// stats, the main column carries the trend chart, the recordings table, and the
// kind-specific drill-in selected from `metric_definition.kind`.

// Every number on this page goes through the SHARED formatter (tsk183) — the
// rule in `.context/usability.md`. The local implementation this replaces did
// `toFixed(2)` with no locale grouping, no compaction, and no `ms`/`%`
// handling, so the same metric read one way here and another on Recorded
// Metrics (which always used the shared one). `unit` is optional only because a
// few call sites genuinely lack it in scope; pass it wherever you have it.
function fmt(v: number, unit?: string | null): string {
  return formatMetricValue(v, unit);
}

/** Short date+time label for an epoch-ms tick on the trend's time axis. */
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

/** Time-range + chart-mode + branch controls for the metric detail page. */
export function MetricControls({
  range,
  onRange,
  mode,
  onMode,
  scale,
  onScale,
  branch,
  branches,
  onBranch,
}: {
  range: TimeRange;
  onRange: (r: TimeRange) => void;
  mode: ChartMode;
  onMode: (m: ChartMode) => void;
  scale: ChartScale;
  onScale: (s: ChartScale) => void;
  branch: string | null;
  branches: string[];
  onBranch: (b: string | null) => void;
}) {
  const [customOpen, setCustomOpen] = useState(false);
  const presetKey = matchPresetKey(range, Date.now());
  // Custom inputs show when the user picks "Custom range…" or whenever the
  // active window doesn't match a preset (e.g. after a chart drag).
  const isCustom = customOpen || presetKey === "custom";
  // Stacked vertically — these live in the narrow (320px) Details rail.
  const selStyle = { fontSize: 12, width: "100%" } as const;
  const rowStyle = { display: "flex", flexDirection: "column", gap: 3 } as const;
  const labelStyle = { opacity: 0.6, fontSize: 12 } as const;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 10 }} data-testid="metric-controls">
      <div style={rowStyle}>
        <span style={labelStyle}>Range</span>
        <select
          value={isCustom ? "custom" : presetKey}
          onChange={(e) => {
            if (e.target.value === "custom") {
              setCustomOpen(true);
            } else {
              setCustomOpen(false);
              onRange(rangeFromPreset(e.target.value, Date.now()));
            }
          }}
          data-testid="range-preset"
          style={selStyle}
        >
          {RANGE_PRESETS.map((p) => (
            <option key={p.key} value={p.key}>
              {p.label}
            </option>
          ))}
          <option value="custom">Custom range…</option>
        </select>
        {isCustom ? (
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              gap: 4,
              border: "1px solid var(--border, #2a2a2a)",
              borderRadius: 4,
              padding: 6,
            }}
          >
            <label style={{ display: "flex", flexDirection: "column", gap: 2, fontSize: 11 }}>
              <span style={{ opacity: 0.6 }}>From</span>
              <input
                type="datetime-local"
                value={toLocalInput(range.from)}
                onChange={(e) => {
                  const from = fromLocalInput(e.target.value);
                  if (from != null) onRange({ from, to: range.to });
                }}
                data-testid="range-from"
                style={selStyle}
              />
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: 2, fontSize: 11 }}>
              <span style={{ opacity: 0.6 }}>To</span>
              <input
                type="datetime-local"
                value={toLocalInput(range.to)}
                onChange={(e) => {
                  const to = fromLocalInput(e.target.value);
                  if (to != null) onRange({ from: range.from, to });
                }}
                data-testid="range-to"
                style={selStyle}
              />
            </label>
          </div>
        ) : null}
      </div>
      <div style={rowStyle}>
        <span style={labelStyle}>Chart</span>
        <select value={mode} onChange={(e) => onMode(e.target.value as ChartMode)} data-testid="chart-mode" style={selStyle}>
          {CHART_MODES.map((m) => (
            <option key={m.key} value={m.key}>
              {m.label}
            </option>
          ))}
        </select>
      </div>
      <div style={rowStyle}>
        <span style={labelStyle}>Scale</span>
        <select
          value={scale}
          onChange={(e) => onScale(e.target.value as ChartScale)}
          title="Auto fits the data; From zero anchors the Y-axis at 0."
          data-testid="chart-scale"
          style={selStyle}
        >
          {CHART_SCALES.map((s) => (
            <option key={s.key} value={s.key}>
              {s.label}
            </option>
          ))}
        </select>
      </div>
      <div style={rowStyle}>
        <span style={labelStyle}>Branch</span>
        <select
          value={branch ?? ""}
          onChange={(e) => onBranch(e.target.value || null)}
          data-testid="branch-filter"
          style={selStyle}
        >
          <option value="">All branches</option>
          {branches.map((b) => (
            <option key={b} value={b}>
              {b}
            </option>
          ))}
        </select>
      </div>
    </div>
  );
}

const PAGE_SIZE = 25;

/** One recordings-table row. Each series point is one capture, so every row is
 *  clickable (browser-style via `useRouteDispatch`) → that capture's item-level
 *  findings drill-in. */
function RecordingRow({
  s,
  unit,
  metricKey,
  onOpenPage,
}: {
  s: SeriesPoint;
  unit?: string | null;
  metricKey?: string;
  onOpenPage?: (ref: TabRef) => void;
}) {
  const { handlers } = useRouteDispatch(
    metricRecordingRef(s.capture_id, {
      metricKey,
      capturedAt: String(s.captured_at),
      value: s.value,
    }),
    { onNavigate: onOpenPage },
  );
  return (
    <tr
      onClick={handlers.onClick}
      onAuxClick={handlers.onAuxClick}
      onContextMenu={handlers.onContextMenu}
      style={{ borderTop: "1px solid var(--border, #2a2a2a)", cursor: "pointer" }}
    >
      <td style={{ padding: "4px 8px", whiteSpace: "nowrap" }}>{new Date(String(s.captured_at)).toLocaleString()}</td>
      <td style={{ padding: "4px 8px", textAlign: "right", fontWeight: 600 }}>{fmt(s.value, unit)}</td>
      <td style={{ padding: "4px 8px", fontFamily: "monospace", fontSize: 11 }}>{s.branch ?? "—"}</td>
      <td style={{ padding: "4px 8px", fontFamily: "monospace", fontSize: 11 }}>
        {s.git_version ? s.git_version.slice(0, 8) : "—"}
      </td>
      <td
        style={{ padding: "4px 8px", opacity: s.provenance === "observed" ? 0.6 : 1 }}
        title={s.source ?? undefined}
      >
        {s.provenance === "observed" ? "observed" : `⚠ ${s.provenance ?? "?"}`}
      </td>
    </tr>
  );
}

/** The actual recordings — every sample, newest first, paginated. Rows with a
 *  run drill into that recording's item-level findings. */
export function RecordingsTable({
  samples,
  unit,
  metricKey,
  onOpenPage,
}: {
  samples: SeriesPoint[];
  unit?: string | null;
  metricKey?: string;
  onOpenPage?: (ref: TabRef) => void;
}) {
  const [page, setPage] = useState(0);
  // Reset to the first page whenever the (filtered) input set changes.
  useEffect(() => setPage(0), [samples]);

  if (samples.length === 0) return <div style={{ opacity: 0.6 }}>No recordings in range.</div>;
  const pageCount = Math.max(1, Math.ceil(samples.length / PAGE_SIZE));
  const cur = Math.min(page, pageCount - 1);
  const start = cur * PAGE_SIZE;
  const rows = samples.slice(start, start + PAGE_SIZE);
  const btn = {
    fontSize: 12,
    padding: "2px 8px",
    cursor: "pointer",
  } as const;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 12 }} data-testid="metric-recordings">
        <thead>
          <tr style={{ textAlign: "left", opacity: 0.6 }}>
            <th style={{ padding: "4px 8px" }}>Time</th>
            <th style={{ padding: "4px 8px", textAlign: "right" }}>Value</th>
            <th style={{ padding: "4px 8px" }}>Branch</th>
            <th style={{ padding: "4px 8px" }}>Version</th>
            <th style={{ padding: "4px 8px" }}>Trust</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((s) => (
            <RecordingRow key={s.capture_id} s={s} unit={unit} metricKey={metricKey} onOpenPage={onOpenPage} />
          ))}
        </tbody>
      </table>
      {samples.length > PAGE_SIZE ? (
        <div style={{ display: "flex", alignItems: "center", gap: 10, fontSize: 12 }} data-testid="recordings-pager">
          <button type="button" style={btn} disabled={cur === 0} onClick={() => setPage(cur - 1)}>
            ‹ Prev
          </button>
          <span style={{ opacity: 0.6 }}>
            {start + 1}–{Math.min(start + PAGE_SIZE, samples.length)} of {samples.length}
          </span>
          <button type="button" style={btn} disabled={cur >= pageCount - 1} onClick={() => setPage(cur + 1)}>
            Next ›
          </button>
        </div>
      ) : null}
    </div>
  );
}

function Stat({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div style={{ display: "flex", justifyContent: "space-between", gap: 12, fontSize: 13, padding: "3px 0" }}>
      <span style={{ opacity: 0.6 }}>{label}</span>
      <span style={{ textAlign: "right", minWidth: 0, overflow: "hidden", textOverflow: "ellipsis" }}>{children}</span>
    </div>
  );
}

/** Right-rail stats for the metric detail page: latest, change, type, id, … */
export function MetricStatsRail({
  def,
  samples,
  effort,
  effortDelta,
}: {
  def: MetricSpec;
  samples: SeriesPoint[];
  effort?: { effortId: string; start: string; end: string | null };
  effortDelta?: EffortMetricDelta | null;
}) {
  const latest = samples[0] ?? null;
  // The "in range" headline follows how the metric rolls up (Σ for sum metrics
  // like tokens, mean for avg, signed last−first for level gauges) — see
  // `inRangeStat` (tsk301).
  const rangeStat = inRangeStat(samples, def.aggregation);
  const rangeText = rangeStat
    ? rangeStat.signed
      ? `${rangeStat.value > 0 ? "+" : ""}${fmt(rangeStat.value, def.unit)}`
      : fmt(rangeStat.value, def.unit)
    : null;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 4 }} data-testid="metric-detail-stats">
      {rangeStat ? (
        <Stat label={rangeStat.label}>
          {/* Exact value on hover — the headline compacts at 10k, so the precise
              number has to stay reachable (tsk183). */}
          <strong title={formatMetricValueExact(rangeStat.value, def.unit)}>{rangeText}</strong>
        </Stat>
      ) : null}
      {/* Definition metadata — the full "what is this metric" block (tsk33). */}
      <Stat label="ID">
        <code style={{ fontSize: 11, wordBreak: "break-all" }}>{def.key}</code>
      </Stat>
      <Stat label="Type">{def.display_kind}</Stat>
      <Stat label="Aggregation">{def.aggregation}</Stat>
      {def.source_measure ? (
        <Stat label="Measure">
          <code style={{ fontSize: 11, wordBreak: "break-all" }}>{def.source_measure}</code>
        </Stat>
      ) : null}
      <Stat label="Scope">{def.scope}</Stat>
      {def.category ? <Stat label="Category">{def.category}</Stat> : null}
      {def.language ? <Stat label="Language">{def.language}</Stat> : null}
      {def.unit ? <Stat label="Unit">{def.unit}</Stat> : null}
      <Stat label="Direction">{def.direction}</Stat>
      {def.target != null ? <Stat label="Target">{fmt(def.target, def.unit)}</Stat> : null}
      {def.warn_at != null ? <Stat label="Warn at">{fmt(def.warn_at, def.unit)}</Stat> : null}
      {def.fail_at != null ? <Stat label="Fail at">{fmt(def.fail_at, def.unit)}</Stat> : null}
      {latest?.branch ? (
        <Stat label="Branch">
          <code style={{ fontSize: 11 }}>{latest.branch}</code>
        </Stat>
      ) : null}
      {effort && effortDelta ? (
        <div
          data-testid="metric-detail-effort"
          style={{
            marginTop: 8,
            border: "1px solid var(--border-subtle, #2a2a2a)",
            borderRadius: 6,
            padding: "8px 10px",
            display: "flex",
            flexDirection: "column",
            gap: 4,
            fontSize: 13,
          }}
        >
          <span style={{ fontWeight: 600 }}>In this effort</span>
          <span style={{ fontFamily: "var(--font-mono)" }}>{deltaSummary(effortDelta)}</span>
          {effortDelta.changed && effortDelta.delta != null && effortDelta.agg !== "sum" ? (
            <span style={{ color: deltaColor(effortDelta) }}>Δ {fmtSigned(effortDelta.delta)}</span>
          ) : null}
          {effortDelta.attributed_files != null && effortDelta.attributed_files > 0 ? (
            <span style={{ opacity: 0.6 }}>
              across {effortDelta.attributed_files}{" "}
              {effortDelta.attributed_files === 1 ? "file" : "files"}
            </span>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function FindingsTable({ findings }: { findings: FactFinding[] }) {
  if (findings.length === 0) return <div style={{ opacity: 0.6 }}>No findings in the latest recording.</div>;
  return (
    <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 12 }}>
      <thead>
        <tr style={{ textAlign: "left", opacity: 0.6 }}>
          <th style={{ padding: "4px 8px" }}>Location</th>
          <th style={{ padding: "4px 8px" }}>Severity</th>
          <th style={{ padding: "4px 8px" }}>Rule</th>
          <th style={{ padding: "4px 8px" }}>Message</th>
        </tr>
      </thead>
      <tbody>
        {findings.map((r, i) => (
          <tr key={i} style={{ borderTop: "1px solid var(--border, #2a2a2a)" }}>
            <td style={{ padding: "4px 8px", fontFamily: "monospace", fontSize: 11 }}>
              {r.path ?? r.subject_ref ?? "—"}
              {r.line != null ? `:${r.line}` : ""}
            </td>
            <td style={{ padding: "4px 8px" }}>{r.severity ?? "—"}</td>
            <td style={{ padding: "4px 8px", fontFamily: "monospace", fontSize: 11 }}>{r.rule ?? "—"}</td>
            <td style={{ padding: "4px 8px" }}>{r.message ?? "—"}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** Top subjects behind an `event`-kind metric — the server rolls the metric's
 *  facts up by `subject` (largest first), replacing the old client-side
 *  `topSubjects` over samples (epic tsk12, T-C3). */
function TopSubjects({ metricKey }: { metricKey: string }) {
  const [rows, setRows] = useState<RollupRow[]>([]);
  useEffect(() => {
    let cancelled = false;
    void metricDimensionRollup(metricKey, "subject").then((r) => {
      if (!cancelled) setRows(r.slice(0, 10));
    });
    return () => {
      cancelled = true;
    };
  }, [metricKey]);
  if (rows.length === 0) return <div style={{ opacity: 0.6 }}>No subject breakdown.</div>;
  const max = Math.max(...rows.map((t) => t.value)) || 1;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
      {rows.map((t) => (
        <div key={t.key} style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12 }}>
          <span style={{ width: 200, fontFamily: "monospace", fontSize: 11, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {t.key}
          </span>
          <span style={{ flex: 1, background: "var(--border, #2a2a2a)", borderRadius: 2, height: 12 }}>
            <span style={{ display: "block", width: `${(t.value / max) * 100}%`, background: "var(--accent, #58a6ff)", height: 12, borderRadius: 2 }} />
          </span>
          <span style={{ width: 56, textAlign: "right" }}>{fmt(t.value)}</span>
        </div>
      ))}
    </div>
  );
}

/** Kind-specific drill-in for the latest recording, or null for plain gauges.
 *  Findings/test/coverage metrics render the uniform per-item finding view over
 *  the recording's facts (`findings_for_spec`); `event` metrics show the
 *  server-rolled subject breakdown (epic tsk12, T-C3). */
export function KindDrillIn({
  def,
  findings,
  metricKey,
}: {
  def: MetricSpec;
  findings: FactFinding[];
  metricKey: string;
}): ReactNode {
  switch (def.display_kind) {
    case "findings":
    case "test":
    case "coverage":
      return <FindingsTable findings={findings} />;
    case "event":
      return <TopSubjects metricKey={metricKey} />;
    default:
      return null;
  }
}

// `breakdownDimensions` moved to `metricDetailData.ts` (tsk150) so the
// dashboard's breakout picker reads the same rule this card does.

/** Breakdown card: roll the metric's latest per-file values up by a chosen
 *  dimension (package / language / …) and render a horizontal bar list,
 *  largest first. Exercises the `metric_subject` package grain + the per-file
 *  dim breakdown (tsk328 package / tsk319 language). Self-hides when the
 *  metric has no per-file samples (e.g. coverage, operational metrics). */
export function MetricBreakdownCard({
  def,
  onAvailability,
  onSelectGroup,
  onDimChange,
  activeGroup,
}: {
  def: MetricSpec;
  /** Reports whether the current dimension's roll-up returned any rows — the
   *  tab wrapper uses it to decide whether to offer a Breakdown tab (tsk134). */
  onAvailability?: (has: boolean) => void;
  /** Click a row to filter the trend chart to that dim value (tsk136). */
  onSelectGroup?: (dim: string, value: string) => void;
  /** Switching the dimension clears any active chart filter (it was on the old dim). */
  onDimChange?: () => void;
  /** The currently charted group value (for the row highlight), or null. */
  activeGroup?: string | null;
}) {
  const dims = useMemo(() => breakdownDimensions(def), [def]);
  const [dim, setDim] = useState<string>(dims[0] ?? "package");
  const [rows, setRows] = useState<RollupRow[]>([]);
  useEffect(() => {
    let cancelled = false;
    void metricDimensionRollup(def.key, dim).then((r) => {
      if (cancelled) return;
      setRows(r);
      onAvailability?.(r.length > 0);
    });
    return () => {
      cancelled = true;
    };
  }, [def.key, dim, onAvailability]);

  // Hide the card only when there is nothing to break down BY — a metric that
  // declares no sliceable dimensions (coverage, most operational metrics).
  //
  // Emphatically NOT when the chosen dimension returned no rows (tsk181): the
  // early return used to sit below this and above the picker, so an empty
  // result unmounted the card *including its own <select>*, leaving the tab
  // selected, blank, and with no control to choose a different dimension —
  // recoverable only by closing the tab. Any dimension can come back empty for
  // the current range or branch, so that trap was reachable with a perfectly
  // valid choice.
  if (dims.length === 0) return null;
  const max = Math.max(...rows.map((r) => r.value), 1);
  const dimLabel = dim.charAt(0).toUpperCase() + dim.slice(1);
  const valueLabel = "Value";
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }} data-testid="metric-breakdown">
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <div
          style={{
            fontSize: 12,
            fontWeight: 600,
            opacity: 0.6,
            textTransform: "uppercase",
            letterSpacing: "0.04em",
          }}
        >
          Breakdown by
        </div>
        <select
          value={dim}
          onChange={(e) => {
            setDim(e.target.value);
            onDimChange?.();
          }}
          aria-label="Breakdown dimension"
          style={{ fontSize: 12 }}
        >
          {dims.map((d) => (
            <option key={d} value={d}>
              {d}
            </option>
          ))}
        </select>
      </div>
      {rows.length === 0 ? (
        <div
          data-testid="breakdown-empty"
          style={{ fontSize: 12, opacity: 0.6, padding: "6px 2px" }}
        >
          No data for <strong>{dim}</strong> in the selected range. Pick another
          dimension above, or widen the range.
        </div>
      ) : (
        <>
      {/* Column header — the value/count numbers are otherwise unlabeled. */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          fontSize: 10,
          opacity: 0.5,
          textTransform: "uppercase",
          letterSpacing: "0.04em",
          borderBottom: "1px solid var(--border, #2a2a2a)",
          paddingBottom: 2,
        }}
      >
        <span style={{ width: "32%" }}>{dimLabel}</span>
        <div style={{ flex: 1 }} />
        <span style={{ width: 64, textAlign: "right" }} title={`Rolled-up ${valueLabel} for the group`}>
          {valueLabel}
        </span>
        <span style={{ width: 56, textAlign: "right" }} title="Number of subjects (functions / files) in the group">
          Subjects
        </span>
      </div>
      {rows.slice(0, 20).map((r) => {
        const active = r.key === activeGroup;
        return (
        <div
          key={r.key}
          onClick={onSelectGroup ? () => onSelectGroup(dim, r.key) : undefined}
          data-testid={`breakdown-row-${r.key}`}
          title={
            onSelectGroup
              ? `${r.key}: ${fmt(r.value, def.unit)} across ${r.subject_count} subject${r.subject_count === 1 ? "" : "s"} — click to chart this ${dim}`
              : `${r.key}: ${fmt(r.value, def.unit)} across ${r.subject_count} subject${r.subject_count === 1 ? "" : "s"}`
          }
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            fontSize: 12,
            cursor: onSelectGroup ? "pointer" : undefined,
            background: active ? "var(--accent-bg, rgba(88,166,255,0.12))" : undefined,
            borderRadius: 3,
            padding: "1px 3px",
            margin: "0 -3px",
          }}
        >
          <span
            style={{
              width: "32%",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
              fontWeight: active ? 600 : undefined,
            }}
          >
            {r.key}
          </span>
          <div style={{ flex: 1, background: "var(--border, #2a2a2a)", borderRadius: 3, height: 12 }}>
            <div
              style={{
                width: `${(r.value / max) * 100}%`,
                background: "var(--accent, #58a6ff)",
                height: 12,
                borderRadius: 3,
              }}
            />
          </div>
          <span style={{ width: 64, textAlign: "right", fontVariantNumeric: "tabular-nums" }}>{fmt(r.value, def.unit)}</span>
          <span style={{ width: 56, textAlign: "right", fontVariantNumeric: "tabular-nums", opacity: 0.55 }}>
            {r.subject_count}
          </span>
        </div>
        );
      })}
      {rows.length > 20 ? (
        <div style={{ fontSize: 11, opacity: 0.5, paddingTop: 2 }}>
          +{rows.length - 20} more {dim === "package" ? "packages" : `${dim} values`}
        </div>
      ) : null}
        </>
      )}
    </div>
  );
}
