import { useCallback, useEffect, useMemo, useState } from "react";

import { MetricFilters } from "../kit/MetricFilters.js";
import { DEFAULT_RANGE_KEY, rangeFromPreset, rangeValue, resolveRange, type TimeRange } from "../kit/rangePresets.js";
import { useViewParam } from "../tabs/useViewParam.js";
import {
  type Dashboard,
  type MetricCatalogEntry,
  type MetricSpec,
  type SeriesPoint,
  addDashboardItem,
  createDashboard,
  listDashboards,
  listMetricCatalog,
  listMetricDefinitions,
  listMetricSamples,
  removeDashboardItem,
  enableMetrics,
  subscribeDashboardEvents,
  querySql,
} from "../api.js";
import { NO_READS, unionReads, useRerunOnChange } from "../lens/lensRerun.js";
import { seriesByGroup } from "../metricsSql.js";
import { DIMENSIONS_SQL } from "./exploreData.js";
import { useRequestGuard } from "../request-guard.js";
import type { Reads } from "../tauri-bridge/generated/bindings.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { showToast } from "../components/toastStore.js";
import { useContextMenu } from "../components/useRowContextMenu.js";
import { buildAddToDashboardMenu, metricTile } from "./customDashboardData.js";
import { customDashboardRef } from "../tabs/pageRefs.js";
import { Page, pageH1Style } from "../tabs/Page.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import { MetricStatsRail, RecordingsTable, TrendChart, UncheckedFiles } from "./MetricDetail.js";
import {
  branchOptions,
  defaultChartMode,
  filterByBranch,
  filterByRange,
  UNCHECKED_FILES_SQL,
  type UncheckedFile,
  widestPresetWindow,
  seriesPoints,
  transformSeries,
} from "./metricDetailData.js";

const SAMPLE_LIMIT = 500;

function SectionLabel({ children }: { children: string }) {
  return (
    <div style={{ fontSize: 12, fontWeight: 600, opacity: 0.6, textTransform: "uppercase", letterSpacing: "0.04em" }}>
      {children}
    </div>
  );
}

/**
 * Metric Detail — one metric's page. The metric name is the page H1 (details
 * layout); the right rail carries the range / branch controls, the stats, Add
 * to dashboard and the enable toggle. The main column has the trend chart (drag
 * to select a range; charted the way the metric rolls up — cumulative for sum
 * metrics) and the paginated recordings, both within the range + branch.
 * Targets live in config (`.oxplow/project.yaml`); breakdowns and per-capture
 * findings are for agents (the MCP metric tools) and lenses.
 */
export function MetricDetailPage({
  metricKey,
  onOpenPage,
}: {
  metricKey?: string;
  onOpenPage?: (ref: TabRef) => void;
} = {}) {
  const [def, setDef] = useState<MetricSpec | null>(null);
  const [entry, setEntry] = useState<MetricCatalogEntry | null>(null);
  const [configBusy, setConfigBusy] = useState(false);
  const [samples, setSamples] = useState<SeriesPoint[]>([]);
  const [loading, setLoading] = useState(true);
  // Filters are the metric's view params: in its address, so Back and a restart bring them back. Default to the last 7
  // days, all branches.
  const [rangeKey, setRangeKey] = useViewParam<string>("range", DEFAULT_RANGE_KEY);
  const [branchName, setBranch] = useViewParam<string>("branch", "");
  const branch = branchName || null;
  // Slice By (P6.F1): a dimension groups the captures into one series each.
  const [sliceKey, setSlice] = useViewParam<string>("slice", "");
  const sliceBy = sliceKey || null;
  // The window the range stands for, fixed when the range changes (a preset
  // is "the last 7 days from then"); a range of all time shows the default.
  const range = useMemo<TimeRange>(
    () => resolveRange(rangeKey, Date.now()) ?? rangeFromPreset(DEFAULT_RANGE_KEY, Date.now()),
    [rangeKey],
  );
  const brush = (from: number, to: number) => setRangeKey(rangeValue({ from, to }, Date.now()));
  const [dimensions, setDimensions] = useState<{ key: string; label: string }[]>([]);
  useEffect(() => {
    void querySql(DIMENSIONS_SQL, [], null)
      .then((r) => setDimensions(r.rows.map(([key, label]) => ({ key: String(key), label: String(label ?? key) }))))
      .catch(() => setDimensions([]));
  }, []);
  // Files the latest scans left out: their facts are stale, not clean.
  const [unchecked, setUnchecked] = useState<UncheckedFile[]>([]);
  // "Add to dashboard" picker (tsk143).
  const [dashboards, setDashboards] = useState<Dashboard[]>([]);
  const addToDashboardMenu = useContextMenu();
  usePageTitle(def?.title ?? entry?.title ?? "Metric");

  // Keep the picker's options live so a dashboard created elsewhere — another
  // tab, or the agent via the MCP tools — shows up without a reload.
  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      void listDashboards().then((rows) => {
        if (!cancelled) setDashboards(rows);
      });
    };
    refresh();
    const off = subscribeDashboardEvents(refresh);
    return () => {
      cancelled = true;
      off();
    };
  }, []);

  // Chart the metric the way it rolls up (sum → cumulative, …).
  const mode = def ? defaultChartMode(def.aggregation) : "value";

  const [reads, setReads] = useState<Reads>(NO_READS);
  const guard = useRequestGuard();

  // The spec, the catalog entry (what a DISABLED metric still has: title,
  // enabled, resolved target — what the Configure block toggles, tsk117) and
  // the captures. It re-runs when something they read changed (P4.6/P4.7).
  const refresh = useCallback(() => {
    if (!metricKey) {
      setLoading(false);
      return;
    }
    const current = guard.begin();
    void Promise.all([
      listMetricDefinitions(),
      listMetricCatalog(),
      // Bound the read to the widest preset (tsk202); the chart's range
      // dropdown switches client-side within it. A metric with nothing to
      // grid (no spec, or a formula) shows no data.
      listMetricSamples(metricKey, SAMPLE_LIMIT, sliceBy, widestPresetWindow(Date.now())).catch(() => ({
        rows: [] as SeriesPoint[],
        reads: NO_READS,
      })),
    ]).then(([defs, catalog, captures]) => {
      if (!current()) return;
      setDef(defs.rows.find((d) => d.key === metricKey) ?? null);
      setEntry(catalog.rows.find((e) => e.key === metricKey) ?? null);
      setSamples(captures.rows);
      setReads(unionReads([defs.reads, catalog.reads, captures.reads]));
      setLoading(false);
    });
  }, [metricKey, sliceBy, guard]);

  useEffect(() => {
    refresh();
  }, [refresh]);
  useRerunOnChange(reads, refresh);

  const measureKey = def?.source_measure ?? null;
  useEffect(() => {
    if (!measureKey) {
      setUnchecked([]);
      return;
    }
    let cancelled = false;
    void querySql(UNCHECKED_FILES_SQL, [measureKey], null)
      .then((r) => {
        if (!cancelled) setUnchecked(r.rows.map(([path, reason]) => ({ path: String(path), reason: String(reason) })));
      })
      .catch(() => {
        if (!cancelled) setUnchecked([]);
      });
    return () => {
      cancelled = true;
    };
  }, [measureKey, samples]);

  const branches = useMemo(() => branchOptions(samples), [samples]);
  // The range + branch window feeds the chart, the recordings and the stats.
  const filtered = useMemo(
    () => filterByBranch(filterByRange(samples, range), branch),
    [samples, range, branch],
  );
  const points = useMemo(() => transformSeries(seriesPoints(filtered), mode), [filtered, mode]);
  // Sliced: one chart per dimension value (the lens line viz's small
  // multiples), each transformed the way the metric rolls up.
  const slices = useMemo(
    () =>
      sliceBy
        ? seriesByGroup(filtered).map((g) => ({ group: g.group, points: transformSeries(seriesPoints(g.points), mode) }))
        : null,
    [filtered, mode, sliceBy],
  );

  const toggleEnabled = async () => {
    if (!entry) return;
    setConfigBusy(true);
    try {
      await enableMetrics([entry.key], !entry.enabled);
    } catch (e) {
      recordOpError({
        label: `${entry.enabled ? "Disable" : "Enable"} ${entry.key}`,
        message: e instanceof Error ? e.message : String(e),
      });
    } finally {
      setConfigBusy(false);
    }
  };
  const addToDashboard = async (dashboardId: string) => {
    if (!metricKey) return;
    try {
      const tileId = await addDashboardItem({ dashboardId, ...metricTile(metricKey) });
      const title = dashboards.find((d) => d.id === dashboardId)?.title ?? "dashboard";
      // Stay on the metric — the toast carries the undo (remove the new tile).
      showToast({ message: `Added to ${title}`, onUndo: () => void removeDashboardItem(tileId) });
    } catch (e) {
      recordOpError({ label: "Add to dashboard", message: e instanceof Error ? e.message : String(e) });
    }
  };

  const addToNewDashboard = async () => {
    if (!metricKey) return;
    try {
      const created = await createDashboard("Untitled dashboard");
      await addDashboardItem({ dashboardId: created.id, ...metricTile(metricKey) });
      // A brand-new dashboard is worth showing; adding to an existing one
      // leaves you here with an undo toast instead.
      onOpenPage?.(customDashboardRef(created.id));
    } catch (e) {
      recordOpError({ label: "New dashboard", message: e instanceof Error ? e.message : String(e) });
    }
  };

  const dashboardBlock = metricKey ? (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <SectionLabel>Dashboard</SectionLabel>
      <button
        type="button"
        onClick={(e) =>
          addToDashboardMenu.open(
            e,
            buildAddToDashboardMenu(
              dashboards,
              (id) => void addToDashboard(id),
              () => void addToNewDashboard(),
            ),
          )
        }
        data-testid="metric-add-to-dashboard"
        style={{
          fontSize: 13,
          padding: "6px 10px",
          borderRadius: 6,
          border: "1px solid var(--border-subtle)",
          background: "var(--surface-card)",
          color: "var(--text, #ddd)",
          cursor: "pointer",
          textAlign: "left",
        }}
      >
        Add to dashboard ▾
      </button>
      {addToDashboardMenu.menu}
    </div>
  ) : null;

  // The configure block: enabling writes a `use:` into .oxplow/project.yaml
  // and the runner reseeds. Targets are set in that file.
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
        {slices ? (
          <div data-testid="metric-slices" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
            {slices.map((s) => (
              <div key={s.group ?? "(none)"}>
                <SectionLabel>{s.group ?? "(none)"}</SectionLabel>
                <TrendChart
                  points={s.points}
                  target={mode === "value" ? def.target : null}
                  domain={range}
                  unit={def.unit}
                  height={140}
                  onSelectRange={brush}
                />
              </div>
            ))}
          </div>
        ) : (
          <TrendChart
            points={points}
            target={mode === "value" ? def.target : null}
            domain={range}
            unit={def.unit}
            onSelectRange={brush}
          />
        )}
        <UncheckedFiles files={unchecked} />
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <SectionLabel>Recordings</SectionLabel>
          <RecordingsTable samples={filtered} unit={def.unit} filterKey={`${rangeKey}|${branchName}|${sliceKey}`} />
        </div>
      </div>
    );
  })();

  return (
    <Page
      testId="page-metric-detail"
      title={def?.title ?? entry?.title ?? "Metric"}
      titleInBody
      width="reading"
      rightRailTitle="Details"
      rightRail={
        def ? (
          <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            <MetricFilters
              range={rangeKey}
              branch={branchName}
              branches={branches}
              slice={sliceKey}
              dimensions={dimensions}
              onApply={(name, value) => ({ range: setRangeKey, branch: setBranch, slice: setSlice })[name](value)}
            />
            <MetricStatsRail def={def} samples={filtered} />
            {dashboardBlock}
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
