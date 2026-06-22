import { useEffect, useMemo, useState } from "react";

import {
  type MetricDefinition,
  type MetricSample,
  listMetricSamples,
} from "../api.js";

// Inline-SVG multi-series chart — no charting lib, matching the codebase's
// other inline-SVG visuals (sparkline, treemap). The Explorer (epic tsk213, P4)
// overlays several measures on one time axis, optionally grouped by a conformed
// dimension, so the agent/user can drill across metrics — e.g. token cost by
// model, complexity by language.

const SERIES_COLORS = [
  "#58a6ff",
  "#3fb950",
  "#e5a50a",
  "#bc8cff",
  "#f85149",
  "#39c5cf",
  "#db61a2",
  "#d29922",
];

type Point = { t: number; v: number };
type Series = { label: string; color: string; points: Point[] };

/** Group-by options offered on top of any declared conformed dimensions. */
const BASE_GROUP_BYS = ["none", "branch", "subject"] as const;

export function dimsValue(s: MetricSample, key: string): string | null {
  if (key === "branch") return s.branch ?? null;
  if (key === "subject") return s.subject_ref ?? s.subject_kind ?? null;
  if (s.dims_json) {
    try {
      const o = JSON.parse(s.dims_json) as Record<string, unknown>;
      const v = o[key];
      if (v != null) return String(v);
    } catch {
      /* ignore */
    }
  }
  // Fall back to subject_ref for entity-style dims (e.g. model:opus).
  if (key === "model" && s.subject_kind === "model") return s.subject_ref ?? null;
  return null;
}

/** Bucket the selected metrics' samples into one chart series per
 *  (measure × group-value). Pure — the component just renders the result. */
export function buildExplorerSeries(
  selected: string[],
  samplesByKey: Record<string, MetricSample[]>,
  groupBy: string,
  defs: MetricDefinition[],
): Series[] {
  const out: Series[] = [];
  let ci = 0;
  for (const key of selected) {
    const def = defs.find((d) => d.key === key);
    const samples = samplesByKey[key] ?? [];
    const buckets = new Map<string, Point[]>();
    for (const s of samples) {
      const g = groupBy === "none" ? null : dimsValue(s, groupBy);
      if (groupBy !== "none" && g == null) continue;
      const label = g == null ? (def?.title ?? key) : `${def?.title ?? key} · ${g}`;
      const t = Date.parse(String(s.captured_at));
      if (Number.isNaN(t)) continue;
      (buckets.get(label) ?? buckets.set(label, []).get(label)!).push({ t, v: s.value });
    }
    for (const [label, points] of buckets) {
      out.push({ label, color: SERIES_COLORS[ci % SERIES_COLORS.length]!, points });
      ci += 1;
    }
  }
  return out;
}

function MultiLineChart({
  series,
  target,
  kind,
}: {
  series: Series[];
  target?: number | null;
  kind: "line" | "bar";
}) {
  const w = 760;
  const h = 260;
  const padL = 44;
  const padR = 12;
  const padT = 12;
  const padB = 26;
  const all = series.flatMap((s) => s.points);
  if (all.length === 0) {
    return <div style={{ opacity: 0.6, padding: 24 }}>No samples for the selected measures.</div>;
  }
  const tMin = Math.min(...all.map((p) => p.t));
  const tMax = Math.max(...all.map((p) => p.t));
  const vMin = Math.min(0, ...all.map((p) => p.v));
  const vMaxRaw = Math.max(...all.map((p) => p.v), target ?? -Infinity);
  const vMax = vMaxRaw === vMin ? vMin + 1 : vMaxRaw;
  const tRange = tMax - tMin || 1;
  const vRange = vMax - vMin || 1;
  const x = (t: number) => padL + ((t - tMin) / tRange) * (w - padL - padR);
  const y = (v: number) => h - padB - ((v - vMin) / vRange) * (h - padT - padB);

  return (
    <svg width={w} height={h} style={{ display: "block", maxWidth: "100%" }} role="img" aria-label="metric chart">
      {/* axes */}
      <line x1={padL} y1={padT} x2={padL} y2={h - padB} stroke="var(--border, #2a2a2a)" />
      <line x1={padL} y1={h - padB} x2={w - padR} y2={h - padB} stroke="var(--border, #2a2a2a)" />
      {/* y ticks */}
      {[0, 0.5, 1].map((f) => {
        const v = vMin + f * (vMax - vMin);
        return (
          <g key={f}>
            <text x={padL - 6} y={y(v) + 3} textAnchor="end" fontSize={9} fill="var(--text-muted, #888)">
              {Number.isInteger(v) ? v : v.toFixed(1)}
            </text>
            <line x1={padL} y1={y(v)} x2={w - padR} y2={y(v)} stroke="var(--border, #2a2a2a)" opacity={0.3} />
          </g>
        );
      })}
      {/* target band */}
      {target != null ? (
        <line
          x1={padL}
          y1={y(target)}
          x2={w - padR}
          y2={y(target)}
          stroke="var(--ok, #3fb950)"
          strokeDasharray="4 3"
          opacity={0.7}
        />
      ) : null}
      {/* series */}
      {series.map((s, si) => {
        const pts = s.points.slice().sort((a, b) => a.t - b.t);
        if (kind === "bar") {
          const bw = Math.max(2, (w - padL - padR) / (pts.length * series.length + 1));
          return pts.map((p, i) => (
            <rect
              key={`${si}-${i}`}
              x={x(p.t) + si * bw - (series.length * bw) / 2}
              y={y(p.v)}
              width={bw - 0.5}
              height={h - padB - y(p.v)}
              fill={s.color}
              opacity={0.85}
            />
          ));
        }
        const d = pts.map((p, i) => `${i === 0 ? "M" : "L"}${x(p.t).toFixed(1)},${y(p.v).toFixed(1)}`).join(" ");
        return (
          <g key={si}>
            <path d={d} fill="none" stroke={s.color} strokeWidth={1.5} />
            {pts.map((p, i) => (
              <circle key={i} cx={x(p.t)} cy={y(p.v)} r={1.8} fill={s.color} />
            ))}
          </g>
        );
      })}
    </svg>
  );
}

/**
 * Metrics Explorer (epic tsk213, P4): overlay several measures on one time axis,
 * optionally grouped by a conformed dimension. Reads the same `metric_sample`
 * facts as the catalog (no bespoke per-metric code) — the whole point of the
 * substrate. Built over `listMetricSamples`; no new backend, no charting lib.
 */
export function MetricsExplorer({ defs }: { defs: MetricDefinition[] }) {
  const [selected, setSelected] = useState<string[]>([]);
  const [groupBy, setGroupBy] = useState<string>("none");
  const [viz, setViz] = useState<"line" | "bar">("line");
  const [samplesByKey, setSamplesByKey] = useState<Record<string, MetricSample[]>>({});

  // Default to the first metric so the chart isn't empty on first paint.
  useEffect(() => {
    if (selected.length === 0 && defs.length > 0) setSelected([defs[0]!.key]);
  }, [defs, selected.length]);

  useEffect(() => {
    let cancelled = false;
    void Promise.all(
      selected.map(async (key) => [key, await listMetricSamples(key, 200)] as const),
    ).then((pairs) => {
      if (!cancelled) setSamplesByKey(Object.fromEntries(pairs));
    });
    return () => {
      cancelled = true;
    };
  }, [selected]);

  // Group-by options = base dims ∪ the declared dimensions of the selected metrics.
  const groupOptions = useMemo(() => {
    const declared = new Set<string>();
    for (const key of selected) {
      const def = defs.find((d) => d.key === key);
      if (def?.dimensions_json) {
        try {
          for (const d of JSON.parse(def.dimensions_json) as string[]) declared.add(d);
        } catch {
          /* ignore */
        }
      }
    }
    return [...BASE_GROUP_BYS, ...[...declared].filter((d) => !BASE_GROUP_BYS.includes(d as never))];
  }, [selected, defs]);

  const series = useMemo<Series[]>(
    () => buildExplorerSeries(selected, samplesByKey, groupBy, defs),
    [selected, samplesByKey, groupBy, defs],
  );

  // A single target line only makes sense for one selected measure.
  const target = selected.length === 1 ? (defs.find((d) => d.key === selected[0])?.target ?? null) : null;

  const toggle = (key: string) =>
    setSelected((prev) => (prev.includes(key) ? prev.filter((k) => k !== key) : [...prev, key]));

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
      <div style={{ display: "flex", gap: 16, flexWrap: "wrap", alignItems: "flex-start" }}>
        {/* measure picker */}
        <div style={{ display: "flex", flexDirection: "column", gap: 3, maxHeight: 180, overflow: "auto", minWidth: 220 }}>
          <div style={{ opacity: 0.6, fontSize: 11, textTransform: "uppercase" }}>Measures</div>
          {defs.map((d) => (
            <label key={d.key} style={{ display: "flex", gap: 6, alignItems: "center", fontSize: 12, cursor: "pointer" }}>
              <input type="checkbox" checked={selected.includes(d.key)} onChange={() => toggle(d.key)} />
              <span>{d.title}</span>
            </label>
          ))}
        </div>
        {/* controls */}
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <label style={{ fontSize: 12, display: "flex", gap: 6, alignItems: "center" }}>
            <span style={{ opacity: 0.6 }}>Group by</span>
            <select value={groupBy} onChange={(e) => setGroupBy(e.target.value)} data-testid="explorer-groupby">
              {groupOptions.map((g) => (
                <option key={g} value={g}>
                  {g}
                </option>
              ))}
            </select>
          </label>
          <label style={{ fontSize: 12, display: "flex", gap: 6, alignItems: "center" }}>
            <span style={{ opacity: 0.6 }}>Chart</span>
            <select value={viz} onChange={(e) => setViz(e.target.value as "line" | "bar")} data-testid="explorer-viz">
              <option value="line">line</option>
              <option value="bar">bar</option>
            </select>
          </label>
        </div>
      </div>
      <MultiLineChart series={series} target={target} kind={viz} />
      {/* legend */}
      <div style={{ display: "flex", gap: 12, flexWrap: "wrap", fontSize: 11 }}>
        {series.map((s) => (
          <span key={s.label} style={{ display: "flex", gap: 4, alignItems: "center" }}>
            <span style={{ width: 10, height: 10, background: s.color, borderRadius: 2 }} />
            {s.label}
          </span>
        ))}
      </div>
    </div>
  );
}
