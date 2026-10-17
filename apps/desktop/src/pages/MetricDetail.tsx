import { Facts, type Fact } from "../kit/Facts.js";
import { useEffect, useState } from "react";

import type { MetricSpec, SeriesPoint } from "../api.js";
import { formatMetricValue, formatMetricValueExact } from "../components/format.js";
import { DataTable, type DataColumn } from "../kit/index.js";
import {
  RANGE_PRESETS,
  type TimeRange,
  type UncheckedFile,
  exclusionReason,
  fromLocalInput,
  inRangeStat,
  specAggregation,
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
  slice,
}: {
  range: TimeRange;
  onRange: (r: TimeRange) => void;
  branch: string | null;
  branches: string[];
  onBranch: (b: string | null) => void;
  /** Slice By (P6.F1): the dimensions a metric can be grouped by, the one
   *  chosen, and the change. */
  slice?: { dimensions: { key: string; label: string }[]; value: string | null; onChange(key: string | null): void };
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
      {slice && slice.dimensions.length > 0 ? (
        <div style={rowStyle}>
          <span style={labelStyle}>Slice By</span>
          <select
            value={slice.value ?? ""}
            onChange={(e) => slice.onChange(e.target.value || null)}
            data-testid="metric-slice"
            style={selStyle}
          >
            <option value="">(none)</option>
            {slice.dimensions.map((d) => (
              <option key={d.key} value={d.key}>
                {d.label}
              </option>
            ))}
          </select>
        </div>
      ) : null}
    </div>
  );
}

const PAGE_SIZE = 25;

/** One recordings-table row: one capture of the metric. */
function recordingColumns(unit?: string | null): DataColumn<SeriesPoint>[] {
  return [
    { key: "time", label: "Time", render: (s) => new Date(String(s.captured_at)).toLocaleString() },
    { key: "value", label: "Value", numeric: true, render: (s) => <strong>{fmt(s.value, unit)}</strong> },
    { key: "branch", label: "Branch", render: (s) => <code>{s.branch ?? "—"}</code> },
    { key: "version", label: "Version", render: (s) => <code>{s.vcs_rev ? s.vcs_rev.slice(0, 8) : "—"}</code> },
    {
      key: "trust",
      label: "Trust",
      title: (s) => s.source ?? undefined,
      render: (s) => (s.provenance === "observed" ? "observed" : `⚠ ${s.provenance ?? "?"}`),
    },
  ];
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
      <DataTable
        testId="metric-recordings"
        columns={recordingColumns(unit)}
        rows={rows}
        rowKey={(s) => s.capture_id}
      />
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
  const code = (text: string) => <code style={{ fontSize: "var(--text-xs)", wordBreak: "break-all" }}>{text}</code>;
  const facts: Fact[] = [
    ...(rangeStat
      ? [
          {
            key: "range",
            label: rangeStat.label,
            // Exact value on hover — the headline compacts at 10k, so the
            // precise number has to stay reachable (tsk183).
            value: <strong title={formatMetricValueExact(rangeStat.value, def.unit)}>{rangeText}</strong>,
          },
        ]
      : []),
    // Definition metadata — the full "what is this metric" block (tsk33).
    { key: "id", label: "ID", value: code(def.key) },
    { key: "type", label: "Type", value: def.display_kind },
    { key: "aggregation", label: "Aggregation", value: specAggregation(def) },
    ...(def.source_measure ? [{ key: "measure", label: "Measure", value: code(def.source_measure) }] : []),
    { key: "scope", label: "Scope", value: def.scope },
    ...(def.category ? [{ key: "category", label: "Category", value: def.category }] : []),
    ...(def.language ? [{ key: "language", label: "Language", value: def.language }] : []),
    ...(def.unit ? [{ key: "unit", label: "Unit", value: def.unit }] : []),
    { key: "direction", label: "Direction", value: def.direction },
    ...(def.target != null ? [{ key: "target", label: "Target", value: fmt(def.target, def.unit) }] : []),
    ...(def.warn_at != null ? [{ key: "warn", label: "Warn at", value: fmt(def.warn_at, def.unit) }] : []),
    ...(def.fail_at != null ? [{ key: "fail", label: "Fail at", value: fmt(def.fail_at, def.unit) }] : []),
    ...(latest?.branch ? [{ key: "branch", label: "Branch", value: code(latest.branch) }] : []),
  ];
  return <Facts testId="metric-detail-stats" facts={facts} />;
}

/**
 * The files a scan left out that no later scan has covered. Their facts are
 * the last good ones, not a clean result, so this says so where the metric is
 * read. Nothing renders when no file is unchecked.
 */
export function UncheckedFiles({ files }: { files: UncheckedFile[] }) {
  if (files.length === 0) return null;
  return (
    <div
      data-testid="metric-unchecked-files"
      style={{ border: "1px solid var(--border, #2a2a2a)", borderRadius: 6, padding: "8px 10px", fontSize: 12 }}
    >
      <div style={{ fontWeight: 600 }}>
        {files.length} {files.length === 1 ? "file was" : "files were"} left out of the last scan
      </div>
      <div style={{ opacity: 0.7, marginBottom: 4 }}>
        Their numbers are the last ones recorded, not rechecked: raise snapshotMaxFileBytes or make them text.
      </div>
      <ul style={{ margin: 0, paddingLeft: 18, fontFamily: "monospace", fontSize: 11 }}>
        {files.slice(0, 20).map((f) => (
          <li key={f.path}>
            {f.path} <span style={{ opacity: 0.6 }}>({exclusionReason(f.reason)})</span>
          </li>
        ))}
      </ul>
      {files.length > 20 ? <div style={{ opacity: 0.6 }}>…and {files.length - 20} more</div> : null}
    </div>
  );
}
