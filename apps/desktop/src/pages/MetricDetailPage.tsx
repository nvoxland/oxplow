import { useEffect, useMemo, useRef, useState } from "react";

import {
  type EffortMetricDelta,
  type MetricDefinition,
  type MetricFinding,
  type MetricSample,
  listEffortMetricDeltas,
  listMetricDefinitions,
  listMetricFindings,
  listMetricSamples,
  subscribeOxplowEvents,
} from "../api.js";
import { Page } from "../tabs/Page.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import {
  KindDrillIn,
  MetricControls,
  MetricStatsRail,
  RecordingsTable,
  TrendChart,
} from "./MetricDetail.js";
import {
  type ChartMode,
  DEFAULT_RANGE_KEY,
  type TimeRange,
  branchOptions,
  defaultChartMode,
  filterByBranch,
  filterByRange,
  rangeFromPreset,
  seriesPoints,
  transformSeries,
} from "./metricDetailData.js";

const SAMPLE_LIMIT = 500;
const DRILL_IN_KINDS = new Set(["findings", "test", "coverage", "event"]);

function SectionLabel({ children }: { children: string }) {
  return (
    <div style={{ fontSize: 12, fontWeight: 600, opacity: 0.6, textTransform: "uppercase", letterSpacing: "0.04em" }}>
      {children}
    </div>
  );
}

/**
 * Metric Detail — one metric's page (tsk291/292/293/294). The metric name is
 * the page H1 (details layout); the right rail carries the stats (latest, Δ,
 * type, id, …) and the "In this effort" callout. The main column has the
 * time-range / chart-mode / branch controls, the trend chart (drag to select a
 * range), the paginated recordings table, then the kind-specific drill-in. All
 * three respect the active range + branch filter.
 */
export function MetricDetailPage({
  metricKey,
  effort,
  onOpenPage,
}: {
  metricKey?: string;
  effort?: { effortId: string; start: string; end: string | null };
  onOpenPage?: (ref: TabRef) => void;
} = {}) {
  const [def, setDef] = useState<MetricDefinition | null>(null);
  const [samples, setSamples] = useState<MetricSample[]>([]);
  const [findings, setFindings] = useState<MetricFinding[]>([]);
  const [effortDelta, setEffortDelta] = useState<EffortMetricDelta | null>(null);
  const [loading, setLoading] = useState(true);
  // Filters. Default to the last 7 days, all branches, raw value.
  const [range, setRange] = useState<TimeRange>(() => rangeFromPreset(DEFAULT_RANGE_KEY, Date.now()));
  const [mode, setMode] = useState<ChartMode>("value");
  // Seed the chart mode from the metric's roll-up (sum→cumulative, …) when the
  // def loads, but stop overriding once the user picks a mode (tsk302).
  const modeTouched = useRef(false);
  const [branch, setBranch] = useState<string | null>(null);
  usePageTitle(def?.title ?? "Metric");

  useEffect(() => {
    if (def && !modeTouched.current) setMode(defaultChartMode(def.default_agg));
  }, [def]);
  const handleMode = (m: ChartMode) => {
    modeTouched.current = true;
    setMode(m);
  };

  useEffect(() => {
    if (!metricKey) {
      setLoading(false);
      return;
    }
    let cancelled = false;
    const refresh = () => {
      void listMetricDefinitions().then((defs) => {
        if (!cancelled) {
          setDef(defs.find((d) => d.key === metricKey) ?? null);
          setLoading(false);
        }
      });
      void listMetricSamples(metricKey, SAMPLE_LIMIT).then(async (rows) => {
        if (cancelled) return;
        setSamples(rows);
        const runId = rows[0]?.run_id ?? null;
        const fs = runId != null ? await listMetricFindings(runId) : [];
        if (!cancelled) setFindings(fs);
      });
    };
    refresh();
    const off = subscribeOxplowEvents((e) => {
      if (e.kind === "metricSamplesChanged") refresh();
    });
    return () => {
      cancelled = true;
      off();
    };
  }, [metricKey]);

  useEffect(() => {
    if (!effort || !metricKey) {
      setEffortDelta(null);
      return;
    }
    let cancelled = false;
    void listEffortMetricDeltas(effort.effortId).then((rows) => {
      if (!cancelled) setEffortDelta(rows.find((r) => r.key === metricKey) ?? null);
    });
    return () => {
      cancelled = true;
    };
  }, [effort?.effortId, metricKey, effort]);

  const branches = useMemo(() => branchOptions(samples), [samples]);
  const filtered = useMemo(
    () => filterByBranch(filterByRange(samples, range), branch),
    [samples, range, branch],
  );
  const points = useMemo(() => transformSeries(seriesPoints(filtered), mode), [filtered, mode]);

  const body = (() => {
    if (loading) return <div style={{ opacity: 0.6 }}>Loading…</div>;
    if (!def) return <div style={{ opacity: 0.6 }}>Metric not found.</div>;
    const hasDrillIn = DRILL_IN_KINDS.has(def.kind);
    return (
      <div style={{ display: "flex", flexDirection: "column", gap: 20 }} data-testid="metric-detail">
        {def.description ? (
          <p style={{ margin: 0, fontSize: 14, lineHeight: 1.5, opacity: 0.8 }} data-testid="metric-description">
            {def.description}
          </p>
        ) : null}
        <TrendChart
          points={points}
          target={mode === "value" ? def.target : null}
          domain={range}
          unit={def.unit}
          onSelectRange={(from, to) => setRange({ from, to })}
        />
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <SectionLabel>Recordings</SectionLabel>
          <RecordingsTable samples={filtered} unit={def.unit} metricKey={def.key} onOpenPage={onOpenPage} />
        </div>
        {hasDrillIn ? (
          <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
            <SectionLabel>Latest run</SectionLabel>
            <KindDrillIn def={def} findings={findings} samples={filtered} />
          </div>
        ) : null}
      </div>
    );
  })();

  return (
    <Page
      testId="page-metric-detail"
      title={def?.title ?? "Metric"}
      layout="details"
      rightRailTitle="Details"
      rightRail={
        def ? (
          <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            <MetricControls
              range={range}
              onRange={setRange}
              mode={mode}
              onMode={handleMode}
              branch={branch}
              branches={branches}
              onBranch={setBranch}
            />
            <MetricStatsRail def={def} samples={filtered} effort={effort} effortDelta={effortDelta} />
          </div>
        ) : undefined
      }
    >
      {body}
    </Page>
  );
}
