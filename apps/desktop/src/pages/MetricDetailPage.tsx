import { useCallback, useEffect, useMemo, useRef, useState } from "react";

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
import { Page, pageH1Style } from "../tabs/Page.js";
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
  type ChartScale,
  DEFAULT_CHART_SCALE,
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
// A grouped fetch returns one point per (capture × group); the read caps TOTAL
// points, so a breakdown-filtered chart needs headroom for every group's whole
// series. Breakdown dims are low-cardinality (package / language), so this is
// generous rather than unbounded (tsk136).
const GROUP_SAMPLE_LIMIT = 20000;
const DRILL_IN_KINDS = new Set(["findings", "test", "coverage", "event"]);

function SectionLabel({ children }: { children: string }) {
  return (
    <div style={{ fontSize: 12, fontWeight: 600, opacity: 0.6, textTransform: "uppercase", letterSpacing: "0.04em" }}>
      {children}
    </div>
  );
}

function TabButton({
  label,
  active,
  onClick,
  testId,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
  testId: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      data-testid={testId}
      aria-selected={active}
      style={{
        fontSize: 12,
        fontWeight: 600,
        padding: "4px 10px",
        border: "none",
        borderBottom: active ? "2px solid var(--accent, #58a6ff)" : "2px solid transparent",
        background: "none",
        color: active ? "var(--text, #ddd)" : "var(--text-muted, #888)",
        cursor: "pointer",
      }}
    >
      {label}
    </button>
  );
}

/** The main-column data section: Breakdown and Recordings behind a tab
 *  selector, **Breakdown default** (most viewers want the roll-up, not the raw
 *  per-capture rows). When the metric has no per-file breakdown — the roll-up
 *  comes back empty (coverage, operational metrics) — the Breakdown tab is
 *  dropped and Recordings shows on its own (tsk134). Breakdown stays mounted via
 *  a display toggle so its availability callback fires even under Recordings. */
function BreakdownRecordings({
  def,
  samples,
  onOpenPage,
  onSelectGroup,
  onDimChange,
  activeGroup,
}: {
  def: MetricSpec;
  samples: SeriesPoint[];
  onOpenPage?: (ref: TabRef) => void;
  onSelectGroup?: (dim: string, value: string) => void;
  onDimChange?: () => void;
  activeGroup?: string | null;
}) {
  const [tab, setTab] = useState<"breakdown" | "recordings">("breakdown");
  // null = still loading; false = the roll-up returned nothing to break down.
  const [hasBreakdown, setHasBreakdown] = useState<boolean | null>(null);
  // Sticky-true: once a dimension has data, switching to an empty dimension
  // must not yank the whole tab away. Stable identity so the card's fetch
  // effect doesn't re-run every render.
  const reportBreakdown = useCallback(
    (has: boolean) => setHasBreakdown((prev) => (prev === true ? true : has)),
    [],
  );
  const active = hasBreakdown === false ? "recordings" : tab;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }} data-testid="metric-detail-panels">
      {hasBreakdown === false ? (
        <SectionLabel>Recordings</SectionLabel>
      ) : (
        <div style={{ display: "flex", gap: 4, borderBottom: "1px solid var(--border, #2a2a2a)" }}>
          <TabButton
            label="Breakdown"
            active={active === "breakdown"}
            onClick={() => setTab("breakdown")}
            testId="metric-tab-breakdown"
          />
          <TabButton
            label="Recordings"
            active={active === "recordings"}
            onClick={() => setTab("recordings")}
            testId="metric-tab-recordings"
          />
        </div>
      )}
      <div style={{ display: active === "breakdown" ? "block" : "none" }}>
        <MetricBreakdownCard
          def={def}
          onAvailability={reportBreakdown}
          onSelectGroup={onSelectGroup}
          onDimChange={onDimChange}
          activeGroup={activeGroup}
        />
      </div>
      <div style={{ display: active === "recordings" ? "block" : "none" }}>
        <RecordingsTable samples={samples} unit={def.unit} metricKey={def.key} onOpenPage={onOpenPage} />
      </div>
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
  const [scale, setScale] = useState<ChartScale>(DEFAULT_CHART_SCALE);
  // Seed the chart mode from the metric's roll-up (sum→cumulative, …) when the
  // def loads, but stop overriding once the user picks a mode (tsk302).
  const modeTouched = useRef(false);
  const [branch, setBranch] = useState<string | null>(null);
  // A clicked breakdown group ({dim, value}) filters the chart to that group's
  // series (tsk136); `groupSamples` holds that group's fetched points.
  const [breakdownFilter, setBreakdownFilter] = useState<{ dim: string; value: string } | null>(null);
  const [groupSamples, setGroupSamples] = useState<SeriesPoint[]>([]);
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

  // Drop any breakdown filter when navigating to a different metric.
  useEffect(() => setBreakdownFilter(null), [metricKey]);

  // When a breakdown group is selected, fetch that dimension's grouped series
  // and keep just the clicked group's points (tsk136). Cleared when the filter
  // is dropped or the metric changes.
  useEffect(() => {
    if (!metricKey || !breakdownFilter) {
      setGroupSamples([]);
      return;
    }
    let cancelled = false;
    const { dim, value } = breakdownFilter;
    void listMetricSamples(metricKey, GROUP_SAMPLE_LIMIT, dim).then((all) => {
      if (!cancelled) setGroupSamples(all.filter((p) => p.group === value));
    });
    return () => {
      cancelled = true;
    };
  }, [metricKey, breakdownFilter]);

  const branches = useMemo(() => branchOptions(samples), [samples]);
  // The whole-metric window feeds the Recordings table + stats rail.
  const filtered = useMemo(
    () => filterByBranch(filterByRange(samples, range), branch),
    [samples, range, branch],
  );
  // The CHART uses the breakdown-filtered group series when one is selected.
  const points = useMemo(() => {
    const src = breakdownFilter ? groupSamples : samples;
    const windowed = filterByBranch(filterByRange(src, range), branch);
    return transformSeries(seriesPoints(windowed), mode);
  }, [breakdownFilter, groupSamples, samples, range, branch, mode]);

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
        <h1 style={pageH1Style} data-testid="metric-detail-title">
          {def.title}
        </h1>
        {def.description ? (
          <p style={{ margin: 0, fontSize: 14, lineHeight: 1.5, opacity: 0.8 }} data-testid="metric-description">
            {def.description}
          </p>
        ) : null}
        {breakdownFilter ? (
          <div
            style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12 }}
            data-testid="breakdown-filter-chip"
          >
            <span style={{ opacity: 0.6 }}>Charting</span>
            <span style={{ fontWeight: 600 }}>{breakdownFilter.value}</span>
            <span style={{ opacity: 0.5 }}>· {breakdownFilter.dim}</span>
            <button
              type="button"
              onClick={() => setBreakdownFilter(null)}
              data-testid="breakdown-filter-clear"
              title="Back to the whole metric"
              aria-label="Back to the whole metric"
              style={{ fontSize: 11, cursor: "pointer", padding: "1px 6px" }}
            >
              ✕
            </button>
          </div>
        ) : null}
        <TrendChart
          points={points}
          target={mode === "value" ? def.target : null}
          domain={range}
          unit={def.unit}
          scale={scale}
          onSelectRange={(from, to) => setRange({ from, to })}
        />
        <BreakdownRecordings
          def={def}
          samples={filtered}
          onOpenPage={onOpenPage}
          onSelectGroup={(dim, value) =>
            setBreakdownFilter((cur) => (cur?.dim === dim && cur.value === value ? null : { dim, value }))
          }
          onDimChange={() => setBreakdownFilter(null)}
          activeGroup={breakdownFilter?.value ?? null}
        />
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
      titleInBody
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
              scale={scale}
              onScale={setScale}
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
