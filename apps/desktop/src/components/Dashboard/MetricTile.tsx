import { useCallback, useEffect, useMemo, useState } from "react";

import { type MetricSpec, type SeriesPoint, querySql } from "../../api.js";
import { seriesPoints as seriesPointRows } from "../../metricsSql.js";
import { NO_READS, useRerunOnChange } from "../../lens/lensRerun.js";
import { useRequestGuard } from "../../request-guard.js";
import { formatMetricValue } from "../format.js";
import { TrendChart } from "../../pages/MetricDetail.js";
import type { TimeRange } from "../../kit/rangePresets.js";
import {
  branchOptions,
  defaultChartMode,
  deltaVsFirst,
  filterByBranch,
  filterByRange,
  seriesPoints,
  transformSeries,
} from "../../pages/metricDetailData.js";
import { deltaTone, latestValue } from "../../pages/customDashboardData.js";

const SAMPLE_LIMIT = 500;

/** Tile headline numbers go through the SHARED formatter (tsk183). No `unit`
 *  is passed: the tile renders `def.unit` in its own span beside the number. */
function fmt(n: number): string {
  if (!Number.isFinite(n)) return "—";
  return formatMetricValue(n);
}

const TONE_COLOR: Record<"good" | "bad" | "neutral", string> = {
  good: "var(--success, #3fb950)",
  bad: "var(--danger, #f85149)",
  neutral: "var(--text-muted, #888)",
};

/**
 * The metric card's body: a metric's captures (its pinned SQL, `sql`) as a
 * `line` (the shared {@link TrendChart}, charted the way the metric rolls
 * up) or a `number` (the latest value + a signed delta colored by the
 * spec's `direction`), windowed by `window`. Live-refreshes on what it
 * read. A dashboard lens's row tiles draw it.
 */
export function MetricTileBody({
  sql,
  metricKey,
  viz: shown,
  size,
  def,
  window,
  onBranches,
}: {
  sql: string | null;
  metricKey: string | null;
  viz: string | null;
  size: string | null;
  def: MetricSpec | null;
  window: { range: TimeRange | null; branch: string | null };
  onBranches?: (branches: string[]) => void;
}) {
  const [samples, setSamples] = useState<SeriesPoint[]>([]);
  const [reads, setReads] = useState(NO_READS);
  const [loading, setLoading] = useState(true);
  const viz = shown ?? "line";
  const guard = useRequestGuard();

  const refresh = useCallback(() => {
    if (!sql) {
      setLoading(false);
      return;
    }
    const current = guard.begin();
    // A metric with nothing to grid (no spec, or a formula) shows no data.
    void querySql(sql, [], SAMPLE_LIMIT)
      .then((result) => ({ rows: seriesPointRows(result), reads: result.reads }))
      .catch(() => ({ rows: [] as SeriesPoint[], reads: NO_READS }))
      .then(({ rows, reads: r }) => {
        if (!current()) return;
        setSamples(rows);
        setReads(r);
        setLoading(false);
        // Feed the dashboard's branch filter its options (union across tiles).
        onBranches?.(branchOptions(rows));
      });
    // `onBranches` is a report-upward callback, excluded from deps on purpose.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sql, guard]);

  useEffect(() => {
    refresh();
  }, [refresh]);
  useRerunOnChange(reads, refresh);

  const windowed = useMemo(() => {
    const byRange = window.range ? filterByRange(samples, window.range) : samples;
    return filterByBranch(byRange, window.branch);
  }, [window, samples]);

  if (!metricKey || !sql) return <div style={{ opacity: 0.6, fontSize: 13 }}>No metric selected.</div>;
  if (loading) return <div style={{ opacity: 0.6, fontSize: 13 }}>Loading…</div>;
  if (!def)
    return (
      <div style={{ opacity: 0.6, fontSize: 13 }} data-testid="metric-tile-missing">
        Metric not found or disabled.
      </div>
    );

  if (viz === "number") {
    const value = latestValue(windowed);
    const delta = deltaVsFirst(windowed);
    const tone = delta != null ? deltaTone(delta, def.direction) : "neutral";
    return (
      <div
        style={{ flex: 1, display: "flex", flexDirection: "column", justifyContent: "center", gap: 4 }}
        data-testid="metric-tile-number"
      >
        <div style={{ fontSize: 34, fontWeight: 700, lineHeight: 1.1 }}>
          {value != null ? fmt(value) : "—"}
          {def.unit ? <span style={{ fontSize: 15, opacity: 0.6, marginLeft: 4 }}>{def.unit}</span> : null}
        </div>
        {delta != null ? (
          <div style={{ fontSize: 13, color: TONE_COLOR[tone] }}>
            {delta > 0 ? "▲" : delta < 0 ? "▼" : "•"} {fmt(Math.abs(delta))} in range
          </div>
        ) : (
          <div style={{ fontSize: 13, opacity: 0.5 }}>
            {windowed.length} sample{windowed.length === 1 ? "" : "s"}
          </div>
        )}
      </div>
    );
  }

  const mode = defaultChartMode(def.aggregation);
  const points = transformSeries(seriesPoints(windowed), mode);
  return (
    <div data-testid="metric-tile-line" style={{ flex: 1, display: "flex", alignItems: "center" }}>
      <TrendChart
        points={points}
        target={mode === "value" ? def.target : null}
        unit={def.unit}
        // Sized near the tile's own width so the drawing renders ~1:1 and the
        // 9px tick labels stay readable instead of scaling down (tsk144).
        width={size === "wide" ? 820 : 400}
        height={size === "tall" ? 380 : 200}
      />
    </div>
  );
}
