import { type ReactNode, useEffect, useState } from "react";

import type { MetricSpec, SeriesPoint } from "../api.js";
import { formatMetricValue, formatMetricValueExact } from "../components/format.js";
import {
  RANGE_PRESETS,
  type TimeRange,
  fromLocalInput,
  inRangeStat,
  matchPresetKey,
  rangeFromPreset,
  toLocalInput,
} from "./metricDetailData.js";

// Composable pieces of the Metric detail page. The page (`MetricDetailPage`)
// lays these out: the right rail carries the range/branch controls and the
// stats, the main column the trend chart and the recordings table.

// Every number on this page goes through the SHARED formatter (tsk183) — the
// rule in `.context/usability.md`. The local implementation this replaces did
// `toFixed(2)` with no locale grouping, no compaction, and no `ms`/`%`
// handling, so the same metric read one way here and another on Metrics
// (which always used the shared one). `unit` is optional only because a
// few call sites genuinely lack it in scope; pass it wherever you have it.
function fmt(v: number, unit?: string | null): string {
  return formatMetricValue(v, unit);
}

export { TrendChart } from "../components/charts/TrendChart.js";

/** Time-range + branch controls for the metric detail page. */
export function MetricControls({
  range,
  onRange,
  branch,
  branches,
  onBranch,
}: {
  range: TimeRange;
  onRange: (r: TimeRange) => void;
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

/** One recordings-table row: one capture of the metric. */
function RecordingRow({ s, unit }: { s: SeriesPoint; unit?: string | null }) {
  return (
    <tr style={{ borderTop: "1px solid var(--border, #2a2a2a)" }}>
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

/** The actual recordings — every sample, newest first, paginated. */
export function RecordingsTable({ samples, unit }: { samples: SeriesPoint[]; unit?: string | null }) {
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
            <RecordingRow key={s.capture_id} s={s} unit={unit} />
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

/** Right-rail stats for the metric detail page: in-range value, type, id, … */
export function MetricStatsRail({ def, samples }: { def: MetricSpec; samples: SeriesPoint[] }) {
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
    </div>
  );
}
