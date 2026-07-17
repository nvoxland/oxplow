import { useEffect, useMemo, useRef, useState } from "react";

import {
  type EffortMetricDelta,
  type FactFinding,
  type MetricCatalogEntry,
  type MetricSpec,
  type SeriesPoint,
  listEffortMetricDeltas,
  listMetricCatalog,
  listMetricDefinitions,
  listMetricFindings,
  listMetricSamples,
  setMetricEnabled,
  setMetricOverride,
  subscribeOxplowEvents,
} from "../api.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { Page } from "../tabs/Page.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import {
  KindDrillIn,
  MetricBreakdownCard,
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
  const [def, setDef] = useState<MetricSpec | null>(null);
  const [entry, setEntry] = useState<MetricCatalogEntry | null>(null);
  const [configBusy, setConfigBusy] = useState(false);
  const [samples, setSamples] = useState<SeriesPoint[]>([]);
  const [findings, setFindings] = useState<FactFinding[]>([]);
  const [effortDelta, setEffortDelta] = useState<EffortMetricDelta | null>(null);
  const [loading, setLoading] = useState(true);
  // Filters. Default to the last 7 days, all branches, raw value.
  const [range, setRange] = useState<TimeRange>(() => rangeFromPreset(DEFAULT_RANGE_KEY, Date.now()));
  const [mode, setMode] = useState<ChartMode>("value");
  // Seed the chart mode from the metric's roll-up (sum→cumulative, …) when the
  // def loads, but stop overriding once the user picks a mode (tsk302).
  const modeTouched = useRef(false);
  const [branch, setBranch] = useState<string | null>(null);
  usePageTitle(def?.title ?? entry?.title ?? "Metric");

  useEffect(() => {
    if (def && !modeTouched.current) setMode(defaultChartMode(def.aggregation));
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
      // The catalog entry is what a DISABLED metric still has (its spec is
      // pruned): it carries title + enabled + resolved target, and it is what
      // the Configure block toggles (tsk117 — Metric Settings folded in here).
      void listMetricCatalog().then((entries) => {
        if (!cancelled) setEntry(entries.find((e) => e.key === metricKey) ?? null);
      });
      void listMetricSamples(metricKey, SAMPLE_LIMIT).then(async (rows) => {
        if (cancelled) return;
        setSamples(rows);
        const captureId = rows[0]?.capture_id ?? null;
        const fs = captureId != null ? await listMetricFindings(metricKey, captureId) : [];
        if (!cancelled) setFindings(fs);
      });
    };
    refresh();
    const off = subscribeOxplowEvents((e) => {
      // configChanged: an enable/target write (ours or an external
      // .oxplow/project.yaml edit) re-resolves the catalog + spec.
      if (e.kind === "metricSamplesChanged" || e.kind === "configChanged") refresh();
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

  const toggleEnabled = async () => {
    if (!entry) return;
    setConfigBusy(true);
    try {
      await setMetricEnabled(entry.key, !entry.enabled);
    } catch (e) {
      recordOpError({
        label: `${entry.enabled ? "Disable" : "Enable"} ${entry.key}`,
        message: e instanceof Error ? e.message : String(e),
      });
    } finally {
      setConfigBusy(false);
    }
  };
  const overrideTarget = async (next: number | null) => {
    if (!entry) return;
    setConfigBusy(true);
    try {
      await setMetricOverride(entry.key, next);
    } catch (e) {
      recordOpError({
        label: `Update ${entry.key}`,
        message: e instanceof Error ? e.message : String(e),
      });
    } finally {
      setConfigBusy(false);
    }
  };

  // The configure block — Metric Settings folded into the detail page
  // (tsk117): the detail IS the one place a metric is configured now. Enable
  // writes a `use:` into .oxplow/project.yaml and the runner reseeds; target
  // is a config override (empty clears), editable only while enabled — same
  // rules the Settings page enforced.
  const configure = entry ? (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <SectionLabel>Configure</SectionLabel>
      <label style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 13 }}>
        <input
          type="checkbox"
          checked={entry.enabled}
          disabled={configBusy}
          onChange={() => void toggleEnabled()}
          data-testid="metric-detail-enabled"
        />
        Enabled
      </label>
      {entry.enabled ? (
        <label style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 13 }}>
          <span style={{ opacity: 0.6 }}>Target</span>
          <input
            // Uncontrolled (so typing isn't clobbered mid-edit), but keyed on
            // the resolved target so an external config edit arriving via
            // `configChanged` remounts it with the new value.
            key={`target-${entry.key}-${entry.target ?? "none"}`}
            type="number"
            defaultValue={entry.target ?? ""}
            placeholder="none"
            disabled={configBusy}
            onBlur={(e) => {
              const raw = e.target.value.trim();
              const next = raw === "" ? null : Number(raw);
              if (next !== entry.target && !(next != null && Number.isNaN(next))) {
                void overrideTarget(next);
              }
            }}
            data-testid="metric-detail-target"
            style={{ width: 80, fontSize: 12, textAlign: "right" }}
          />
        </label>
      ) : null}
    </div>
  ) : null;

  const body = (() => {
    if (loading) return <div style={{ opacity: 0.6 }}>Loading…</div>;
    if (!def && entry)
      return (
        <div style={{ opacity: 0.6, lineHeight: 1.6 }} data-testid="metric-detail-disabled">
          This metric is disabled — nothing records for it. Enable it in the
          Configure panel to start collecting.
        </div>
      );
    if (!def) return <div style={{ opacity: 0.6 }}>Metric not found.</div>;
    const hasDrillIn = DRILL_IN_KINDS.has(def.display_kind);
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
        <MetricBreakdownCard def={def} />
        {hasDrillIn ? (
          <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
            <SectionLabel>Latest recording</SectionLabel>
            <KindDrillIn def={def} findings={findings} metricKey={def.key} />
          </div>
        ) : null}
      </div>
    );
  })();

  return (
    <Page
      testId="page-metric-detail"
      title={def?.title ?? entry?.title ?? "Metric"}
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
            {configure}
          </div>
        ) : (
          // A disabled metric has no spec (pruned) but still configures —
          // the rail is exactly how it gets turned back on (tsk117).
          (configure ?? undefined)
        )
      }
    >
      {body}
    </Page>
  );
}
