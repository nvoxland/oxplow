import { Facts, type Fact } from "../kit/Facts.js";
import { useEffect, useRef } from "react";

import { useHistoryState } from "../tabs/useHistoryState.js";

import type { MetricSpec, SeriesPoint } from "../api.js";
import { formatMetricValue, formatMetricValueExact } from "../components/format.js";
import { DataTable, type DataColumn } from "../kit/index.js";
import {
  type UncheckedFile,
  exclusionReason,
  inRangeStat,
  specAggregation,
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
export function RecordingsTable({ samples, unit, filterKey }: { samples: SeriesPoint[]; unit?: string | null; filterKey: string }) {
  // The page is this step's history state: Back and a restart keep it.
  const [page, setPage] = useHistoryState<number>("recordingsPage", 0);
  // A different filter starts at the first page again — not the page's own
  // mount, which keeps the one it came back to.
  const filtered = useRef(filterKey);
  useEffect(() => {
    if (filtered.current === filterKey) return;
    filtered.current = filterKey;
    setPage(0);
  }, [filterKey, setPage]);

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
